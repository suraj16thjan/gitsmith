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
    Members,
}

impl Tab {
    pub const ALL: [Tab; 11] = [
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
        Tab::Members,
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
            Tab::Members => "Members",
        }
    }

    /// Relative column widths, one per header — the table gives each column a
    /// share of the row proportional to its weight (equal shares would squeeze
    /// titles to nothing next to a 3-char id).
    pub fn weights(&self) -> &'static [u16] {
        match self {
            Tab::Issues => &[5, 34, 18, 10, 13, 12],
            Tab::MergeRequests => &[4, 23, 15, 15, 9, 10, 12],
            Tab::Pipelines => &[6, 14, 40, 14],
            Tab::Runners => &[8, 50, 14],
            Tab::Releases => &[20, 40, 16],
            Tab::Tags => &[22, 12, 45],
            Tab::Branches => &[32, 20, 12, 12],
            Tab::Commits => &[10, 45, 18, 14],
            Tab::Todos => &[24, 50, 14],
            Tab::Milestones => &[45, 14, 16],
            Tab::Members => &[24, 30, 16, 14],
        }
    }

    pub fn headers(&self) -> &'static [&'static str] {
        match self {
            Tab::Issues => &["#", "Title", "Labels", "State", "Author", "Updated"],
            Tab::MergeRequests => &["#", "Title", "Branch", "Labels", "State", "Author", "Updated"],
            Tab::Pipelines => &["#", "Status", "Ref", "Updated"],
            Tab::Runners => &["ID", "Description", "Status"],
            Tab::Releases => &["Tag", "Name", "Published"],
            Tab::Tags => &["Tag", "Commit", "Message"],
            Tab::Branches => &["Branch", "Owner", "Protected", "Commit"],
            Tab::Commits => &["SHA", "Message", "Author", "When"],
            Tab::Todos => &["Project", "Title", "Updated"],
            Tab::Milestones => &["Title", "State", "Due"],
            Tab::Members => &["User", "Name", "Role", "State"],
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

/// The roles that can be granted on each host, weakest first. GitLab names its
/// access levels; GitHub names permissions. Both lists are what the respective
/// API accepts, so they work on any instance.
pub fn member_roles(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Glab => &["Guest", "Reporter", "Developer", "Maintainer", "Owner"],
        Kind::Gh => &["read", "triage", "write", "maintain", "admin"],
    }
}

/// GitLab's numeric access level for a role name (the members API takes numbers).
pub fn access_level(role: &str) -> Option<u32> {
    Some(match role.to_lowercase().as_str() {
        "minimal" => 5,
        "guest" => 10,
        "reporter" => 20,
        "developer" => 30,
        "maintainer" => 40,
        "owner" => 50,
        _ => return None,
    })
}

/// A role name for GitLab's numeric access level.
pub fn role_name(level: u64) -> String {
    match level {
        50 => "Owner",
        40 => "Maintainer",
        30 => "Developer",
        20 => "Reporter",
        10 => "Guest",
        5 => "Minimal",
        _ => return level.to_string(),
    }
    .to_string()
}

/// How much power a member has, for sorting. Reads GitLab's `access_level` or
/// GitHub's `role_name`/`permissions`, so one comparator covers both.
fn member_rank(raw: &serde_json::Value) -> u64 {
    if let Some(level) = raw.get("access_level").and_then(serde_json::Value::as_u64) {
        return level;
    }
    if let Some(role) = raw.get("role_name").and_then(serde_json::Value::as_str) {
        return match role {
            "admin" => 50,
            "maintain" => 40,
            "write" | "push" => 30,
            "triage" => 20,
            _ => 10,
        };
    }
    let perm = |k: &str| raw.get("permissions").and_then(|p| p.get(k)).and_then(serde_json::Value::as_bool) == Some(true);
    if perm("admin") {
        50
    } else if perm("maintain") {
        40
    } else if perm("push") {
        30
    } else if perm("triage") {
        20
    } else {
        10
    }
}

/// A merge request / pull request to open. Empty `source` or `target` defer to
/// the CLI, which uses the checked-out branch and the repo's default branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewMr {
    pub source: String,
    pub target: String,
    pub title: String,
    pub description: String,
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

// --- row ordering ---

/// Impose a stable order on a tab's rows.
///
/// Every endpoint returns its own order (GitLab sorts issues by created, tags by
/// updated, branches alphabetically; GitHub differs again), so a list looked
/// arbitrary. gitsmith normalizes: most-recent-first wherever there's a
/// timestamp, natural name order for refs, soonest-due-first for milestones.
/// The sort is stable, so rows the key can't distinguish keep the API's order.
pub fn sort_rows(tab: Tab, rows: &mut [Row]) {
    match tab {
        // Anything still open comes first — that's the work in front of you —
        // and within each group the most recently touched leads.
        Tab::Issues | Tab::MergeRequests => {
            by_time_desc(rows, &["updated_at", "created_at"]);
            rows.sort_by_key(|r| !is_open(&r.raw)); // stable: keeps the time order
        }
        Tab::Pipelines | Tab::Todos => by_time_desc(rows, &["updated_at", "created_at"]),
        Tab::Releases => by_time_desc(rows, &["released_at", "published_at", "created_at"]),
        Tab::Commits => by_time_desc(
            rows,
            &["created_at", "committed_date", "commit.author.date", "commit.committer.date"],
        ),
        // GitHub's tag list carries no date, so fall back to the name.
        Tab::Tags => rows.sort_by(|a, b| {
            let (ta, tb) = (time_of(&a.raw, TAG_TIMES), time_of(&b.raw, TAG_TIMES));
            tb.cmp(&ta).then_with(|| natural_cmp(&b.id, &a.id))
        }),
        // Protected branches lead: they're the ones you merge into.
        Tab::Branches => rows.sort_by(|a, b| {
            is_protected(&b.raw).cmp(&is_protected(&a.raw)).then_with(|| natural_cmp(&a.id, &b.id))
        }),
        // Most access first — that's who you look for on a members list.
        Tab::Members => rows.sort_by(|a, b| {
            member_rank(&b.raw).cmp(&member_rank(&a.raw)).then_with(|| natural_cmp(&a.id, &b.id))
        }),
        Tab::Runners => rows.sort_by_key(|r| std::cmp::Reverse(num_of(&r.raw, "id"))),
        // Soonest deadline first; undated milestones last.
        Tab::Milestones => rows.sort_by(|a, b| match (due_of(&a.raw), due_of(&b.raw)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }),
    }
}

const TAG_TIMES: &[&str] = &["commit.created_at", "commit.committed_date"];

/// Whether an issue/MR is still open. GitLab says `opened`, GitHub says `open`;
/// `closed`, `merged` and `locked` are all "not open".
fn is_open(raw: &serde_json::Value) -> bool {
    matches!(raw.get("state").and_then(serde_json::Value::as_str), Some("opened" | "open"))
}

fn is_protected(raw: &serde_json::Value) -> bool {
    raw.get("protected").and_then(serde_json::Value::as_bool) == Some(true)
}

fn by_time_desc(rows: &mut [Row], paths: &[&str]) {
    rows.sort_by_key(|r| std::cmp::Reverse(time_of(&r.raw, paths)));
}

/// Follow a dotted path (`commit.author.date`) into a JSON object.
fn at<'a>(v: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.').try_fold(v, |cur, seg| cur.get(seg))
}

