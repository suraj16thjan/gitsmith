use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// A unique temp path for a one-shot comment buffer.
/// ponytail: hand-rolled temp path (no `tempfile` crate) for a single-use file.
fn comment_temp_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("glabtui-comment-{}-{nanos}.md", std::process::id()))
}

/// Open $EDITOR (fallback `vi`) on a temp file and return the trimmed body,
/// or None if the editor exited non-zero or the buffer was left empty.
/// The caller MUST have already suspended the TUI (raw mode / alt screen off).
pub fn edit_comment() -> Result<Option<String>> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let path = comment_temp_path();
    std::fs::write(&path, b"").with_context(|| format!("creating {}", path.display()))?;
    let result = (|| {
        let status = std::process::Command::new(&editor)
            .arg(&path)
            .status()
            .with_context(|| format!("launching editor `{editor}`"))?;
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        if !status.success() {
            return Ok(None);
        }
        let body = body.trim().to_string();
        Ok((!body.is_empty()).then_some(body))
    })();
    let _ = std::fs::remove_file(&path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_paths_are_unique_and_in_tempdir() {
        let a = comment_temp_path();
        let b = comment_temp_path();
        assert_ne!(a, b);
        assert!(a.starts_with(std::env::temp_dir()));
        assert_eq!(a.extension().and_then(|e| e.to_str()), Some("md"));
    }
}
