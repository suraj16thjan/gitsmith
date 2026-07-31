pub mod glab;
pub mod gh;
pub(crate) mod json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Issues,
    MergeRequests,
    Pipelines,
    Runners,
    Releases,
    Tags,
    Branches,
    Commits,
    Todos,
    Milestones,
}

impl Tab {
    pub const ALL: [Tab; 10] = [
        Tab::Issues,
        Tab::MergeRequests,
        Tab::Pipelines,
        Tab::Runners,
        Tab::Releases,
        Tab::Tags,
        Tab::Branches,
        Tab::Commits,
        Tab::Todos,
        Tab::Milestones,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Issues => "Issues",
            Tab::MergeRequests => "MRs/PRs",
            Tab::Pipelines => "Pipelines",
            Tab::Runners => "Runners",
            Tab::Releases => "Releases",
            Tab::Tags => "Tags",
            Tab::Branches => "Branches",
            Tab::Commits => "Commits",
            Tab::Todos => "Todos",
            Tab::Milestones => "Milestones",
        }
    }

    pub fn headers(&self) -> &'static [&'static str] {
        match self {
            Tab::Issues => &["#", "Title", "State", "Author", "Updated"],
            Tab::MergeRequests => &["#", "Title", "State", "Author", "Updated"],
            Tab::Pipelines => &["#", "Status", "Ref", "Updated"],
            Tab::Runners => &["ID", "Description", "Status"],
            Tab::Releases => &["Tag", "Name", "Published"],
            Tab::Tags => &["Tag", "Commit", "Message"],
            Tab::Branches => &["Branch", "Protected", "Commit"],
            Tab::Commits => &["SHA", "Message", "Author", "When"],
            Tab::Todos => &["Project", "Title", "Updated"],
            Tab::Milestones => &["Title", "State", "Due"],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub cells: Vec<String>,
    pub web_url: String,
    /// The item's raw JSON object, retained so the detail view can render metadata
    /// and the description without a re-fetch. `Null` in tests that don't need it.
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    IssueClose,
    IssueReopen,
    MrApprove,
    MrMerge,
    MrClose,
    PipelineRetry,
    PipelineCancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobAction {
    Retry,
    Cancel,
}

impl JobAction {
    pub fn label(&self, id: &str) -> String {
        match self {
            JobAction::Retry => format!("Retry job #{id}?"),
            JobAction::Cancel => format!("Cancel job #{id}?"),
        }
    }
}

impl Action {
    pub fn label(&self, id: &str) -> String {
        match self {
            Action::IssueClose => format!("Close issue #{id}?"),
            Action::IssueReopen => format!("Reopen issue #{id}?"),
            Action::MrApprove => format!("Approve MR !{id}?"),
            Action::MrMerge => format!("Merge MR !{id}?"),
            Action::MrClose => format!("Close MR !{id}?"),
            Action::PipelineRetry => format!("Retry pipeline #{id}?"),
            Action::PipelineCancel => format!("Cancel pipeline #{id}?"),
        }
    }
}

/// Map a key on a tab to a mutation. `None` = no action for that key on that tab.
/// (Comment is not here — it needs the terminal and is routed separately.)
pub fn action_for(tab: Tab, key: char) -> Option<Action> {
    match (tab, key) {
        (Tab::Issues, 'c') => Some(Action::IssueClose),
        (Tab::Issues, 'O') => Some(Action::IssueReopen),
        (Tab::MergeRequests, 'a') => Some(Action::MrApprove),
        (Tab::MergeRequests, 'M') => Some(Action::MrMerge),
        (Tab::MergeRequests, 'c') => Some(Action::MrClose),
        (Tab::Pipelines, 'r') => Some(Action::PipelineRetry),
        (Tab::Pipelines, 'd') => Some(Action::PipelineCancel),
        _ => None,
    }
}

/// Whether a key is an action key on *some* tab — used to flash "not available
/// here" when it's pressed on a tab that doesn't bind it. `C` (comment) included.
pub fn is_action_key(key: char) -> bool {
    matches!(key, 'a' | 'M' | 'c' | 'O' | 'r' | 'd' | 'C')
}

pub trait Backend: Send + Sync {
    fn list(&self, tab: Tab) -> anyhow::Result<Vec<Row>>;

    fn act(&self, _action: Action, _id: &str) -> anyhow::Result<()> {
        anyhow::bail!("action not supported by this backend")
    }

    fn comment(&self, _id: &str, _body: &str) -> anyhow::Result<()> {
        anyhow::bail!("comment not supported by this backend")
    }

    fn diff(&self, _tab: Tab, _id: &str) -> anyhow::Result<String> {
        anyhow::bail!("diff not supported by this backend")
    }

    /// Unified diff for a commit SHA (for the Commits tab).
    fn commit_diff(&self, _sha: &str) -> anyhow::Result<String> {
        anyhow::bail!("commit diff not supported by this backend")
    }

    /// Raw JSON array of discussion comments/notes for an item (Issues/MRs only).
    fn comments(&self, _tab: Tab, _id: &str) -> anyhow::Result<String> {
        anyhow::bail!("comments not supported by this backend")
    }

    /// Raw JSON of a pipeline's jobs.
    fn jobs(&self, _pipeline_id: &str) -> anyhow::Result<String> {
        anyhow::bail!("jobs not supported by this backend")
    }

    /// Raw log text for a job.
    fn job_log(&self, _job_id: &str) -> anyhow::Result<String> {
        anyhow::bail!("logs not supported by this backend")
    }

    /// Retry or cancel a single job.
    fn job_act(&self, _action: JobAction, _job_id: &str) -> anyhow::Result<()> {
        anyhow::bail!("job action not supported by this backend")
    }

    /// The user's repositories on this host as `owner/repo` (or `group/…/repo`).
    fn repos(&self) -> anyhow::Result<Vec<String>> {
        anyhow::bail!("repo list not supported by this backend")
    }
}

// --- active repo override (set by the `P` repo picker) ---

fn repo_override_cell() -> &'static Mutex<Option<String>> {
    static O: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    O.get_or_init(|| Mutex::new(None))
}

/// Point all subsequent commands at `repo` (`owner/repo`), or clear with `None`.
pub fn set_repo_override(repo: Option<String>) {
    *repo_override_cell().lock().unwrap_or_else(|e| e.into_inner()) = repo;
}

pub fn repo_override() -> Option<String> {
    repo_override_cell().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Glab,
    Gh,
}

pub fn backend_kind(remote_url: &str) -> Kind {
    if remote_url.contains("gitlab") {
        Kind::Glab
    } else {
        Kind::Gh
    }
}

/// Decide which backend to use from env overrides and an optional git remote URL.
/// Priority: `GITLAB_HOST` > `GH_HOST` > remote URL. `None` means undetermined.
pub fn choose_kind(gitlab_host: bool, gh_host: bool, remote_url: Option<&str>) -> Option<Kind> {
    if gitlab_host {
        Some(Kind::Glab)
    } else if gh_host {
        Some(Kind::Gh)
    } else {
        remote_url.map(backend_kind)
    }
}

/// The current repo's remote URL: prefer `origin`, else the first configured remote.
pub fn remote_url() -> Option<String> {
    let get = |name: &str| {
        run("git", &["remote", "get-url", name])
            .ok()
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
    };
    if let Some(url) = get("origin") {
        return Some(url);
    }
    let first = run("git", &["remote"]).ok()?;
    get(first.split_whitespace().next()?)
}

/// Parse an `owner/repo` (or `group/…/repo`) path from a git remote URL. Handles
/// `https://host/a/b/c(.git)`, `ssh://git@host:22/a/b/c`, and `git@host:a/b/c(.git)`.
pub(crate) fn repo_from_url(url: &str) -> Option<String> {
    let url = url.trim().strip_suffix(".git").unwrap_or(url.trim());
    let path = match url.split_once("://") {
        Some((_, host_and_path)) => host_and_path.split_once('/').map(|(_, p)| p)?,
        None => url.split_once(':').map(|(_, p)| p)?,
    };
    let path = path.trim_matches('/');
    (!path.is_empty()).then(|| path.to_string())
}

/// The current repo's display name, derived from its git remote.
pub fn current_repo() -> Option<String> {
    remote_url().as_deref().and_then(repo_from_url)
}

/// Detect the backend, its kind, and the repo name for the current directory.
/// Honors `GITLAB_HOST`/`GH_HOST`, then falls back to the git remote.
pub fn detect() -> anyhow::Result<(std::sync::Arc<dyn Backend>, Kind, Option<String>)> {
    let gitlab_host = std::env::var_os("GITLAB_HOST").is_some();
    let gh_host = std::env::var_os("GH_HOST").is_some();
    let kind = choose_kind(gitlab_host, gh_host, remote_url().as_deref()).ok_or_else(|| {
        anyhow::anyhow!(
            "could not determine host — run glabtui inside a git repo with a GitLab/GitHub \
             remote, or set GITLAB_HOST or GH_HOST"
        )
    })?;
    let backend: std::sync::Arc<dyn Backend> = match kind {
        Kind::Glab => std::sync::Arc::new(glab::GlabBackend),
        Kind::Gh => std::sync::Arc::new(gh::GhBackend),
    };
    Ok((backend, kind, current_repo()))
}

pub fn next_index(i: usize) -> usize {
    (i + 1) % Tab::ALL.len()
}

pub fn prev_index(i: usize) -> usize {
    (i + Tab::ALL.len() - 1) % Tab::ALL.len()
}

// --- executed-command log (for the `x` overlay + status line) ---

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

struct CmdEntry {
    id: u64,
    line: String,
    ok: Option<bool>, // None = still running / recorded before completion
}

fn cmd_log() -> &'static Mutex<VecDeque<CmdEntry>> {
    static LOG: OnceLock<Mutex<VecDeque<CmdEntry>>> = OnceLock::new();
    LOG.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn record_cmd(line: String) -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut log = cmd_log().lock().unwrap_or_else(|e| e.into_inner());
    log.push_back(CmdEntry { id, line, ok: None });
    while log.len() > 200 {
        log.pop_front();
    }
    id
}