/// The first of `paths` holding an RFC-3339 timestamp, as UTC. Both hosts emit
/// RFC-3339, but GitLab uses the instance's offset — so parse, never compare the
/// strings.
fn time_of(v: &serde_json::Value, paths: &[&str]) -> Option<chrono::DateTime<chrono::Utc>> {
    paths.iter().find_map(|p| {
        let s = at(v, p)?.as_str()?;
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.with_timezone(&chrono::Utc))
    })
}

fn num_of(v: &serde_json::Value, key: &str) -> Option<u64> {
    v.get(key).and_then(serde_json::Value::as_u64)
}

/// A milestone's due date as `YYYY-MM-DD` — `due_date` on GitLab, `due_on`
/// (a full timestamp) on GitHub. ISO dates compare correctly as strings.
fn due_of(v: &serde_json::Value) -> Option<String> {
    ["due_date", "due_on"]
        .iter()
        .find_map(|k| v.get(*k).and_then(serde_json::Value::as_str))
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(10).collect())
}

/// Compare names the way a person reads them: runs of digits compare as numbers,
/// so `v1.10` sorts after `v1.9`, and the rest compares case-insensitively.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(cx), Some(cy)) => {
                if cx.is_ascii_digit() && cy.is_ascii_digit() {
                    let nx: String = take_digits(&mut x);
                    let ny: String = take_digits(&mut y);
                    // Compare as numbers; equal values fall through to the rest
                    // of the string (so `01` and `1` don't compare as different).
                    let ord = nx.trim_start_matches('0').len().cmp(&ny.trim_start_matches('0').len())
                        .then_with(|| nx.trim_start_matches('0').cmp(ny.trim_start_matches('0')));
                    if ord != Ordering::Equal {
                        return ord;
                    }
                } else {
                    let ord = cx.to_ascii_lowercase().cmp(&cy.to_ascii_lowercase());
                    if ord != Ordering::Equal {
                        return ord;
                    }
                    x.next();
                    y.next();
                }
            }
        }
    }
}

