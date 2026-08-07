//! Persisted UI preferences: the theme and the tab bar layout.
//!
//! One small JSON file, written when the theme picker commits or the tab manager
//! closes. Every failure here is non-fatal — a missing, unreadable, or corrupt
//! file just means defaults, and a failed write is dropped silently rather than
//! interrupting the TUI.

use crate::backend::Tab;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Config {
    /// Index into `ui::THEMES`.
    pub theme: usize,
    /// Display order of the tab bar: a permutation of `0..Tab::ALL.len()`.
    pub tab_order: Vec<usize>,
    /// Hidden tabs, keyed by `Tab::ALL` index.
    pub tab_hidden: Vec<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: 0,
            tab_order: (0..Tab::ALL.len()).collect(),
            tab_hidden: vec![false; Tab::ALL.len()],
        }
    }
}

impl Config {
    /// Repair anything that would break the UI: an unknown theme, a tab order
    /// with duplicates or unknown tabs, or an every-tab-hidden bar (which would
    /// leave nothing to draw).
    ///
    /// A config written by an older gitsmith simply lists fewer tabs. That's not
    /// corruption — the new tabs are appended in their default position and the
    /// user's ordering survives the upgrade.
    pub fn sanitized(mut self) -> Config {
        let n = Tab::ALL.len();
        if self.theme >= crate::ui::THEMES.len() {
            self.theme = 0;
        }
        let mut seen = vec![false; n];
        let mut order: Vec<usize> = Vec::with_capacity(n);
        for &t in &self.tab_order {
            if t < n && !std::mem::replace(&mut seen[t], true) {
                order.push(t);
            }
        }
        // Whatever the saved order didn't mention (new tabs, or entries dropped
        // as duplicates) goes to the end in `Tab::ALL` order.
        order.extend((0..n).filter(|t| !seen[*t]));
        self.tab_order = order;

        self.tab_hidden.truncate(n);
        self.tab_hidden.resize(n, false); // tabs the old config never knew about start visible
        if self.tab_hidden.iter().all(|&h| h) {
            self.tab_hidden = vec![false; n];
        }
        self
    }
}

/// Where the config lives, per platform:
///
/// - `$GITSMITH_CONFIG` always wins (a full file path — handy for tests and for
///   keeping a per-project layout)
/// - Windows: `%APPDATA%\gitsmith\config.json`, falling back to
///   `%USERPROFILE%\AppData\Roaming\…`
/// - macOS / Linux: `$XDG_CONFIG_HOME/gitsmith/config.json`, else
///   `$HOME/.config/gitsmith/config.json` — where terminal tools live on both,
///   matching `gh` and `glab`
///
/// `None` means no home could be determined; the caller then runs without
/// persistence rather than guessing at a location.
pub fn path() -> Option<PathBuf> {
    resolve(|k| std::env::var_os(k), cfg!(windows))
}

/// The platform rules above, over an injected environment so every branch is
/// testable from any host.
fn resolve(env: impl Fn(&str) -> Option<std::ffi::OsString>, windows: bool) -> Option<PathBuf> {
    // Empty vars are treated as unset — a stray `XDG_CONFIG_HOME=` shouldn't
    // send the config to a relative path.
    let var = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);

    if let Some(p) = var("GITSMITH_CONFIG") {
        return Some(p);
    }
    let dir = if windows {
        var("APPDATA")
            .or_else(|| var("USERPROFILE").map(|h| h.join("AppData").join("Roaming")))
            .or_else(|| var("HOME").map(|h| h.join("AppData").join("Roaming")))?
    } else {
        // XDG says a relative XDG_CONFIG_HOME is invalid and must be ignored.
        var("XDG_CONFIG_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| var("HOME").map(|h| h.join(".config")))?
    };
    Some(dir.join("gitsmith").join("config.json"))
}

/// Read and validate the config; anything unusable falls back to defaults.
pub fn load(path: &Path) -> Config {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Config>(&s).ok())
        .unwrap_or_default()
        .sanitized()
}