fn mark_cmd(id: u64, ok: bool) {
    let mut log = cmd_log().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(e) = log.iter_mut().find(|e| e.id == id) {
        e.ok = Some(ok);
    }
}

/// Snapshot of recently-run commands (oldest→newest) with success markers.
pub fn recent_cmds() -> Vec<(String, Option<bool>)> {
    let log = cmd_log().lock().unwrap_or_else(|e| e.into_inner());
    log.iter().map(|e| (e.line.clone(), e.ok)).collect()
}

pub(crate) fn run(cmd: &str, args: &[&str]) -> anyhow::Result<String> {
    use anyhow::{bail, Context};
    let id = record_cmd(format!("{cmd} {}", args.join(" ")).trim_end().to_string());
    let result = (|| {
        let mut command = std::process::Command::new(cmd);
        command.args(args);
        // gh honors GH_REPO for both subcommands and `api {owner}/{repo}` placeholders,
        // so a repo override needs no per-command flags on the gh side.
        if cmd == "gh"
            && let Some(repo) = repo_override()
        {
            command.env("GH_REPO", repo);
        }
        let out = command
            .output()
            .with_context(|| format!("failed to run `{cmd}` — is it installed and on PATH?"))?;
        if !out.status.success() {
            bail!("`{cmd}` failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        anyhow::Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    })();
    mark_cmd(id, result.is_ok());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_records_success_and_failure() {
        // `false` exits non-zero → recorded ok=false; `true` exits zero → ok=true.
        // Unique markers avoid collisions with other tests' entries (args ignored).
        let _ = run("false", &["glabtui-marker-fail"]);
        let _ = run("true", &["glabtui-marker-ok"]);
        let cmds = recent_cmds();
        let fail = cmds.iter().find(|(l, _)| l.contains("glabtui-marker-fail"));
        let ok = cmds.iter().find(|(l, _)| l.contains("glabtui-marker-ok"));
        assert_eq!(fail.and_then(|(_, o)| *o), Some(false));
        assert_eq!(ok.and_then(|(_, o)| *o), Some(true));
    }

    #[test]
    fn all_tabs_present() {
        assert_eq!(Tab::ALL.len(), 10);
        assert_eq!(Tab::Branches.headers().len(), 3);
        assert_eq!(Tab::Commits.headers().len(), 4);
        assert_eq!(Tab::Issues.title(), "Issues");
        assert_eq!(Tab::Issues.headers().len(), 5);
        assert_eq!(Tab::MergeRequests.headers().len(), 5);
        assert_eq!(Tab::Pipelines.headers().len(), 4);
        assert_eq!(Tab::Runners.headers().len(), 3);
        assert_eq!(Tab::Releases.headers().len(), 3);
        assert_eq!(Tab::Tags.title(), "Tags");
        assert_eq!(Tab::Tags.headers().len(), 3);
        assert_eq!(Tab::Todos.headers().len(), 3);
        assert_eq!(Tab::Milestones.headers().len(), 3);
    }

    #[test]
    fn kind_from_remote() {
        assert!(matches!(backend_kind("git@gitlab.com:o/r.git"), Kind::Glab));
        assert!(matches!(backend_kind("https://gitlab.example.com/o/r"), Kind::Glab));
        assert!(matches!(backend_kind("git@github.com:o/r.git"), Kind::Gh));
        assert!(matches!(backend_kind("https://github.com/o/r.git"), Kind::Gh));
    }

    #[test]
    fn choose_kind_priority() {
        // env overrides win, and don't require a remote
        assert!(matches!(choose_kind(true, false, None), Some(Kind::Glab)));
        assert!(matches!(choose_kind(false, true, None), Some(Kind::Gh)));
        // GITLAB_HOST beats a github remote
        assert!(matches!(
            choose_kind(true, false, Some("https://github.com/o/r")),
            Some(Kind::Glab)
        ));
        // no env → fall back to the remote URL
        assert!(matches!(
            choose_kind(false, false, Some("git@gitlab.innovatetech.io:o/r.git")),
            Some(Kind::Glab)
        ));
        assert!(matches!(
            choose_kind(false, false, Some("https://github.com/o/r")),
            Some(Kind::Gh)
        ));
        // no signal at all → undetermined
        assert!(choose_kind(false, false, None).is_none());
    }

    #[test]
    fn index_wraps() {
        assert_eq!(next_index(9), 0);
        assert_eq!(prev_index(0), 9);
        assert_eq!(next_index(0), 1);
    }

    #[test]
    fn action_key_mapping() {
        assert_eq!(action_for(Tab::MergeRequests, 'M'), Some(Action::MrMerge));
        assert_eq!(action_for(Tab::MergeRequests, 'a'), Some(Action::MrApprove));
        assert_eq!(action_for(Tab::Issues, 'c'), Some(Action::IssueClose));
        assert_eq!(action_for(Tab::Pipelines, 'r'), Some(Action::PipelineRetry));
        // wrong tab → no action
        assert_eq!(action_for(Tab::Pipelines, 'a'), None);
        // action key recognized even where unbound (drives the "not available" flash)
        assert!(is_action_key('a'));
        assert!(!is_action_key('z'));
    }

    #[test]
    fn action_label_includes_id() {
        assert_eq!(Action::MrMerge.label("123"), "Merge MR !123?");
        assert_eq!(Action::IssueClose.label("7"), "Close issue #7?");
    }
}