fn take_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied() {
        if !c.is_ascii_digit() {
            break;
        }
        s.push(c);
        it.next();
    }
    s
}

/// Rows requested per page. The hosts cap REST pages at 100.
pub const PER_PAGE: u32 = 100;

/// How many pages a tab will pull in the background before stopping. Five pages
/// is 500 rows — enough for any repo you'd read in a TUI, and a hard stop so a
/// huge project can't spool forever.
pub const MAX_PAGES: u32 = 5;

pub trait Backend: Send + Sync {
    /// One page of rows, 1-indexed. A short page means the last one.
    fn list(&self, tab: Tab, page: u32) -> anyhow::Result<Vec<Row>>;

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

    /// Raw log text for a job (a single full fetch — the non-streaming fallback).
    fn job_log(&self, _job_id: &str) -> anyhow::Result<String> {
        anyhow::bail!("logs not supported by this backend")
    }

    /// The command (program + argv) that streams a job's live trace to stdout,
    /// or `None` when the backend can't stream (use `job_log` polling instead).
    fn trace_cmd(&self, _job_id: &str) -> Option<(String, Vec<String>)> {
        None
    }

    /// Retry or cancel a single job.
    fn job_act(&self, _action: JobAction, _job_id: &str) -> anyhow::Result<()> {
        anyhow::bail!("job action not supported by this backend")
    }

    /// The user's repositories on this host as `owner/repo` (or `group/…/repo`).
    fn repos(&self) -> anyhow::Result<Vec<String>> {
        anyhow::bail!("repo list not supported by this backend")
    }

    /// Open a merge request / pull request; returns whatever the CLI prints,
    /// which is the new MR's URL.
    fn create_mr(&self, _mr: &NewMr) -> anyhow::Result<String> {
        anyhow::bail!("creating merge requests is not supported by this backend")
    }

    /// Grant `user` the named `role` on the current repo.
    fn add_member(&self, _user: &str, _role: &str) -> anyhow::Result<String> {
        anyhow::bail!("adding members is not supported by this backend")
    }

    /// Open an issue; returns the CLI's output, which is the new issue's URL.
    fn create_issue(&self, _title: &str, _description: &str) -> anyhow::Result<String> {
        anyhow::bail!("creating issues is not supported by this backend")
    }

    /// Start a CI pipeline on `branch`; returns the CLI's output (the pipeline URL).
    fn create_pipeline(&self, _branch: &str) -> anyhow::Result<String> {
        anyhow::bail!("creating pipelines is not supported by this backend")
    }

    /// Tag `branch` with `name`; returns the CLI's output (the tag URL or name).
    fn create_tag(&self, _branch: &str, _name: &str) -> anyhow::Result<String> {
        anyhow::bail!("creating tags is not supported by this backend")
    }
}

// --- active host override (set by the host picker) ---

fn host_override_cell() -> &'static Mutex<Option<String>> {
    static H: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(None))
}

/// Point all subsequent commands at `host` (e.g. `gitlab.example.com`).
///
/// Without this, a CLI invoked outside a git repo falls back to its default host
/// — `gitlab.com` / `github.com` — where a self-hosted user has no token, and
/// `glab api projects?membership=true` then returns an empty list with a zero
/// exit code: no error, no repos.
pub fn set_host_override(host: Option<String>) {
    *host_override_cell().lock().unwrap_or_else(|e| e.into_inner()) = host;
}