/// Write the config, creating the directory if needed. Errors are the caller's
/// to ignore — a read-only home shouldn't stop the UI.
///
/// Writes to a sibling temp file and renames it into place, so an interrupted
/// write can't leave a half-written config behind. `rename` replaces the
/// destination on Windows as well as Unix, as long as both sides are on the
/// same directory — which they are here.
pub fn save(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(cfg).unwrap_or_default();
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, json + "\n")?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("gitsmith-cfg-{}-{name}.json", std::process::id()));
        p
    }

    /// A fake environment, so the Windows rules can be tested on Unix and vice versa.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> + use<> {
        let owned: Vec<(String, String)> =
            pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
        move |k: &str| {
            owned.iter().find(|(key, _)| key == k).map(|(_, v)| std::ffi::OsString::from(v))
        }
    }

    #[test]
    fn linux_and_mac_use_xdg_then_dot_config() {
        let p = resolve(env(&[("HOME", "/home/dev")]), false).unwrap();
        assert_eq!(p, PathBuf::from("/home/dev/.config/gitsmith/config.json"));

        let p = resolve(env(&[("HOME", "/Users/dev"), ("XDG_CONFIG_HOME", "/Users/dev/cfg")]), false)
            .unwrap();
        assert_eq!(p, PathBuf::from("/Users/dev/cfg/gitsmith/config.json"));
    }

    #[test]
    fn windows_uses_appdata() {
        let p = resolve(env(&[("APPDATA", r"C:\Users\dev\AppData\Roaming")]), true).unwrap();
        assert_eq!(p, PathBuf::from(r"C:\Users\dev\AppData\Roaming").join("gitsmith").join("config.json"));

        // no APPDATA (a bare shell) — derive it from the profile
        let p = resolve(env(&[("USERPROFILE", r"C:\Users\dev")]), true).unwrap();
        assert_eq!(
            p,
            PathBuf::from(r"C:\Users\dev").join("AppData").join("Roaming").join("gitsmith").join("config.json")
        );

        // XDG/HOME-style vars must not send Windows to a Unix-looking path
        let p = resolve(env(&[("XDG_CONFIG_HOME", "/home/dev/.config"), ("APPDATA", r"C:\AppData")]), true)
            .unwrap();
        assert!(p.starts_with(r"C:\AppData"));
    }

    #[test]
    fn explicit_override_wins_everywhere() {
        for windows in [true, false] {
            let p = resolve(
                env(&[("GITSMITH_CONFIG", "/tmp/mine.json"), ("HOME", "/home/dev"), ("APPDATA", r"C:\A")]),
                windows,
            );
            assert_eq!(p, Some(PathBuf::from("/tmp/mine.json")));
        }
    }

    #[test]
    fn empty_or_relative_vars_are_ignored() {
        // empty XDG falls through to HOME
        let p = resolve(env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/home/dev")]), false).unwrap();
        assert_eq!(p, PathBuf::from("/home/dev/.config/gitsmith/config.json"));
        // relative XDG is invalid per the spec — fall through too
        let p = resolve(env(&[("XDG_CONFIG_HOME", "relative/cfg"), ("HOME", "/home/dev")]), false).unwrap();
        assert_eq!(p, PathBuf::from("/home/dev/.config/gitsmith/config.json"));
        // nothing at all: no persistence rather than a guess
        assert!(resolve(env(&[]), false).is_none());
        assert!(resolve(env(&[]), true).is_none());
    }

    #[test]
    fn save_replaces_an_existing_file_and_leaves_no_temp() {
        let path = tmp("replace");
        save(&path, &Config::default()).unwrap();
        let mut cfg = Config::default();
        cfg.tab_hidden[0] = true;
        save(&path, &cfg).unwrap(); // rename over an existing file
        assert_eq!(load(&path), cfg);
        // exactly the temp path this save would have used — other tests write
        // their own alongside it, and those are none of this test's business
        let leftover = path.with_extension(format!("tmp{}", std::process::id()));
        assert!(!leftover.exists(), "temp file was not cleaned up");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trips() {
        let path = tmp("round");
        let mut cfg = Config { theme: 3, ..Config::default() };
        cfg.tab_order.swap(0, 4);
        cfg.tab_hidden[2] = true;
        save(&path, &cfg).unwrap();
        assert_eq!(load(&path), cfg);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_or_corrupt_file_is_default() {
        assert_eq!(load(Path::new("/nonexistent/gitsmith.json")), Config::default());
        let path = tmp("junk");
        std::fs::write(&path, "not json at all").unwrap();
        assert_eq!(load(&path), Config::default());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn invalid_values_are_repaired() {
        let n = Tab::ALL.len();
        // out-of-range theme, duplicated tab, wrong-length hidden list
        let bad = Config { theme: 999, tab_order: vec![0; n], tab_hidden: vec![true, false] };
        let fixed = bad.sanitized();
        assert_eq!(fixed.theme, 0);
        assert_eq!(fixed.tab_order, (0..n).collect::<Vec<_>>());
        assert_eq!(fixed.tab_hidden.len(), n);
    }

    #[test]
    fn all_hidden_is_rejected() {
        let cfg = Config { tab_hidden: vec![true; Tab::ALL.len()], ..Config::default() }.sanitized();
        assert!(cfg.tab_hidden.iter().all(|&h| !h));
    }

    #[test]
    fn a_config_from_an_older_version_keeps_its_layout() {
        // written when gitsmith had two fewer tabs, with a custom order
        let n = Tab::ALL.len();
        let old_order: Vec<usize> = (0..n - 2).rev().collect();
        let cfg = Config {
            theme: 2,
            tab_order: old_order.clone(),
            tab_hidden: vec![false; n - 2],
        }
        .sanitized();
        assert_eq!(cfg.theme, 2);
        assert_eq!(cfg.tab_order[..n - 2], old_order[..], "the user's ordering survives");
        assert_eq!(cfg.tab_order[n - 2..], [n - 2, n - 1], "new tabs land at the end");
        assert_eq!(cfg.tab_hidden.len(), n);
        assert!(cfg.tab_hidden.iter().all(|&h| !h), "new tabs start visible");
    }

    #[test]
    fn duplicates_and_unknown_tabs_are_dropped() {
        let n = Tab::ALL.len();
        let cfg = Config { tab_order: vec![3, 3, 999, 1], ..Config::default() }.sanitized();
        assert_eq!(cfg.tab_order.len(), n);
        assert_eq!(cfg.tab_order[..2], [3, 1]);
        let mut sorted = cfg.tab_order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..n).collect::<Vec<_>>(), "still a permutation");
    }
}