pub fn host_override() -> Option<String> {
    host_override_cell().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Where a set of hosts came from, so the picker can say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSource {
    /// Configured hosts, the CLI's default first.
    pub hosts: Vec<String>,
    /// Human label: the config file the hosts were read from, or how else we
    /// found them. Empty when there are none.
    pub origin: String,
}

/// The hosts a CLI can talk to.
///
/// Read from the CLI's own config file first — it lists every configured
/// instance, costs no network call, and is how someone with several GitLab
/// accounts (work self-hosted + gitlab.com) keeps them side by side. Only if no
/// config file turns up do we fall back to `<cli> auth status`, which is slower
/// because it calls each API.
///
/// `GITLAB_HOST` / `GH_HOST` win over both — that's what the CLIs themselves
/// honor, so gitsmith must agree.
pub fn auth_hosts(kind: Kind) -> HostSource {
    let (cmd, env) = match kind {
        Kind::Glab => ("glab", "GITLAB_HOST"),
        Kind::Gh => ("gh", "GH_HOST"),
    };
    if let Some(h) = std::env::var_os(env).and_then(|v| v.into_string().ok()).filter(|h| !h.is_empty()) {
        return HostSource { hosts: vec![h], origin: format!("${env}") };
    }
    if let Some(path) = cli_config_path(kind, |k| std::env::var_os(k))
        && let Ok(text) = std::fs::read_to_string(&path)
    {
        // Only host names are taken from this file — it also holds tokens.
        let parsed = parse_config_hosts(kind, &text);
        if !parsed.is_empty() {
            return HostSource { hosts: parsed, origin: path.display().to_string() };
        }
    }
    let hosts = parse_auth_hosts(&probe(cmd, &["auth", "status"]));
    let origin = if hosts.is_empty() { String::new() } else { format!("{cmd} auth status") };
    HostSource { hosts, origin }
}

/// The CLI's config file, looked for where that CLI itself looks: its own
/// `*_CONFIG_DIR` override, then XDG, then `~/.config`, then the platform spot
/// (macOS Application Support, Windows `%APPDATA%`).
fn cli_config_path(kind: Kind, env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let var = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let (dir_var, dir_name, file, mac_dir) = match kind {
        Kind::Glab => ("GLAB_CONFIG_DIR", "glab-cli", "config.yml", "glab-cli"),
        Kind::Gh => ("GH_CONFIG_DIR", "gh", "hosts.yml", "GitHub CLI"),
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(d) = var(dir_var) {
        candidates.push(d.join(file));
    }
    if let Some(d) = var("XDG_CONFIG_HOME").filter(|p| p.is_absolute()) {
        candidates.push(d.join(dir_name).join(file));
    }
    if let Some(home) = var("HOME") {
        candidates.push(home.join(".config").join(dir_name).join(file));
        candidates.push(home.join("Library").join("Application Support").join(mac_dir).join(file));
    }
    if let Some(appdata) = var("APPDATA").or_else(|| var("USERPROFILE").map(|h| h.join("AppData").join("Roaming"))) {
        candidates.push(appdata.join(mac_dir).join(file));
        candidates.push(appdata.join(dir_name).join(file));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Host names from a CLI config file, the CLI's default host first.
///
/// glab nests them under `hosts:` and names its default in a top-level `host:`;
/// gh's `hosts.yml` is a flat map of host to settings. Both are read with a
/// small indentation scan rather than a YAML dependency — the shape is fixed and
/// we only ever want the keys.
pub(crate) fn parse_config_hosts(kind: Kind, text: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    let mut default: Option<String> = None;
    let mut in_hosts = matches!(kind, Kind::Gh); // gh's file *is* the host map
    // The indent glab nests its host keys at — discovered from the first key
    // under `hosts:` rather than assumed, since glab writes 4-space indents but
    // hand-edited files vary. Deeper per-host settings sit further in and are
    // skipped by this level check.
    let mut host_indent: Option<usize> = None;

    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() || trimmed.trim_start().starts_with('#') {
            continue;
        }
        let indent = trimmed.len() - trimmed.trim_start().len();
        let Some((key, value)) = trimmed.trim().split_once(':') else { continue };
        let (key, value) = (key.trim().trim_matches(['"', '\'']), value.trim());

        if indent == 0 && kind == Kind::Glab {
            in_hosts = key == "hosts";
            if !in_hosts {
                host_indent = None;
            }
            if key == "host" && !value.is_empty() {
                default = Some(value.trim_matches(['"', '\'']).to_string());
            }
            continue;
        }
        // A host is a key with no inline value: glab at the first indent level
        // under `hosts:`, gh at the top level of its own file.
        let host_level = if kind == Kind::Glab {
            in_hosts && indent == *host_indent.get_or_insert(indent)
        } else {
            indent == 0
        };
        if in_hosts && host_level && value.is_empty() && !key.is_empty() && !hosts.iter().any(|h| h == key) {
            hosts.push(key.to_string());
        }
    }

    // The CLI's default goes first so the picker starts where the CLI would.
    if let Some(d) = default
        && let Some(i) = hosts.iter().position(|h| *h == d)
    {
        hosts.swap(0, i);
    }
    hosts
}

/// Hostnames from `auth status` output — the lines reading
/// "Logged in to gitlab.example.com as alice" (glab) or
/// "Logged in to github.com account alice" (gh). Hosts that failed to
/// authenticate are reported differently ("x host: API call failed"), so they
/// fall out here, which is exactly what we want.
pub(crate) fn parse_auth_hosts(out: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for line in out.lines() {
        let Some(rest) = line.split("Logged in to ").nth(1) else { continue };
        let Some(host) = rest.split_whitespace().next() else { continue };
        let host = host.trim_end_matches([':', ',']).to_string();
        if !host.is_empty() && !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
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

/// The `web_url` of a created object, falling back to the raw output for CLIs
/// that just print the URL.
pub fn created_url(out: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(out) {
        let url = json::s(&v, "web_url");
        if !url.is_empty() {
            return url;
        }
    }
    out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()
}

/// The checked-out branch, when there is one — used to prefill a new MR's source
/// branch. `None` outside a git repo or on a detached HEAD.
pub fn current_branch() -> Option<String> {
    let out = run("git", &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    let branch = out.trim().to_string();
    (!branch.is_empty() && branch != "HEAD").then_some(branch)
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
/// The host to talk to, from `GITLAB_HOST` / `GH_HOST` or the repo's remote.
/// `None` outside a git repo (or in one with no GitLab/GitHub remote) — the UI
/// then asks the user which host to use instead of bailing out.
pub fn detect_kind() -> Option<Kind> {
    let gitlab_host = std::env::var_os("GITLAB_HOST").is_some();
    let gh_host = std::env::var_os("GH_HOST").is_some();
    choose_kind(gitlab_host, gh_host, remote_url().as_deref())
}

pub fn backend_for(kind: Kind) -> std::sync::Arc<dyn Backend> {
    match kind {
        Kind::Glab => std::sync::Arc::new(glab::GlabBackend),
        Kind::Gh => std::sync::Arc::new(gh::GhBackend),
    }
}

// --- executed-command log (for the `x` overlay + status line) ---

use std::collections::VecDeque;
use std::path::PathBuf;
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

pub(crate) fn record_cmd(line: String) -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut log = cmd_log().lock().unwrap_or_else(|e| e.into_inner());
    log.push_back(CmdEntry { id, line, ok: None });
    while log.len() > 200 {
        log.pop_front();
    }
    id
}

pub(crate) fn mark_cmd(id: u64, ok: bool) {
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

/// Run a command purely to read what it prints: stdout and stderr combined,
/// exit status ignored. `glab auth status` writes to stderr and exits non-zero
/// when *any* configured host fails to authenticate — including hosts we don't
/// care about — so `run` would throw away the answer.
pub(crate) fn probe(cmd: &str, args: &[&str]) -> String {
    let id = record_cmd(format!("{cmd} {}", args.join(" ")).trim_end().to_string());
    let out = std::process::Command::new(cmd).args(args).output();
    mark_cmd(id, out.is_ok());
    match out {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            text
        }
        Err(_) => String::new(),
    }
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
        // Same idea for the host: both CLIs read it from the environment, so one
        // env var covers subcommands and raw `api` calls alike.
        if let Some(host) = host_override() {
            match cmd {
                "glab" => { command.env("GITLAB_HOST", host); }
                "gh" => { command.env("GH_HOST", host); }
                _ => {}
            }
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

    fn raw_row(id: &str, raw: serde_json::Value) -> Row {
        Row { id: id.into(), cells: vec![id.into()], web_url: String::new(), raw }
    }

    fn ids(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn activity_tabs_sort_newest_first() {
        let mut rows = vec![
            raw_row("old", serde_json::json!({ "updated_at": "2026-07-01T10:00:00Z" })),
            raw_row("new", serde_json::json!({ "updated_at": "2026-07-30T10:00:00Z" })),
            raw_row("mid", serde_json::json!({ "updated_at": "2026-07-15T10:00:00Z" })),
            raw_row("none", serde_json::json!({})),
        ];
        sort_rows(Tab::Issues, &mut rows);
        assert_eq!(ids(&rows), ["new", "mid", "old", "none"]);
    }

    #[test]
    fn timestamps_compare_across_offsets() {
        // GitLab stamps its own offset — 09:00+02:00 is *before* 08:00Z.
        let mut rows = vec![
            raw_row("berlin", serde_json::json!({ "updated_at": "2026-07-30T09:00:00.000+02:00" })),
            raw_row("utc", serde_json::json!({ "updated_at": "2026-07-30T08:00:00.000Z" })),
        ];
        sort_rows(Tab::MergeRequests, &mut rows);
        assert_eq!(ids(&rows), ["utc", "berlin"]);
    }

    #[test]
    fn commits_and_releases_use_their_own_time_fields() {
        let mut commits = vec![
            raw_row("gh-old", serde_json::json!({ "commit": { "author": { "date": "2026-01-01T00:00:00Z" } } })),
            raw_row("glab-new", serde_json::json!({ "created_at": "2026-06-01T00:00:00Z" })),
        ];
        sort_rows(Tab::Commits, &mut commits);
        assert_eq!(ids(&commits), ["glab-new", "gh-old"]);

        let mut releases = vec![
            raw_row("gh", serde_json::json!({ "published_at": "2026-02-01T00:00:00Z" })),
            raw_row("glab", serde_json::json!({ "released_at": "2026-05-01T00:00:00Z" })),
        ];
        sort_rows(Tab::Releases, &mut releases);
        assert_eq!(ids(&releases), ["glab", "gh"]);
    }

    #[test]
    fn refs_sort_naturally() {
        let mut branches = vec![
            raw_row("release-10", serde_json::json!({})),
            raw_row("main", serde_json::json!({})),
            raw_row("release-9", serde_json::json!({})),
            raw_row("Feature", serde_json::json!({})),
        ];
        sort_rows(Tab::Branches, &mut branches);
        assert_eq!(ids(&branches), ["Feature", "main", "release-9", "release-10"]);

        // Tags without dates (GitHub) fall back to newest-looking name first.
        let mut tags = vec![
            raw_row("v1.9.0", serde_json::json!({})),
            raw_row("v1.10.0", serde_json::json!({})),
            raw_row("v2.0.0", serde_json::json!({})),
        ];
        sort_rows(Tab::Tags, &mut tags);
        assert_eq!(ids(&tags), ["v2.0.0", "v1.10.0", "v1.9.0"]);
    }

    #[test]
    fn tags_prefer_commit_dates_when_present() {
        let mut tags = vec![
            raw_row("v2.0.0", serde_json::json!({ "commit": { "created_at": "2026-01-01T00:00:00Z" } })),
            raw_row("v1.0.0", serde_json::json!({ "commit": { "created_at": "2026-06-01T00:00:00Z" } })),
        ];
        sort_rows(Tab::Tags, &mut tags);
        assert_eq!(ids(&tags), ["v1.0.0", "v2.0.0"], "the newer tag wins over the higher version");
    }

    #[test]
    fn milestones_sort_by_due_date_with_undated_last() {
        let mut rows = vec![
            raw_row("none", serde_json::json!({})),
            raw_row("later", serde_json::json!({ "due_date": "2026-12-01" })),
            raw_row("gh-soon", serde_json::json!({ "due_on": "2026-08-01T00:00:00Z" })),
            raw_row("empty", serde_json::json!({ "due_date": "" })),
        ];
        sort_rows(Tab::Milestones, &mut rows);
        assert_eq!(ids(&rows), ["gh-soon", "later", "none", "empty"]);
    }

    #[test]
    fn runners_sort_by_id_desc() {
        let mut rows = vec![
            raw_row("7", serde_json::json!({ "id": 7 })),
            raw_row("42", serde_json::json!({ "id": 42 })),
        ];
        sort_rows(Tab::Runners, &mut rows);
        assert_eq!(ids(&rows), ["42", "7"]);
    }

    #[test]
    fn equal_keys_keep_api_order() {
        let same = serde_json::json!({ "updated_at": "2026-07-30T10:00:00Z" });
        let mut rows = vec![
            raw_row("first", same.clone()),
            raw_row("second", same.clone()),
            raw_row("third", same),
        ];
        sort_rows(Tab::Issues, &mut rows);
        assert_eq!(ids(&rows), ["first", "second", "third"]);
    }

    #[test]
    fn glab_config_lists_every_host_default_first() {
        // the shape glab actually writes: settings, then a hosts map
        let yaml = "\
git_protocol: ssh
editor:
check_update: false
host: gitlab.selfhosted.io
hosts:
  gitlab.com:
    token: SECRET
    api_host: gitlab.com
  gitlab.selfhosted.io:
    token: SECRET
    api_protocol: https
  gitlab.other.dev:
    token: SECRET
telemetry: false
";
        // the CLI's own default host comes first, and no token ever leaves the file
        assert_eq!(
            parse_config_hosts(Kind::Glab, yaml),
            ["gitlab.selfhosted.io", "gitlab.com", "gitlab.other.dev"]
        );
    }

    #[test]
    fn glab_config_handles_four_space_indent_and_nested_empty_keys() {
        // glab actually writes 4-space indents, and each host carries nested
        // settings — several with empty values (subfolder, user). Those must not
        // be mistaken for hosts, and both real hosts must be found.
        let yaml = "\
git_protocol: ssh
host: gitlab.com
hosts:
    gitlab.com:
        api_protocol: https
        subfolder:
        token: SECRET
        user:
    gitlab.innovatetech.io:
        token: SECRET
        api_host: gitlab.innovatetech.io
        user: suraj.lama
telemetry: true
";
        assert_eq!(
            parse_config_hosts(Kind::Glab, yaml),
            ["gitlab.com", "gitlab.innovatetech.io"]
        );
    }

    #[test]
    fn gh_hosts_file_is_a_flat_map() {
        let yaml = "\
github.com:
    user: alice
    oauth_token: SECRET
    git_protocol: ssh
ghe.corp.example:
    user: alice
    oauth_token: SECRET
";
        assert_eq!(parse_config_hosts(Kind::Gh, yaml), ["github.com", "ghe.corp.example"]);
    }

    #[test]
    fn config_parsing_survives_odd_files() {
        // comments, blank lines, quoted keys, and a section after `hosts:`
        let yaml = "\
# a comment
host: \"gitlab.com\"

hosts:
  # another comment
  \"gitlab.com\":
    token: SECRET

telemetry: false
other:
  not.a.host:
";
        assert_eq!(parse_config_hosts(Kind::Glab, yaml), ["gitlab.com"]);
        // nothing configured at all
        assert!(parse_config_hosts(Kind::Glab, "git_protocol: ssh\n").is_empty());
        assert!(parse_config_hosts(Kind::Glab, "").is_empty());
    }

    #[test]
    fn config_files_are_looked_for_where_each_cli_keeps_them() {
        // A directory tree standing in for a machine, so the lookup order is
        // observable without touching the real home directory.
        let root = std::env::temp_dir().join(format!("gitsmith-cfgpath-{}", std::process::id()));
        let xdg = root.join("xdg");
        let home = root.join("home");
        let mac = home.join("Library/Application Support/glab-cli");
        std::fs::create_dir_all(xdg.join("glab-cli")).unwrap();
        std::fs::create_dir_all(&mac).unwrap();
        let env = |pairs: Vec<(&str, PathBuf)>| {
            let owned: Vec<(String, std::ffi::OsString)> =
                pairs.into_iter().map(|(k, v)| (k.to_string(), v.into_os_string())).collect();
            move |k: &str| owned.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
        };

        // only the macOS location exists → that's the one used
        std::fs::write(mac.join("config.yml"), "hosts:\n").unwrap();
        let found = cli_config_path(Kind::Glab, env(vec![("HOME", home.clone())]));
        assert_eq!(found, Some(mac.join("config.yml")));

        // XDG wins once it exists
        std::fs::write(xdg.join("glab-cli/config.yml"), "hosts:\n").unwrap();
        let found = cli_config_path(Kind::Glab, env(vec![("HOME", home.clone()), ("XDG_CONFIG_HOME", xdg.clone())]));
        assert_eq!(found, Some(xdg.join("glab-cli/config.yml")));

        // and an explicit GLAB_CONFIG_DIR beats everything
        std::fs::write(root.join("config.yml"), "hosts:\n").unwrap();
        let found = cli_config_path(
            Kind::Glab,
            env(vec![("HOME", home.clone()), ("XDG_CONFIG_HOME", xdg), ("GLAB_CONFIG_DIR", root.clone())]),
        );
        assert_eq!(found, Some(root.join("config.yml")));

        // nothing configured anywhere
        assert!(cli_config_path(Kind::Glab, env(vec![("HOME", root.join("nowhere"))])).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn auth_hosts_are_read_from_status_output() {
        // glab: the failing host is reported differently and must not be offered
        let glab = "\
gitlab.com
  x gitlab.com: API call failed: GET https://gitlab.com/api/v4/user: 401 {message: 401 Unauthorized}
  ! No token found (checked config file, keyring, and environment variables).
gitlab.selfhosted.io
  ✓ Logged in to gitlab.selfhosted.io as alice (/Users/a/Library/Application Support/glab-cli/config.yml)
  ✓ Token found in configuration file (plaintext): ****";
        assert_eq!(parse_auth_hosts(glab), vec!["gitlab.selfhosted.io"]);

        // gh phrases it slightly differently ("account" instead of "as")
        let gh = "\
github.com
  ✓ Logged in to github.com account alice (keyring)
  - Active account: true";
        assert_eq!(parse_auth_hosts(gh), vec!["github.com"]);

        // several logins are all offered, in the order printed, without repeats
        let many = "  ✓ Logged in to a.io as x\n  ✓ Logged in to b.io as y\n  ✓ Logged in to a.io as z";
        assert_eq!(parse_auth_hosts(many), vec!["a.io", "b.io"]);

        assert!(parse_auth_hosts("").is_empty());
        assert!(parse_auth_hosts("not logged in anywhere").is_empty());
    }

    #[test]
    fn created_url_prefers_the_api_web_url() {
        let json = r#"{"iid":12,"web_url":"https://gitlab.example.com/o/r/-/merge_requests/12"}"#;
        assert_eq!(created_url(json), "https://gitlab.example.com/o/r/-/merge_requests/12");
        // a CLI that just prints the URL still works
        assert_eq!(created_url("https://host/o/r/-/issues/3\n"), "https://host/o/r/-/issues/3");
        assert_eq!(created_url(""), "");
    }

    #[test]
    fn open_items_lead_closed_ones() {
        let mut rows = vec![
            raw_row("closed-recent", serde_json::json!({ "state": "closed", "updated_at": "2026-07-30T10:00:00Z" })),
            raw_row("open-old", serde_json::json!({ "state": "opened", "updated_at": "2026-01-01T10:00:00Z" })),
            raw_row("open-recent", serde_json::json!({ "state": "opened", "updated_at": "2026-07-29T10:00:00Z" })),
            raw_row("merged", serde_json::json!({ "state": "merged", "updated_at": "2026-07-28T10:00:00Z" })),
        ];
        sort_rows(Tab::MergeRequests, &mut rows);
        // open first (newest first inside the group), then everything settled
        assert_eq!(ids(&rows), ["open-recent", "open-old", "closed-recent", "merged"]);

        // GitHub spells it "open"
        let mut gh = vec![
            raw_row("done", serde_json::json!({ "state": "closed", "updated_at": "2026-07-30T10:00:00Z" })),
            raw_row("todo", serde_json::json!({ "state": "open", "updated_at": "2026-07-01T10:00:00Z" })),
        ];
        sort_rows(Tab::Issues, &mut gh);
        assert_eq!(ids(&gh), ["todo", "done"]);
    }

    #[test]
    fn protected_branches_lead() {
        let mut rows = vec![
            raw_row("zzz-feature", serde_json::json!({})),
            raw_row("main", serde_json::json!({ "protected": true })),
            raw_row("aaa-feature", serde_json::json!({})),
            raw_row("develop", serde_json::json!({ "protected": true })),
        ];
        sort_rows(Tab::Branches, &mut rows);
        assert_eq!(ids(&rows), ["develop", "main", "aaa-feature", "zzz-feature"]);
    }

    #[test]
    fn members_sort_by_access_then_name() {
        let mut rows = vec![
            raw_row("zoe", serde_json::json!({ "access_level": 30 })),
            raw_row("ann", serde_json::json!({ "access_level": 50 })),
            raw_row("bob", serde_json::json!({ "access_level": 30 })),
            raw_row("guest", serde_json::json!({ "access_level": 10 })),
        ];
        sort_rows(Tab::Members, &mut rows);
        assert_eq!(ids(&rows), ["ann", "bob", "zoe", "guest"]);

        // GitHub says it in words, or only in permission flags
        let mut gh = vec![
            raw_row("reader", serde_json::json!({ "role_name": "read" })),
            raw_row("owner", serde_json::json!({ "role_name": "admin" })),
            raw_row("pusher", serde_json::json!({ "permissions": { "push": true } })),
        ];
        sort_rows(Tab::Members, &mut gh);
        assert_eq!(ids(&gh), ["owner", "pusher", "reader"]);
    }

    #[test]
    fn roles_map_to_what_each_api_takes() {
        assert_eq!(member_roles(Kind::Glab).first(), Some(&"Guest"));
        assert_eq!(member_roles(Kind::Gh).first(), Some(&"read"));
        // GitLab wants numbers, and is case-insensitive about the name we show
        assert_eq!(access_level("Developer"), Some(30));
        assert_eq!(access_level("owner"), Some(50));
        assert_eq!(access_level("nonsense"), None);
        // …and back, for display
        assert_eq!(role_name(40), "Maintainer");
        assert_eq!(role_name(50), "Owner");
        // an instance-specific custom level still shows something truthful
        assert_eq!(role_name(35), "35");
    }

    #[test]
    fn every_tab_has_one_weight_per_column() {
        for tab in Tab::ALL {
            assert_eq!(tab.weights().len(), tab.headers().len(), "{}", tab.title());
        }
    }

    #[test]
    fn all_tabs_present() {
        assert_eq!(Tab::ALL.len(), 11);
        assert_eq!(Tab::Branches.headers().len(), 4);
        assert_eq!(Tab::Commits.headers().len(), 4);
        assert_eq!(Tab::Issues.title(), "Issues");
        assert_eq!(Tab::Issues.headers().len(), 6);
        assert_eq!(Tab::MergeRequests.headers().len(), 7);
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
