use crate::backend::{action_for, is_action_key, Action, Backend, Kind, Row, Tab};
use crate::detail::{Comment, Detail};
use crate::fetch::{self, Msg};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;
use std::sync::mpsc::Sender;
use std::sync::Arc;

#[derive(Default)]
pub struct TabState {
    pub rows: Vec<Row>,
    pub table_state: TableState,
    pub loading: bool,
    pub error: Option<String>,
    /// This tab's filter query. Per-tab so a filter typed on Issues doesn't
    /// silently empty out Branches when you switch.
    pub filter: Option<String>,
    /// Highest page fetched so far. Later pages stream in behind the first.
    pub pages: u32,
    /// Rows the table last had room for — the renderer reports it back so
    /// half-page jumps match what's actually on screen.
    pub viewport_h: u16,
}

pub struct DiffView {
    pub title: String,
    pub parsed: crate::diff::ParsedDiff,
    pub scroll: u16,
    pub loading: bool,
    pub error: Option<String>,
}

pub struct DetailView {
    pub detail: Detail,
    pub comments: Vec<Comment>,
    pub scroll: u16,
    pub comments_loading: bool,
    pub comments_error: Option<String>,
}

pub struct JobsView {
    pub title: String,
    pub pipeline_id: String,
    pub jobs: Vec<crate::jobs::Job>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub last_fetch: std::time::Instant,
}

pub struct LogView {
    pub title: String,
    pub job_id: String,
    pub rows: Vec<Vec<(ratatui::style::Color, String)>>,
    pub scroll: u16,
    /// Follow mode: keep the newest lines in view and re-poll the log (like GitLab's
    /// live trace). Turned off when the user scrolls up; `G` turns it back on.
    pub follow: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub last_fetch: std::time::Instant,
    /// Last rendered content height, so leaving follow mode starts from the
    /// bottom-of-viewport line rather than jumping.
    pub viewport_h: u16,
}

pub struct RepoPicker {
    pub all: Vec<String>,
    pub filter: String,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl RepoPicker {
    /// Repos matching the current filter (fuzzy subsequence), in source order.
    pub fn matches(&self) -> Vec<&String> {
        if self.filter.is_empty() {
            return self.all.iter().collect();
        }
        self.all.iter().filter(|r| fuzzy_match(&self.filter, r)).collect()
    }
}

/// Case-insensitive subsequence match: every non-space char of `needle` appears
/// in `haystack` in order. ponytail: no scoring/ranking, just a filter.
fn fuzzy_match(needle: &str, haystack: &str) -> bool {
    let mut hay = haystack.chars().flat_map(char::to_lowercase);
    'outer: for nc in needle.chars().flat_map(char::to_lowercase) {
        if nc.is_whitespace() {
            continue;
        }
        for hc in hay.by_ref() {
            if hc == nc {
                continue 'outer;
            }
        }
        return false;
    }
    true
}

/// What `n` is creating on the current tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewKind {
    Mr,
    Issue,
    Member,
    Pipeline,
    Tag,
}

/// The `n` form: one text field per label, some with a type-to-filter dropdown
/// (branches for an MR/pipeline/tag, roles for a member). Blank branch fields
/// defer to the CLI/API — the branch you're on, or the project's default branch.
pub struct NewItemForm {
    pub kind: NewKind,
    /// One value per entry in `fields()`.
    pub values: Vec<String>,
    /// Cursor position within `fields()`.
    pub field: usize,
    /// Dropdown candidates for the current form: branch names, or role names.
    pub options: Vec<String>,
    pub options_loading: bool,
    /// Highlighted entry in the open dropdown.
    pub option_sel: usize,
    pub submitting: bool,
    pub error: Option<String>,
    /// Pipeline forms fetch branches and tags concurrently — these track arrival.
    pub branches_done: bool,
    pub tags_done: bool,
}

impl NewItemForm {
    const MR_FIELDS: [&'static str; 4] = ["Source branch", "Target branch", "Title", "Description"];
    const ISSUE_FIELDS: [&'static str; 2] = ["Title", "Description"];
    const MEMBER_FIELDS: [&'static str; 2] = ["Username", "Role"];
    const PIPELINE_FIELDS: [&'static str; 1] = ["Branch / Tag"];
    const TAG_FIELDS: [&'static str; 2] = ["Tag name", "Branch"];

    pub fn fields(&self) -> &'static [&'static str] {
        match self.kind {
            NewKind::Mr => &Self::MR_FIELDS,
            NewKind::Issue => &Self::ISSUE_FIELDS,
            NewKind::Member => &Self::MEMBER_FIELDS,
            NewKind::Pipeline => &Self::PIPELINE_FIELDS,
            NewKind::Tag => &Self::TAG_FIELDS,
        }
    }

    pub fn value(&self, field: usize) -> &str {
        self.values.get(field).map(String::as_str).unwrap_or("")
    }

    fn value_mut(&mut self, field: usize) -> &mut String {
        &mut self.values[field]
    }

    /// Whether the focused field offers a dropdown (which then owns ↑/↓).
    pub fn on_option_field(&self) -> bool {
        match self.kind {
            NewKind::Mr => self.field <= 1,
            NewKind::Pipeline => self.field == 0,
            NewKind::Tag => self.field == 1,
            NewKind::Member => self.field == 1,
            NewKind::Issue => false,
        }
    }

    /// Options matching what's typed in the focused field. Empty unless a field
    /// with a dropdown has focus, so it only shows where it applies.
    pub fn matches(&self) -> Vec<&String> {
        if !self.on_option_field() {
            return Vec::new();
        }
        let typed = self.value(self.field).to_lowercase();
        self.options
            .iter()
            .filter(|o| typed.is_empty() || o.to_lowercase().contains(&typed))
            .collect()
    }

    /// The field that must not be empty, and what to call it when it is. A pipeline
    /// needs nothing: an empty branch means the project's default branch.
    fn required(&self) -> Option<(usize, &'static str)> {
        match self.kind {
            NewKind::Pipeline => None,
            NewKind::Mr => Some((2, "a title is required")),
            NewKind::Issue => Some((0, "a title is required")),
            NewKind::Member => Some((0, "a username is required")),
            NewKind::Tag => Some((0, "a tag name is required")),
        }
    }
}

/// The two stages of the startup host picker: which service, then — when the
/// CLI is logged into more than one — which instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostPick {
    /// 0 = GitLab, 1 = GitHub.
    Service(usize),
    Instance {
        kind: Kind,
        hosts: Vec<String>,
        selected: usize,
        loading: bool,
        /// Where the list came from — the CLI's config file, usually.
        origin: String,
    },
}

pub enum Pending {
    Tab(Action),
    Job(crate::backend::JobAction),
}

pub struct PendingAction {
    pub kind: Pending,
    pub id: String,
    pub label: String,
}

pub struct App {
    /// Index into `Tab::ALL` (and into `tabs`) — not a display position, so the
    /// tab bar can be reordered without touching any per-tab state.
    pub active: usize,
    pub tabs: Vec<TabState>,
    /// Display order of the tab bar: `Tab::ALL` indices, left to right.
    pub tab_order: Vec<usize>,
    /// Hidden tabs, keyed by `Tab::ALL` index — skipped by the bar and by h/l.
    pub tab_hidden: Vec<bool>,
    /// Cursor into `tab_order` while the tab manager (`T`) is open.
    pub tab_manager: Option<usize>,
    /// The startup host picker, set only when gitsmith started outside a git repo.
    pub host_picker: Option<HostPick>,
    /// The open new-MR/PR or new-issue form, if any.
    pub new_item: Option<NewItemForm>,
    /// Where preferences are persisted. `None` (the default, and what tests get)
    /// means nothing is ever read or written.
    config_path: Option<std::path::PathBuf>,
    /// Whether the filter input is capturing keys (the query itself is per-tab).
    pub searching: bool,
    pub show_help: bool,
    /// Scroll offset of the `?` manual, for terminals too short to show it all.
    pub help_scroll: u16,
    pub should_quit: bool,
    pub pending_action: Option<PendingAction>,
    pub flash: Option<String>,
    pub diff: Option<DiffView>,
    pub detail: Option<DetailView>,
    pub jobs_view: Option<JobsView>,
    pub log_view: Option<LogView>,
    pub show_cmdlog: bool,
    pub repo_picker: Option<RepoPicker>,
    /// Currently-highlighted theme index while the theme picker is open.
    pub theme_picker: Option<usize>,
    theme_idx: usize,
    repos_cache: Vec<String>,
    comment_request: Option<String>,
    repo: String,
    kind: Kind,
    backend: Arc<dyn Backend>,
    tx: Sender<Msg>,
}

impl App {
    pub fn new(backend: Arc<dyn Backend>, kind: Kind, repo: String, tx: Sender<Msg>) -> App {
        let mut app = App::idle(backend, kind, repo, tx);
        app.refresh_active();
        // Prefetch the repo list in the background so `P` opens instantly.
        fetch::spawn_repos(app.backend.clone(), app.tx.clone());
        app
    }

    /// Start with no host: gitsmith is running outside a git repo, so ask which
    /// host to use, then which repo. Nothing is fetched until the user picks —
    /// the backend here is a placeholder that `set_host` replaces.
    pub fn choose_host(tx: Sender<Msg>) -> App {
        let mut app = App::idle(crate::backend::backend_for(Kind::Gh), Kind::Gh, String::new(), tx);
        app.host_picker = Some(HostPick::Service(0));
        app
    }

    /// A fully-built App that has issued no commands yet.
    fn idle(backend: Arc<dyn Backend>, kind: Kind, repo: String, tx: Sender<Msg>) -> App {
        let tabs = Tab::ALL.iter().map(|_| TabState::default()).collect();
        App {
            active: 0,
            tabs,
            tab_order: (0..Tab::ALL.len()).collect(),
            tab_hidden: vec![false; Tab::ALL.len()],
            tab_manager: None,
            host_picker: None,
            new_item: None,
            config_path: None,
            searching: false,
            show_help: false,
            help_scroll: 0,
            should_quit: false,
            pending_action: None,
            flash: None,
            diff: None,
            detail: None,
            jobs_view: None,
            log_view: None,
            show_cmdlog: false,
            repo_picker: None,
            theme_picker: None,
            theme_idx: 0,
            repos_cache: Vec::new(),
            comment_request: None,
            repo,
            kind,
            backend,
            tx,
        }
    }

    /// The backend for the chosen host — read fresh, since the host picker can
    /// replace it after startup.
    pub fn backend(&self) -> Arc<dyn Backend> {
        self.backend.clone()
    }

    /// Open the `n` form for whatever the current tab creates.
    fn open_new_item(&mut self) {
        let kind = match Tab::ALL[self.active] {
            Tab::MergeRequests => NewKind::Mr,
            Tab::Issues => NewKind::Issue,
            Tab::Members => NewKind::Member,
            Tab::Pipelines => NewKind::Pipeline,
            Tab::Tags => NewKind::Tag,
            _ => {
                self.flash = Some("n: new MR/PR, issue, member, pipeline, or tag — on those tabs".into());
                return;
            }
        };
        let (values, field, options, loading) = match kind {
            NewKind::Mr => (
                // Prefilled with the checkout, but the cursor starts here: which
                // branch you're merging is the first thing to get right, and the
                // dropdown is open on it from the first keystroke.
                vec![crate::backend::current_branch().unwrap_or_default(), String::new(), String::new(), String::new()],
                0,
                Vec::new(),
                true,
            ),
            NewKind::Issue => (vec![String::new(), String::new()], 0, Vec::new(), false),
            NewKind::Member => {
                let roles = crate::backend::member_roles(self.kind);
                (vec![String::new(), String::new()], 0, roles.iter().map(|r| (*r).to_string()).collect(), false)
            }
            // An empty branch means the project's default branch.
            NewKind::Pipeline => (vec![String::new()], 0, Vec::new(), true),
            NewKind::Tag => (vec![String::new(), String::new()], 0, Vec::new(), true),
        };
        self.new_item = Some(NewItemForm {
            kind,
            values,
            field,
            options,
            options_loading: loading,
            option_sel: 0,
            submitting: false,
            error: None,
            branches_done: false,
            tags_done: false,
        });
        if loading {
            fetch::spawn_branches(self.backend.clone(), self.tx.clone());
            if kind == NewKind::Pipeline {
                fetch::spawn_tags(self.backend.clone(), self.tx.clone());
            }
        }
    }

    fn submit_new_item(&mut self) {
        let Some(form) = self.new_item.as_mut() else { return };
        if let Some((required, message)) = form.required()
            && form.value(required).trim().is_empty()
        {
            form.error = Some(message.into());
            form.field = required;
            return;
        }
        form.submitting = true;
        form.error = None;
        let (kind, values) = (form.kind, form.values.clone());
        match kind {
            NewKind::Mr => {
                let mr = crate::backend::NewMr {
                    source: values[0].clone(),
                    target: values[1].clone(),
                    title: values[2].clone(),
                    description: values[3].clone(),
                };
                fetch::spawn_create_mr(self.backend.clone(), mr, self.tx.clone());
            }
            NewKind::Issue => fetch::spawn_create_issue(
                self.backend.clone(),
                values[0].clone(),
                values[1].clone(),
                self.tx.clone(),
            ),
            NewKind::Member => {
                // An unset role means the weakest one — never a surprise grant.
                let role = if values[1].trim().is_empty() {
                    crate::backend::member_roles(self.kind)[0].to_string()
                } else {
                    values[1].clone()
                };
                fetch::spawn_add_member(self.backend.clone(), values[0].clone(), role, self.tx.clone());
            }
            NewKind::Pipeline => {
                fetch::spawn_create_pipeline(self.backend.clone(), values[0].clone(), self.tx.clone());
            }
            NewKind::Tag => {
                fetch::spawn_create_tag(self.backend.clone(), values[1].clone(), values[0].clone(), self.tx.clone());
            }
        }
    }

    fn handle_new_item_key(&mut self, key: KeyEvent) {
        let Some(form) = self.new_item.as_mut() else { return };
        // While the CLI runs, only Esc (stop watching) is accepted.
        if form.submitting {
            if key.code == KeyCode::Esc {
                self.new_item = None;
            }
            return;
        }
        let last = form.fields().len() - 1;
        let matches = form.matches().len();
        match key.code {
            KeyCode::Esc => self.new_item = None,
            // On a field with a dropdown, Enter takes the highlighted option and
            // moves on — unless it's already the field's value, in which case
            // there's nothing to accept and Enter submits as usual.
            KeyCode::Enter
                if matches > 0
                    && form.matches().get(form.option_sel).map(|o| o.as_str())
                        != Some(form.value(form.field)) =>
            {
                let picked = form.matches().get(form.option_sel).map(|o| (*o).clone());
                if let Some(o) = picked {
                    let field = form.field;
                    *form.value_mut(field) = o;
                    form.field = (field + 1).min(last);
                    form.option_sel = 0;
                }
            }
            KeyCode::Enter => self.submit_new_item(),
            KeyCode::Tab => {
                form.field = if form.field == last { 0 } else { form.field + 1 };
                form.option_sel = 0;
            }
            KeyCode::BackTab => {
                form.field = if form.field == 0 { last } else { form.field - 1 };
                form.option_sel = 0;
            }
            // ↑/↓ walk the dropdown when there is one, otherwise the fields.
            KeyCode::Down if matches > 0 => form.option_sel = (form.option_sel + 1).min(matches - 1),
            KeyCode::Up if matches > 0 => form.option_sel = form.option_sel.saturating_sub(1),
            KeyCode::Down => form.field = if form.field == last { 0 } else { form.field + 1 },
            KeyCode::Up => form.field = if form.field == 0 { last } else { form.field - 1 },
            KeyCode::Backspace => {
                let field = form.field;
                form.value_mut(field).pop();
                form.option_sel = 0;
            }
            // Ctrl-<key> is a command, not text.
            KeyCode::Char(_) if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            KeyCode::Char(c) => {
                let field = form.field;
                form.value_mut(field).push(c);
                form.option_sel = 0;
            }
            _ => {}
        }
    }

    /// Stage one: a service was chosen. Which *instance* of it isn't knowable
    /// yet — `auth status` shells out — so show a spinner and ask off-thread.
    fn choose_service(&mut self, kind: Kind) {
        self.host_picker =
            Some(HostPick::Instance { kind, hosts: Vec::new(), selected: 0, loading: true, origin: String::new() });
        fetch::spawn_hosts(kind, self.tx.clone());
    }

    /// Stage two: commit host + service, then go straight to the repo list —
    /// the only way to get anywhere without a local remote.
    ///
    /// Everything already loaded belongs to the *previous* host, so it all goes:
    /// the repo, its rows, any open overlay, and the prefetched repo list.
    fn set_host(&mut self, kind: Kind, host: Option<String>) {
        self.kind = kind;
        self.backend = crate::backend::backend_for(kind);
        crate::backend::set_host_override(host);
        crate::backend::set_repo_override(None);
        self.repo.clear();
        self.repos_cache.clear();
        self.tabs = Tab::ALL.iter().map(|_| TabState::default()).collect();
        self.diff = None;
        self.detail = None;
        self.jobs_view = None;
        self.log_view = None;
        self.host_picker = None;
        self.open_repo_picker();
    }

    /// The startup host picker: `1`/`2` or j/k+Enter for the service, then the
    /// same keys over the instance list when the CLI has several logins.
    fn handle_host_key(&mut self, key: KeyEvent) {
        // At startup there's nothing behind this overlay, so leaving means
        // quitting; reopened later with `S`, it just closes.
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            if self.has_repo() {
                self.host_picker = None;
            } else {
                self.should_quit = true;
            }
            return;
        }
        match self.host_picker.as_mut() {
            Some(HostPick::Service(sel)) => {
                let sel = *sel;
                match key.code {
                    KeyCode::Char('1') => self.choose_service(Kind::Glab),
                    KeyCode::Char('2') => self.choose_service(Kind::Gh),
                    KeyCode::Enter => self.choose_service(if sel == 0 { Kind::Glab } else { Kind::Gh }),
                    KeyCode::Down | KeyCode::Char('j') => self.host_picker = Some(HostPick::Service(1.min(sel + 1))),
                    KeyCode::Up | KeyCode::Char('k') => self.host_picker = Some(HostPick::Service(sel.saturating_sub(1))),
                    _ => {}
                }
            }
            Some(HostPick::Instance { kind, hosts, selected, loading, .. }) => {
                if *loading {
                    return; // still asking the CLI which hosts it knows
                }
                let (kind, last) = (*kind, hosts.len().saturating_sub(1));
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => *selected = last.min(*selected + 1),
                    KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                    KeyCode::Enter => {
                        let host = hosts.get(*selected).cloned();
                        if host.is_some() {
                            self.set_host(kind, host);
                        }
                    }
                    // Back to the service list.
                    KeyCode::Backspace | KeyCode::Left => self.host_picker = Some(HostPick::Service(0)),
                    _ => {}
                }
            }
            None => {}
        }
    }

    /// Product name shown in the header. The backend (GitLab/GitHub) is conveyed
    /// by the repo name beside it, so the brand stays constant.
    pub fn brand(&self) -> &'static str {
        let _ = self.kind;
        "gitsmith"
    }

    /// Whether the active host is GitLab — the UI says "MR" there and "PR" on GitHub.
    pub fn is_gitlab(&self) -> bool {
        self.kind == Kind::Glab
    }

    /// Whether a repo has been resolved yet (false until the picker runs when
    /// gitsmith starts outside a git repo).
    pub fn has_repo(&self) -> bool {
        !self.repo.is_empty()
    }

    /// The current repo name for the header (falls back when there's no remote).
    pub fn repo_label(&self) -> &str {
        if self.repo.is_empty() { "(no remote)" } else { &self.repo }
    }

    fn active_tab(&self) -> Tab {
        Tab::ALL[self.active]
    }

    pub fn refresh_active(&mut self) {
        let tab = self.active_tab();
        let ts = &mut self.tabs[self.active];
        ts.loading = true;
        ts.error = None;
        ts.pages = 0; // a refresh restarts at page 1
        fetch::spawn(self.backend.clone(), tab, 1, self.tx.clone());
    }

    /// Point every command at `repo` on the current host and reload. The picker
    /// only lists same-host repos, so the backend/kind stay the same.
    fn switch_to_repo(&mut self, repo: String) {
        crate::backend::set_repo_override(Some(repo.clone()));
        self.repo = repo;
        self.tabs = Tab::ALL.iter().map(|_| TabState::default()).collect();
        self.diff = None;
        self.detail = None;
        self.jobs_view = None;
        self.log_view = None;
        // Back to the leftmost tab — the layout survives the switch, so honor it.
        self.active = self.visible_tabs().first().copied().unwrap_or(0);
        self.flash = Some(format!("switched to {}", self.repo_label()));
        self.refresh_active();
    }

    fn open_repo_picker(&mut self) {
        // Open instantly from the prefetched cache; only fetch if it's still empty.
        let cached = self.repos_cache.clone();
        let loading = cached.is_empty();
        self.repo_picker = Some(RepoPicker {
            all: cached,
            filter: String::new(),
            selected: 0,
            loading,
            error: None,
        });
        if loading {
            fetch::spawn_repos(self.backend.clone(), self.tx.clone());
        }
    }

    /// Adopt the saved theme and tab layout, and persist later changes to `path`.
    /// Called once at startup; without it the app runs on defaults and saves nothing.
    pub fn load_config(&mut self, path: std::path::PathBuf) {
        let cfg = crate::config::load(&path);
        self.config_path = Some(path);
        self.theme_idx = cfg.theme;
        crate::ui::set_theme(cfg.theme);
        self.tab_order = cfg.tab_order;
        self.tab_hidden = cfg.tab_hidden;
        // The saved layout may hide whatever tab we opened on.
        if self.tab_hidden[self.active] {
            self.active = self.visible_tabs().first().copied().unwrap_or(0);
            self.refresh_active();
        }
    }

    /// Best-effort write of the current preferences; a no-op without a config path.
    fn save_config(&self) {
        let Some(path) = &self.config_path else { return };
        let cfg = crate::config::Config {
            theme: self.theme_idx,
            tab_order: self.tab_order.clone(),
            tab_hidden: self.tab_hidden.clone(),
        };
        let _ = crate::config::save(path, &cfg);
    }

    /// The tabs the bar draws, as `Tab::ALL` indices in display order.
    pub fn visible_tabs(&self) -> Vec<usize> {
        self.tab_order.iter().copied().filter(|&t| !self.tab_hidden[t]).collect()
    }

    /// Move `delta` tabs along the *visible* bar, wrapping at both ends.
    fn step_tab(&mut self, delta: isize) {
        let vis = self.visible_tabs();
        if vis.is_empty() {
            return;
        }
        let pos = vis.iter().position(|&t| t == self.active).unwrap_or(0) as isize;
        let next = vis[(pos + delta).rem_euclid(vis.len() as isize) as usize];
        if next != self.active {
            self.active = next;
            self.refresh_active();
        }
    }

    /// Show/hide a tab. The last visible tab can't be hidden — there would be
    /// nothing left to draw — and hiding the active tab moves off it first.
    fn toggle_tab_hidden(&mut self, tab: usize) {
        if !self.tab_hidden[tab] && self.visible_tabs().len() == 1 {
            self.flash = Some("at least one tab must stay visible".into());
            return;
        }
        self.tab_hidden[tab] = !self.tab_hidden[tab];
        if self.tab_hidden[tab] && self.active == tab {
            // Prefer the next tab to the right, else the last one still visible.
            let pos = self.tab_order.iter().position(|&t| t == tab).unwrap_or(0);
            let next = self.tab_order[pos..]
                .iter()
                .chain(self.tab_order[..pos].iter().rev())
                .copied()
                .find(|&t| !self.tab_hidden[t]);
            if let Some(t) = next {
                self.active = t;
                self.refresh_active();
            }
        }
    }

    /// The `T` tab manager: reorder with J/K, show/hide with space, `r` resets.
    fn handle_tab_manager_key(&mut self, key: KeyEvent) {
        let Some(cur) = self.tab_manager else { return };
        let last = self.tab_order.len() - 1;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('T') => {
                self.tab_manager = None;
                self.save_config(); // the layout is committed on close, not on every key
            }
            // Enter also jumps to the tab under the cursor, if it's visible.
            KeyCode::Enter => {
                let tab = self.tab_order[cur];
                self.tab_manager = None;
                self.save_config();
                if !self.tab_hidden[tab] && tab != self.active {
                    self.active = tab;
                    self.refresh_active();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.tab_manager = Some((cur + 1).min(last)),
            KeyCode::Up | KeyCode::Char('k') => self.tab_manager = Some(cur.saturating_sub(1)),
            KeyCode::Char('J') => {
                if cur < last {
                    self.tab_order.swap(cur, cur + 1);
                    self.tab_manager = Some(cur + 1);
                }
            }
            KeyCode::Char('K') => {
                if cur > 0 {
                    self.tab_order.swap(cur, cur - 1);
                    self.tab_manager = Some(cur - 1);
                }
            }
            KeyCode::Char(' ') | KeyCode::Char('x') => {
                let tab = self.tab_order[cur];
                self.toggle_tab_hidden(tab);
            }
            KeyCode::Char('r') => {
                self.tab_order = (0..Tab::ALL.len()).collect();
                self.tab_hidden = vec![false; Tab::ALL.len()];
                self.flash = Some("tabs reset".into());
            }
            _ => {}
        }
    }

    fn handle_theme_key(&mut self, key: KeyEvent) {
        let Some(sel) = self.theme_picker else { return };
        let last = crate::ui::THEMES.len() - 1;
        match key.code {
            KeyCode::Esc => {
                crate::ui::set_theme(self.theme_idx); // revert live preview
                self.theme_picker = None;
            }
            KeyCode::Enter => {
                self.theme_idx = sel; // commit
                self.theme_picker = None;
                self.save_config();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let n = (sel + 1).min(last);
                self.theme_picker = Some(n);
                crate::ui::set_theme(n); // live preview
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let n = sel.saturating_sub(1);
                self.theme_picker = Some(n);
                crate::ui::set_theme(n);
            }
            _ => {}
        }
    }

    fn handle_picker_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(rp) = self.repo_picker.as_mut() else { return };
        let count = rp.matches().len();
        match key.code {
            KeyCode::Esc => self.repo_picker = None,
            KeyCode::Enter => {
                let repo = rp.matches().get(rp.selected).map(|s| (*s).clone());
                if let Some(repo) = repo {
                    self.repo_picker = None;
                    self.switch_to_repo(repo);
                }
            }
            KeyCode::Down => rp.selected = (rp.selected + 1).min(count.saturating_sub(1)),
            KeyCode::Up => rp.selected = rp.selected.saturating_sub(1),
            KeyCode::Char('n') if ctrl => rp.selected = (rp.selected + 1).min(count.saturating_sub(1)),
            KeyCode::Char('p') if ctrl => rp.selected = rp.selected.saturating_sub(1),
            KeyCode::Backspace => {
                rp.filter.pop();
                rp.selected = 0;
            }
            // Ctrl-<key> is a command, not text — without this, Ctrl-d types "d".
            KeyCode::Char(_) if ctrl => {}
            KeyCode::Char(c) => {
                rp.filter.push(c);
                rp.selected = 0;
            }
            _ => {}
        }
    }

    /// The active tab's filter query, if any.
    pub fn search(&self) -> Option<&String> {
        self.tabs[self.active].filter.as_ref()
    }

    pub fn visible_rows(&self) -> Vec<&Row> {
        let rows = &self.tabs[self.active].rows;
        match &self.tabs[self.active].filter {
            None => rows.iter().collect(),
            Some(q) => {
                // ponytail: case-insensitive substring; swap for fuzzy-matcher for ranked matches.
                let q = q.to_lowercase();
                rows.iter()
                    .filter(|r| r.cells.iter().any(|c| c.to_lowercase().contains(&q)))
                    .collect()
            }
        }
    }

    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Fetched { tab, page, result } => {
                let i = Self::index_of(tab);
                let ts = &mut self.tabs[i];
                match result {
                    Ok(rows) => {
                        ts.error = None;
                        let full = rows.len() as u32 >= crate::backend::PER_PAGE;
                        if page <= 1 {
                            ts.rows = rows;
                            ts.table_state.select(if ts.rows.is_empty() { None } else { Some(0) });
                        } else {
                            ts.rows.extend(rows);
                        }
                        ts.pages = ts.pages.max(page);
                        // A full page means there's probably more; keep pulling in
                        // the background so the list fills in while it's readable.
                        let more = full && page < crate::backend::MAX_PAGES;
                        ts.loading = more;
                        if more {
                            fetch::spawn(self.backend.clone(), tab, page + 1, self.tx.clone());
                        } else {
                            crate::backend::sort_rows(tab, &mut self.tabs[i].rows);
                        }
                    }
                    Err(e) => {
                        ts.loading = false;
                        // A failed later page keeps whatever already arrived.
                        if page <= 1 {
                            ts.error = Some(e);
                        }
                    }
                }
            }
            Msg::Acted { tab, result } => {
                let i = Self::index_of(tab);
                self.tabs[i].loading = false;
                match result {
                    Ok(()) => {
                        self.tabs[i].error = None;
                        if i == self.active {
                            self.refresh_active();
                        }
                    }
                    Err(e) => self.tabs[i].error = Some(e),
                }
            }
            Msg::Diff { result } => {
                if let Some(dv) = self.diff.as_mut() {
                    dv.loading = false;
                    match result {
                        Ok(text) => {
                            dv.error = None;
                            dv.parsed = crate::diff::parse(&text);
                            dv.scroll = 0;
                        }
                        Err(e) => dv.error = Some(e),
                    }
                }
            }
            Msg::Comments { result } => {
                let kind = self.kind;
                if let Some(dv) = self.detail.as_mut() {
                    dv.comments_loading = false;
                    match result {
                        Ok(json) => {
                            dv.comments_error = None;
                            dv.comments = crate::detail::parse_comments(kind, &json, chrono::Utc::now());
                        }
                        Err(e) => dv.comments_error = Some(e),
                    }
                }
            }
            Msg::Jobs { result } => {
                let kind = self.kind;
                if let Some(jv) = self.jobs_view.as_mut() {
                    jv.loading = false;
                    match result {
                        Ok(json) => {
                            jv.error = None;
                            jv.jobs = crate::jobs::parse(kind, &json);
                            if jv.selected >= jv.jobs.len() {
                                jv.selected = jv.jobs.len().saturating_sub(1);
                            }
                        }
                        Err(e) => jv.error = Some(e),
                    }
                }
            }
            Msg::JobLog { result } => {
                if let Some(lv) = self.log_view.as_mut() {
                    lv.loading = false;
                    match result {
                        Ok(text) => {
                            lv.error = None;
                            lv.rows = crate::ansi::parse(&text);
                            // In follow mode the renderer pins to the bottom; only the
                            // first (non-follow was never set) load needs scroll reset.
                            if !lv.follow {
                                lv.scroll = lv.scroll.min(lv.rows.len().saturating_sub(1) as u16);
                            }
                        }
                        Err(e) => lv.error = Some(e),
                    }
                }
            }
            Msg::JobLogDone => {
                if let Some(lv) = self.log_view.as_mut() {
                    lv.loading = false;
                }
            }
            Msg::JobActed { result } => match result {
                Ok(()) => {
                    self.flash = Some("job action queued".into());
                    // refresh the jobs list for the pipeline still open
                    if let Some(jv) = self.jobs_view.as_mut() {
                        jv.last_fetch = std::time::Instant::now();
                        let pid = jv.pipeline_id.clone();
                        fetch::spawn_jobs(self.backend.clone(), pid, self.tx.clone());
                    }
                }
                Err(e) => self.flash = Some(format!("job action failed: {e}")),
            },
            Msg::Branches { result } => {
                if let Some(form) = self.new_item.as_mut() {
                    // A failed branch list isn't fatal — the field is still typeable.
                    let branches = result.unwrap_or_default();
                    if form.kind == NewKind::Pipeline {
                        // Pipeline forms receive both branches and tags — extend
                        // so neither message overwrites the other regardless of
                        // arrival order.
                        form.options.extend(branches);
                        form.branches_done = true;
                        if form.tags_done {
                            form.options_loading = false;
                        }
                    } else {
                        form.options = branches;
                        form.options_loading = false;
                    }
                }
            }
            Msg::Tags { result } => {
                if let Some(form) = self.new_item.as_mut() {
                    form.options.extend(result.unwrap_or_default());
                    form.tags_done = true;
                    if form.branches_done {
                        form.options_loading = false;
                    }
                }
            }
            Msg::Created { result } => match result {
                Ok(out) => {
                    let kind = self.new_item.as_ref().map(|f| f.kind);
                    self.new_item = None;
                    let line = out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
                    self.flash = Some(match (kind, line.is_empty()) {
                        // Adding a member reports in words; the rest print a URL.
                        (Some(NewKind::Member), _) => line.to_string(),
                        (_, true) => "created".to_string(),
                        (_, false) => format!("created {line}"),
                    });
                    let created_tab = match kind {
                        Some(NewKind::Issue) => Tab::Issues,
                        Some(NewKind::Member) => Tab::Members,
                        Some(NewKind::Pipeline) => Tab::Pipelines,
                        Some(NewKind::Tag) => Tab::Tags,
                        _ => Tab::MergeRequests,
                    };
                    if Tab::ALL[self.active] == created_tab {
                        self.refresh_active();
                    }
                }
                Err(e) => {
                    if let Some(form) = self.new_item.as_mut() {
                        form.submitting = false;
                        form.error = Some(e);
                    } else {
                        self.flash = Some(format!("create failed: {e}"));
                    }
                }
            },
            Msg::Hosts { kind, hosts, origin } => {
                if let Some(HostPick::Instance { hosts: slot, loading, origin: from, .. }) =
                    self.host_picker.as_mut()
                {
                    *loading = false;
                    *slot = hosts.clone();
                    *from = origin.clone();
                }
                match hosts.len() {
                    // Nothing configured: say so rather than opening an empty list.
                    0 => {
                        let cli = if kind == Kind::Glab { "glab" } else { "gh" };
                        self.flash = Some(format!("no {cli} config found — run `{cli} auth login`"));
                    }
                    // One configured host needs no prompt, but say which it is.
                    1 => {
                        let host = hosts.into_iter().next();
                        if let Some(h) = &host {
                            self.flash = Some(format!("using {h}"));
                        }
                        self.set_host(kind, host);
                    }
                    _ => {}
                }
            }
            Msg::Repos { result } => match result {
                Ok(repos) => {
                    self.repos_cache = repos.clone(); // cache for instant future opens
                    if let Some(rp) = self.repo_picker.as_mut() {
                        rp.loading = false;
                        rp.error = None;
                        rp.all = repos;
                        rp.selected = 0;
                    }
                }
                Err(e) => {
                    if let Some(rp) = self.repo_picker.as_mut() {
                        rp.loading = false;
                        rp.error = Some(e);
                    }
                }
            },
        }
    }

    fn index_of(tab: Tab) -> usize {
        Tab::ALL.iter().position(|t| *t == tab).unwrap_or(0)
    }

    /// Move the selection by `rows`, stopping at the ends rather than wrapping —
    /// a half-page jump that wrapped around would lose your place.
    fn scroll_selection(&mut self, rows: isize) {
        let len = self.visible_rows().len();
        if len == 0 {
            return;
        }
        let ts = &mut self.tabs[self.active];
        let cur = ts.table_state.selected().unwrap_or(0) as isize;
        let next = (cur + rows).clamp(0, len as isize - 1) as usize;
        ts.table_state.select(Some(next));
    }

    /// Half the visible table, at least one row. Falls back to a sensible jump
    /// before the first render has reported a height.
    fn half_page(&self) -> isize {
        let h = self.tabs[self.active].viewport_h;
        let h = if h == 0 { 20 } else { h };
        (h as isize / 2).max(1)
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.visible_rows().len();
        if len == 0 {
            return;
        }
        let ts = &mut self.tabs[self.active];
        let cur = ts.table_state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).rem_euclid(len as isize) as usize;
        ts.table_state.select(Some(next));
    }

    pub fn selected_row(&self) -> Option<&Row> {
        let i = self.tabs[self.active].table_state.selected()?;
        self.visible_rows().into_iter().nth(i)
    }

    pub fn take_comment_request(&mut self) -> Option<String> {
        self.comment_request.take()
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl-C quits from anywhere — raw mode delivers it as a key, not a signal.
        // It is the only way to quit (plain `q` closes overlays but no longer exits).
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        self.flash = None; // transient — cleared each keypress

        // 0. Confirm modal: only y/Y dispatches; anything else cancels. Checked
        //    before the overlays so an action triggered from inside a detail/diff
        //    overlay can still be confirmed (the modal renders on top of them).
        if let Some(p) = self.pending_action.take() {
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                match p.kind {
                    Pending::Tab(action) => {
                        self.tabs[self.active].loading = true;
                        self.tabs[self.active].error = None;
                        fetch::spawn_act(
                            self.backend.clone(),
                            Tab::ALL[self.active],
                            action,
                            p.id,
                            self.tx.clone(),
                        );
                    }
                    Pending::Job(action) => {
                        fetch::spawn_job_act(self.backend.clone(), action, p.id, self.tx.clone());
                    }
                }
            }
            // non-y: modal already taken (closed), nothing dispatched
            return;
        }

        // The command-log inspector (opened with `x` from the list) closes on x/q/Esc
        // and holds otherwise so it can be read.
        if self.show_cmdlog {
            if matches!(key.code, KeyCode::Char('x') | KeyCode::Char('q') | KeyCode::Esc) {
                self.show_cmdlog = false;
            }
            return;
        }

        // 1. An open overlay owns all keys — scroll/close (plus MR/job actions),
        //    nothing leaks to the list. Precedence top→bottom: a log stacks on jobs;
        //    a diff stacks on a detail. Only one stack is ever open at a time.
        if self.log_view.is_some() {
            self.handle_log_key(key);
            return;
        }
        if self.jobs_view.is_some() {
            self.handle_jobs_key(key);
            return;
        }
        if self.diff.is_some() {
            self.handle_diff_key(key);
            return;
        }
        if self.detail.is_some() {
            self.handle_detail_key(key);
            return;
        }

        // The new-item form is a text entry — it must swallow every key,
        // including the ones that are global shortcuts elsewhere (q, j/k, /).
        if self.new_item.is_some() {
            self.handle_new_item_key(key);
            return;
        }

        // Host picker: nothing else exists until it's answered, so it goes first.
        if self.host_picker.is_some() {
            self.handle_host_key(key);
            return;
        }

        // Repo picker (fuzzy-find remote repos) owns all keys while open.
        if self.repo_picker.is_some() {
            self.handle_picker_key(key);
            return;
        }

        // Theme picker owns all keys while open (with live preview).
        if self.theme_picker.is_some() {
            self.handle_theme_key(key);
            return;
        }

        // Tab manager (reorder / show / hide) owns all keys while open.
        if self.tab_manager.is_some() {
            self.handle_tab_manager_key(key);
            return;
        }

        // 2. Search typing mode (unchanged from M1).
        if self.searching {
            let ts = &mut self.tabs[self.active];
            match key.code {
                KeyCode::Esc => {
                    ts.filter = None;
                    self.searching = false;
                }
                KeyCode::Enter => self.searching = false, // commit: keep query, resume navigation
                KeyCode::Backspace => {
                    if let Some(q) = ts.filter.as_mut() {
                        q.pop();
                    }
                }
                KeyCode::Char(_) if key.modifiers.contains(KeyModifiers::CONTROL) => {}
                KeyCode::Char(c) => {
                    if let Some(q) = ts.filter.as_mut() {
                        q.push(c);
                    }
                }
                _ => {}
            }
            // The match set shifted under the cursor — start from the top.
            let top = if self.visible_rows().is_empty() { None } else { Some(0) };
            self.tabs[self.active].table_state.select(top);
            return;
        }

        // The manual scrolls with j/k; anything else dismisses it.
        if self.show_help {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => self.help_scroll = self.help_scroll.saturating_add(1),
                KeyCode::Char('k') | KeyCode::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
                _ => self.show_help = false,
            }
            return;
        }

        // 3. Global keys.
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.tabs[self.active].filter = None,
            KeyCode::Char('l') | KeyCode::Right => self.step_tab(1),
            KeyCode::Char('h') | KeyCode::Left => self.step_tab(-1),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            // Ctrl-d / Ctrl-u: half a screen, the way pagers do it. Checked
            // before the action keys so `d` still cancels a pipeline on its own.
            KeyCode::Char('d') if ctrl => self.scroll_selection(self.half_page()),
            KeyCode::Char('u') if ctrl => self.scroll_selection(-self.half_page()),
            KeyCode::Char('f') | KeyCode::Char('/') => {
                self.tabs[self.active].filter = Some(String::new());
                self.searching = true;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                self.help_scroll = 0;
            }
            KeyCode::Char('R') => self.refresh_active(),
            KeyCode::Char('P') => self.open_repo_picker(),
            KeyCode::Char('o') => self.open_selected(),
            KeyCode::Char('v') => self.open_diff(),
            KeyCode::Char('n') => self.open_new_item(),
            KeyCode::Char('S') => self.host_picker = Some(HostPick::Service(0)),
            KeyCode::Char('x') => self.show_cmdlog = true,
            KeyCode::Char('t') => self.theme_picker = Some(self.theme_idx),
            KeyCode::Char('T') => {
                let cur = self.tab_order.iter().position(|&t| t == self.active).unwrap_or(0);
                self.tab_manager = Some(cur);
            }
            KeyCode::Enter => self.open_detail(),
            KeyCode::Char(c) => self.handle_action_key(c),
            _ => {}
        }
    }

    fn handle_action_key(&mut self, c: char) {
        let tab = Tab::ALL[self.active];
        let id = self.selected_row().map(|r| r.id.clone()).filter(|s| !s.is_empty());
        // Comment: special-case (needs the terminal).
        if c == 'C' && tab == Tab::Issues {
            if let Some(id) = id {
                self.comment_request = Some(id);
            } else {
                self.flash = Some("comment: no row selected".into());
            }
            return;
        }
        match action_for(tab, c) {
            Some(action) => match id {
                Some(id) => {
                    let label = action.label(&id);
                    self.pending_action = Some(PendingAction { kind: Pending::Tab(action), id, label });
                }
                None => self.flash = Some("no row selected".into()),
            },
            None if is_action_key(c) => {
                self.flash = Some(format!("{c}: not available on {}", tab.title()));
            }
            None => {}
        }
    }

    fn open_diff(&mut self) {
        let tab = Tab::ALL[self.active];
        if tab != Tab::MergeRequests {
            self.flash = Some("diff: available on the MRs tab".into());
            return;
        }
        let Some(id) = self.selected_row().map(|r| r.id.clone()).filter(|s| !s.is_empty()) else {
            self.flash = Some("no row selected".into());
            return;
        };
        self.diff = Some(DiffView {
            title: format!("MR !{id} diff"),
            parsed: crate::diff::ParsedDiff::default(),
            scroll: 0,
            loading: true,
            error: None,
        });
        fetch::spawn_diff(self.backend.clone(), tab, id, self.tx.clone());
    }

    fn open_detail(&mut self) {
        let tab = Tab::ALL[self.active];
        if tab == Tab::Pipelines {
            return self.open_jobs();
        }
        if tab == Tab::Commits {
            return self.open_commit_diff();
        }
        let Some(row) = self.selected_row() else {
            self.flash = Some("no row selected".into());
            return;
        };
        let (raw, id) = (row.raw.clone(), row.id.clone());
        let detail = crate::detail::from_raw(self.kind, tab, &raw, chrono::Utc::now());
        let has_comments = detail.has_comments;
        self.detail = Some(DetailView {
            detail,
            comments: vec![],
            scroll: 0,
            comments_loading: has_comments,
            comments_error: None,
        });
        if has_comments && !id.is_empty() {
            fetch::spawn_comments(self.backend.clone(), tab, id, self.tx.clone());
        }
    }

    /// Approximate content length (logical lines) for scroll clamping. Wrapping
    /// may add lines, so this can slightly under-scroll very long wrapped bodies.
    fn detail_len(dv: &DetailView) -> u16 {
        let d = &dv.detail;
        let mut n = d.fields.len() + 2; // title + blank
        n += d.body.lines().count().max(1);
        if d.has_comments {
            n += 2; // divider + blank
            for c in &dv.comments {
                n += 1 + c.body.lines().count().max(1) + 1;
            }
        }
        n as u16
    }

    fn handle_detail_key(&mut self, key: KeyEvent) {
        // On an MR detail: `v` stacks the diff; a/M/c approve/merge/close (via the
        // same confirm modal as the list). Handled before borrowing the overlay.
        if Tab::ALL[self.active] == Tab::MergeRequests {
            match key.code {
                KeyCode::Char('v') => {
                    self.open_diff();
                    return;
                }
                KeyCode::Char(c @ ('a' | 'M' | 'c')) => {
                    self.handle_action_key(c);
                    return;
                }
                _ => {}
            }
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(dv) = self.detail.as_mut() else { return };
        let max = Self::detail_len(dv).saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.detail = None,
            KeyCode::Char('j') | KeyCode::Down => dv.scroll = (dv.scroll + 1).min(max),
            KeyCode::Char('k') | KeyCode::Up => dv.scroll = dv.scroll.saturating_sub(1),
            KeyCode::Char('d') if ctrl => dv.scroll = (dv.scroll + 10).min(max),
            KeyCode::Char('u') if ctrl => dv.scroll = dv.scroll.saturating_sub(10),
            KeyCode::PageDown => dv.scroll = (dv.scroll + 20).min(max),
            KeyCode::PageUp => dv.scroll = dv.scroll.saturating_sub(20),
            KeyCode::Char('g') => dv.scroll = 0,
            KeyCode::Char('G') => dv.scroll = max,
            _ => {}
        }
    }

    fn open_commit_diff(&mut self) {
        let Some(id) = self.selected_row().map(|r| r.id.clone()).filter(|s| !s.is_empty()) else {
            self.flash = Some("no commit selected".into());
            return;
        };
        let short: String = id.chars().take(8).collect();
        self.diff = Some(DiffView {
            title: format!("commit {short}"),
            parsed: crate::diff::ParsedDiff::default(),
            scroll: 0,
            loading: true,
            error: None,
        });
        fetch::spawn_commit_diff(self.backend.clone(), id, self.tx.clone());
    }

    fn open_jobs(&mut self) {
        let Some(row) = self.selected_row() else {
            self.flash = Some("no row selected".into());
            return;
        };
        let id = row.id.clone();
        if id.is_empty() {
            self.flash = Some("no pipeline selected".into());
            return;
        }
        let status = row.cells.get(1).cloned().unwrap_or_default();
        self.open_jobs_for(id.clone(), format!("Pipeline #{id} · {status}"));
    }

    fn open_jobs_for(&mut self, pipeline_id: String, title: String) {
        self.jobs_view = Some(JobsView {
            title,
            pipeline_id: pipeline_id.clone(),
            jobs: vec![],
            selected: 0,
            loading: true,
            error: None,
            last_fetch: std::time::Instant::now(),
        });
        fetch::spawn_jobs(self.backend.clone(), pipeline_id, self.tx.clone());
    }

    fn open_log(&mut self, job_id: String, job_name: String) {
        self.log_view = Some(LogView {
            title: format!("job {job_name} log"),
            job_id: job_id.clone(),
            rows: vec![],
            scroll: 0,
            follow: true,
            loading: true,
            error: None,
            last_fetch: std::time::Instant::now(),
            viewport_h: 0,
        });
        fetch::spawn_job_trace(self.backend.clone(), job_id, self.tx.clone());
    }

    /// Live-refresh open overlays: the job log tails, and the jobs list re-polls so
    /// statuses (e.g. after a retry) update on their own. Called each event loop.
    pub fn tick(&mut self) {
        self.tick_log();
        self.tick_jobs();
    }

    fn tick_log(&mut self) {
        // GitLab streams its trace (`glab ci trace`) so there's nothing to poll;
        // only GitHub's one-shot log needs re-fetching while the run is live.
        if self.kind != Kind::Gh {
            return;
        }
        let due = self.log_view.as_ref().is_some_and(|lv| {
            lv.follow
                && lv.error.is_none()
                && !lv.loading
                && lv.last_fetch.elapsed() >= std::time::Duration::from_millis(1500)
        });
        if due {
            let job_id = self.log_view.as_ref().unwrap().job_id.clone();
            if let Some(lv) = self.log_view.as_mut() {
                lv.last_fetch = std::time::Instant::now();
                lv.loading = true;
            }
            fetch::spawn_job_log(self.backend.clone(), job_id, self.tx.clone());
        }
    }

    fn tick_jobs(&mut self) {
        // Only while the jobs list is the front overlay (not under an open log).
        if self.log_view.is_some() {
            return;
        }
        let due = self.jobs_view.as_ref().is_some_and(|jv| {
            !jv.loading && jv.error.is_none() && jv.last_fetch.elapsed() >= std::time::Duration::from_millis(3000)
        });
        if due {
            let pid = self.jobs_view.as_ref().unwrap().pipeline_id.clone();
            if let Some(jv) = self.jobs_view.as_mut() {
                jv.last_fetch = std::time::Instant::now();
            }
            fetch::spawn_jobs(self.backend.clone(), pid, self.tx.clone());
        }
    }

    fn confirm_job(&mut self, action: crate::backend::JobAction, job_id: &str) {
        let label = action.label(job_id);
        self.pending_action = Some(PendingAction { kind: Pending::Job(action), id: job_id.to_string(), label });
    }

    fn handle_jobs_key(&mut self, key: KeyEvent) {
        use crate::backend::JobAction;
        // snapshot the selected job before any mutable borrow (for log/actions)
        let sel = self.jobs_view.as_ref().and_then(|jv| jv.jobs.get(jv.selected).cloned());
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.jobs_view = None,
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(jv) = self.jobs_view.as_mut()
                    && !jv.jobs.is_empty()
                {
                    jv.selected = (jv.selected + 1).min(jv.jobs.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(jv) = self.jobs_view.as_mut() {
                    jv.selected = jv.selected.saturating_sub(1);
                }
            }
            KeyCode::Enter | KeyCode::Char('l') => {
                if let Some(job) = sel {
                    match job.downstream {
                        // A bridge/trigger job → drill into its downstream pipeline.
                        Some(dp) => self.open_jobs_for(dp.clone(), format!("↳ pipeline #{dp}")),
                        None => self.open_log(job.id, job.name),
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Some(job) = sel {
                    self.confirm_job(JobAction::Retry, &job.id);
                }
            }
            KeyCode::Char('d') => {
                if let Some(job) = sel {
                    self.confirm_job(JobAction::Cancel, &job.id);
                }
            }
            KeyCode::Char('p') => {
                if let Some(job) = sel {
                    if job.status == "manual" {
                        self.confirm_job(JobAction::Play, &job.id);
                    } else {
                        self.flash = Some(format!("job #{} is {} — only manual jobs can be played", job.id, job.status));
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_log_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(lv) = self.log_view.as_mut() else { return };
        let max = lv.rows.len().saturating_sub(1) as u16;
        // Any manual scroll leaves follow mode, seeded from the current bottom view
        // so it doesn't jump. `G` re-follows (and resumes live polling).
        let bottom = (lv.rows.len() as u16).saturating_sub(lv.viewport_h);
        let base = if lv.follow { bottom } else { lv.scroll };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.log_view = None,
            KeyCode::Char('G') => {
                lv.follow = true;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                lv.follow = false;
                lv.scroll = (base + 1).min(max);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                lv.follow = false;
                lv.scroll = base.saturating_sub(1);
            }
            KeyCode::Char('d') if ctrl => {
                lv.follow = false;
                lv.scroll = (base + 10).min(max);
            }
            KeyCode::Char('u') if ctrl => {
                lv.follow = false;
                lv.scroll = base.saturating_sub(10);
            }
            KeyCode::PageDown => {
                lv.follow = false;
                lv.scroll = (base + 20).min(max);
            }
            KeyCode::PageUp => {
                lv.follow = false;
                lv.scroll = base.saturating_sub(20);
            }
            KeyCode::Char('g') => {
                lv.follow = false;
                lv.scroll = 0;
            }
            _ => {}
        }
    }

    fn handle_diff_key(&mut self, key: KeyEvent) {
        // The diff only opens on MRs, so a/M/c approve/merge/close here too.
        if let KeyCode::Char(c @ ('a' | 'M' | 'c')) = key.code {
            self.handle_action_key(c);
            return;
        }
        let Some(dv) = self.diff.as_mut() else { return };
        let max = dv.parsed.rows.len().saturating_sub(1) as u16;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // ponytail: fixed page/half-page sizes; renderer doesn't report its height back.
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.diff = None,
            KeyCode::Char('j') | KeyCode::Down => dv.scroll = (dv.scroll + 1).min(max),
            KeyCode::Char('k') | KeyCode::Up => dv.scroll = dv.scroll.saturating_sub(1),
            KeyCode::Char('d') if ctrl => dv.scroll = (dv.scroll + 10).min(max),
            KeyCode::Char('u') if ctrl => dv.scroll = dv.scroll.saturating_sub(10),
            KeyCode::PageDown => dv.scroll = (dv.scroll + 20).min(max),
            KeyCode::PageUp => dv.scroll = dv.scroll.saturating_sub(20),
            KeyCode::Char('g') => dv.scroll = 0,
            KeyCode::Char('G') => dv.scroll = max,
            // Jump between the files of the change, like clicking through the
            // web diff's file list.
            KeyCode::Char(']') => {
                let files = dv.parsed.file_positions();
                if let Some((_, at)) = files.iter().find(|(_, at)| *at as u16 > dv.scroll) {
                    dv.scroll = (*at as u16).min(max);
                }
            }
            KeyCode::Char('[') => {
                let files = dv.parsed.file_positions();
                if let Some((_, at)) = files.iter().rev().find(|(_, at)| (*at as u16) < dv.scroll) {
                    dv.scroll = *at as u16;
                }
            }
            _ => {}
        }
    }

    fn open_selected(&self) {
        if let Some(i) = self.tabs[self.active].table_state.selected()
            && let Some(row) = self.visible_rows().get(i)
            && !row.web_url.is_empty()
        {
            // ponytail: macOS `open`; add xdg-open/`start` for linux/windows later.
            let _ = std::process::Command::new("open")
                .arg(&row.web_url)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, Row, Tab};
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use std::sync::mpsc;
    use std::sync::Arc;

    struct Fake;
    impl Backend for Fake {
        fn list(&self, _t: Tab, _page: u32) -> anyhow::Result<Vec<Row>> {
            Ok(vec![])
        }
    }

    fn app() -> App {
        let (tx, _rx) = mpsc::channel();
        App::new(Arc::new(Fake), Kind::Glab, "org/repo".into(), tx)
    }

    /// The repo/host overrides are process-wide (the backends read them from any
    /// thread), so tests that assert on them have to take turns.
    fn globals() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        crate::backend::set_repo_override(None);
        crate::backend::set_host_override(None);
        guard
    }

    fn row(cells: &[&str]) -> Row {
        Row { id: String::new(), cells: cells.iter().map(|s| s.to_string()).collect(), web_url: String::new(), raw: serde_json::Value::Null }
    }

    #[test]
    fn tab_nav_wraps() {
        let mut a = app();
        assert_eq!(a.active, 0);
        a.handle_key(KeyEvent::from(KeyCode::Char('h')));
        assert_eq!(a.active, Tab::ALL.len() - 1);
        a.handle_key(KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(a.active, 0);
    }

    #[test]
    fn tab_manager_reorders_the_bar() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('T')));
        assert_eq!(a.tab_manager, Some(0));
        // move Issues down one slot: MRs is now first
        a.handle_key(KeyEvent::from(KeyCode::Char('J')));
        assert_eq!(a.tab_manager, Some(1));
        assert_eq!(a.tab_order[0], 1);
        assert_eq!(a.tab_order[1], 0);
        assert_eq!(a.visible_tabs()[0], 1);
        a.handle_key(KeyEvent::from(KeyCode::Char('q')));
        assert!(a.tab_manager.is_none());
        // h/l follow the new order, not Tab::ALL
        assert_eq!(a.active, 0);
        a.handle_key(KeyEvent::from(KeyCode::Char('h')));
        assert_eq!(a.active, 1);
    }

    #[test]
    fn hidden_tabs_are_skipped_and_restored() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('T')));
        a.handle_key(KeyEvent::from(KeyCode::Char('j'))); // cursor on MRs
        a.handle_key(KeyEvent::from(KeyCode::Char(' '))); // hide MRs
        assert!(a.tab_hidden[1]);
        assert!(!a.visible_tabs().contains(&1));
        a.handle_key(KeyEvent::from(KeyCode::Esc));
        // l from Issues now lands on Pipelines
        a.handle_key(KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(a.active, 2);
        // reset brings it back
        a.handle_key(KeyEvent::from(KeyCode::Char('T')));
        a.handle_key(KeyEvent::from(KeyCode::Char('r')));
        assert!(!a.tab_hidden[1]);
        assert_eq!(a.tab_order, (0..Tab::ALL.len()).collect::<Vec<_>>());
    }

    #[test]
    fn hiding_the_active_tab_moves_off_it() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('T')));
        a.handle_key(KeyEvent::from(KeyCode::Char(' '))); // hide Issues, the active tab
        assert!(a.tab_hidden[0]);
        assert_eq!(a.active, 1);
    }

    #[test]
    fn last_visible_tab_cannot_be_hidden() {
        let mut a = app();
        a.tab_hidden = vec![true; Tab::ALL.len()];
        a.tab_hidden[3] = false;
        a.active = 3;
        a.handle_key(KeyEvent::from(KeyCode::Char('T')));
        assert_eq!(a.tab_manager, Some(3));
        a.handle_key(KeyEvent::from(KeyCode::Char(' ')));
        assert!(!a.tab_hidden[3]);
        assert_eq!(a.visible_tabs(), vec![3]);
    }

    #[test]
    fn tab_layout_and_theme_persist_across_sessions() {
        let mut path = std::env::temp_dir();
        path.push(format!("gitsmith-app-cfg-{}.json", std::process::id()));
        std::fs::remove_file(&path).ok();

        let mut a = app();
        a.load_config(path.clone());
        // reorder + hide, then close the manager (which commits)
        a.handle_key(KeyEvent::from(KeyCode::Char('T')));
        a.handle_key(KeyEvent::from(KeyCode::Char('J')));
        a.handle_key(KeyEvent::from(KeyCode::Char('j')));
        a.handle_key(KeyEvent::from(KeyCode::Char(' ')));
        let (order, hidden) = (a.tab_order.clone(), a.tab_hidden.clone());
        assert!(hidden.iter().any(|&h| h));
        a.handle_key(KeyEvent::from(KeyCode::Esc));

        // a fresh session picks it back up
        let mut b = app();
        b.load_config(path.clone());
        assert_eq!(b.tab_order, order);
        assert_eq!(b.tab_hidden, hidden);
        assert!(!b.tab_hidden[b.active], "never opens on a hidden tab");

        // theme only commits on Enter, not on Esc
        b.handle_key(KeyEvent::from(KeyCode::Char('t')));
        b.handle_key(KeyEvent::from(KeyCode::Char('j')));
        b.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(crate::config::load(&path).theme, 1);

        std::fs::remove_file(&path).ok();
        crate::ui::set_theme(0); // reset the global for other tests
    }

    #[test]
    fn without_a_config_path_nothing_is_written() {
        let mut a = app();
        assert!(a.config_path.is_none());
        a.handle_key(KeyEvent::from(KeyCode::Char('T')));
        a.handle_key(KeyEvent::from(KeyCode::Char('J')));
        a.handle_key(KeyEvent::from(KeyCode::Esc));
        // save_config is a no-op — the reorder stayed in memory
        assert_eq!(a.tab_order[0], 1);
    }

    #[test]
    fn search_filters_case_insensitively() {
        let mut a = app();
        a.tabs[0].rows = vec![row(&["1", "Fix Bug"]), row(&["2", "Docs"])];
        a.tabs[0].filter = Some("bug".into());
        let vis = a.visible_rows();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].cells[0], "1");
    }

    #[test]
    fn ctrl_c_quits_but_plain_q_does_not() {
        use ratatui::crossterm::event::KeyModifiers;
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('q')));
        assert!(!a.should_quit); // q no longer quits
        a.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.should_quit);
    }

    #[test]
    fn apply_stores_error() {
        let mut a = app();
        a.apply(Msg::Fetched { tab: Tab::Issues, page: 1, result: Err("nope".into()) });
        assert_eq!(a.tabs[0].error.as_deref(), Some("nope"));
        assert!(!a.tabs[0].loading);
    }

    #[test]
    fn apply_acted_ok_refreshes_and_clears_error() {
        let mut a = app();
        a.tabs[0].error = Some("old".into());
        a.apply(Msg::Acted { tab: Tab::Issues, result: Ok(()) });
        assert!(a.tabs[0].error.is_none());
    }

    #[test]
    fn apply_acted_err_sets_error() {
        let mut a = app();
        a.apply(Msg::Acted { tab: Tab::Issues, result: Err("nope".into()) });
        assert_eq!(a.tabs[0].error.as_deref(), Some("nope"));
    }

    #[test]
    fn no_git_repo_starts_on_the_host_picker() {
        let (tx, _rx) = mpsc::channel();
        let a = App::choose_host(tx);
        assert_eq!(a.host_picker, Some(HostPick::Service(0)));
        assert!(!a.has_repo());
        assert!(!a.tabs[0].loading, "nothing is fetched before a host is chosen");
        assert!(a.repo_picker.is_none());
    }

    #[test]
    fn choosing_a_service_asks_the_cli_for_its_hosts() {
        let (tx, _rx) = mpsc::channel();
        let mut a = App::choose_host(tx);
        a.handle_key(KeyEvent::from(KeyCode::Down)); // GitLab -> GitHub
        assert_eq!(a.host_picker, Some(HostPick::Service(1)));
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(
            a.host_picker,
            Some(HostPick::Instance { kind: Kind::Gh, hosts: vec![], selected: 0, loading: true, origin: String::new() })
        );
        assert!(a.repo_picker.is_none(), "the repo list waits for a host");
    }

    #[test]
    fn a_single_login_skips_the_instance_list() {
        let _globals = globals();
        let (tx, _rx) = mpsc::channel();
        let mut a = App::choose_host(tx);
        a.handle_key(KeyEvent::from(KeyCode::Char('1'))); // GitLab
        a.apply(Msg::Hosts { kind: Kind::Glab, hosts: vec!["gitlab.example.com".into()], origin: "cfg".into() });
        assert!(a.host_picker.is_none());
        assert_eq!(a.flash.as_deref(), Some("using gitlab.example.com"), "says which host it picked");
        assert_eq!(a.kind, Kind::Glab);
        assert_eq!(crate::backend::host_override().as_deref(), Some("gitlab.example.com"));
        assert!(a.repo_picker.as_ref().is_some_and(|rp| rp.loading));
        crate::backend::set_host_override(None);
    }

    #[test]
    fn several_logins_are_offered_as_a_list() {
        let _globals = globals();
        let (tx, _rx) = mpsc::channel();
        let mut a = App::choose_host(tx);
        a.handle_key(KeyEvent::from(KeyCode::Char('1')));
        a.apply(Msg::Hosts {
            kind: Kind::Glab,
            hosts: vec!["gitlab.com".into(), "gitlab.selfhosted.io".into()],
            origin: "~/.config/glab-cli/config.yml".into(),
        });
        // still choosing — the self-hosted instance is the one with the repos
        assert!(a.repo_picker.is_none());
        a.handle_key(KeyEvent::from(KeyCode::Char('j')));
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(crate::backend::host_override().as_deref(), Some("gitlab.selfhosted.io"));
        assert!(a.repo_picker.is_some());
        crate::backend::set_host_override(None);
    }

    #[test]
    fn no_login_at_all_says_so_instead_of_hanging() {
        let (tx, _rx) = mpsc::channel();
        let mut a = App::choose_host(tx);
        a.handle_key(KeyEvent::from(KeyCode::Char('2'))); // GitHub
        a.apply(Msg::Hosts { kind: Kind::Gh, hosts: vec![], origin: "cfg".into() });
        assert!(a.repo_picker.is_none());
        assert_eq!(a.flash.as_deref(), Some("no gh config found — run `gh auth login`"));
    }

    #[test]
    fn instance_list_can_go_back_to_the_services() {
        let (tx, _rx) = mpsc::channel();
        let mut a = App::choose_host(tx);
        a.handle_key(KeyEvent::from(KeyCode::Char('1')));
        a.apply(Msg::Hosts { kind: Kind::Glab, hosts: vec!["a.io".into(), "b.io".into()], origin: "cfg".into() });
        a.handle_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(a.host_picker, Some(HostPick::Service(0)));
    }

    #[test]
    fn host_picker_esc_quits() {
        let (tx, _rx) = mpsc::channel();
        let mut a = App::choose_host(tx);
        a.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(a.should_quit, "there is nothing to show without a host");
    }

    #[test]
    fn picking_a_repo_starts_the_usual_workflow() {
        let _globals = globals();
        let (tx, _rx) = mpsc::channel();
        let mut a = App::choose_host(tx);
        a.handle_key(KeyEvent::from(KeyCode::Char('2'))); // GitHub
        a.apply(Msg::Hosts { kind: Kind::Gh, hosts: vec!["github.com".into()], origin: "cfg".into() });
        a.apply(Msg::Repos { result: Ok(vec!["acme/api".into(), "acme/web".into()]) });
        a.handle_key(KeyEvent::from(KeyCode::Down));
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(a.repo_picker.is_none());
        assert!(a.has_repo());
        assert_eq!(a.repo_label(), "acme/web");
        assert!(a.tabs[0].loading, "the first tab starts loading");
        assert_eq!(crate::backend::repo_override().as_deref(), Some("acme/web"));
        crate::backend::set_repo_override(None); // globals — don't leak into other tests
        crate::backend::set_host_override(None);
    }

    #[test]
    fn pages_accumulate_until_a_short_one() {
        let mut a = app();
        let full: Vec<Row> = (0..crate::backend::PER_PAGE)
            .map(|i| row(&[&i.to_string(), "x", "", "", "", ""]))
            .collect();
        a.apply(Msg::Fetched { tab: Tab::Issues, page: 1, result: Ok(full.clone()) });
        assert_eq!(a.tabs[0].rows.len(), crate::backend::PER_PAGE as usize);
        assert!(a.tabs[0].loading, "a full page means more are coming");

        // page 2 appends
        a.apply(Msg::Fetched { tab: Tab::Issues, page: 2, result: Ok(full) });
        assert_eq!(a.tabs[0].rows.len(), 2 * crate::backend::PER_PAGE as usize);
        assert!(a.tabs[0].loading);

        // a short page ends it
        a.apply(Msg::Fetched { tab: Tab::Issues, page: 3, result: Ok(vec![row(&["last", "y", "", "", "", ""])]) });
        assert_eq!(a.tabs[0].rows.len(), 2 * crate::backend::PER_PAGE as usize + 1);
        assert!(!a.tabs[0].loading);
        assert_eq!(a.tabs[0].pages, 3);
    }

    #[test]
    fn paging_stops_at_the_cap() {
        let mut a = app();
        let full: Vec<Row> = (0..crate::backend::PER_PAGE)
            .map(|i| row(&[&i.to_string(), "x", "", "", "", ""]))
            .collect();
        for page in 1..=crate::backend::MAX_PAGES {
            a.apply(Msg::Fetched { tab: Tab::Issues, page, result: Ok(full.clone()) });
        }
        assert!(!a.tabs[0].loading, "stops chasing pages at the cap");
        assert_eq!(a.tabs[0].pages, crate::backend::MAX_PAGES);
    }

    #[test]
    fn a_failed_later_page_keeps_what_arrived() {
        let mut a = app();
        let full: Vec<Row> = (0..crate::backend::PER_PAGE)
            .map(|i| row(&[&i.to_string(), "x", "", "", "", ""]))
            .collect();
        a.apply(Msg::Fetched { tab: Tab::Issues, page: 1, result: Ok(full) });
        a.apply(Msg::Fetched { tab: Tab::Issues, page: 2, result: Err("rate limited".into()) });
        assert_eq!(a.tabs[0].rows.len(), crate::backend::PER_PAGE as usize);
        assert!(a.tabs[0].error.is_none(), "page 1 is still usable, so no error banner");
        assert!(!a.tabs[0].loading);
    }

    #[test]
    fn refresh_restarts_at_page_one() {
        let mut a = app();
        a.apply(Msg::Fetched { tab: Tab::Issues, page: 1, result: Ok(vec![row(&["1", "old", "", "", "", ""])]) });
        a.tabs[0].pages = 3;
        a.handle_key(KeyEvent::from(KeyCode::Char('R')));
        assert_eq!(a.tabs[0].pages, 0);
        assert!(a.tabs[0].loading);
    }

    #[test]
    fn n_creates_the_right_thing_per_tab() {
        let mut a = app();
        // Issues: title + description only
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        let form = a.new_item.as_ref().expect("form opens on Issues");
        assert_eq!(form.kind, NewKind::Issue);
        assert_eq!(form.fields(), ["Title", "Description"]);
        assert_eq!(form.field, 0);
        a.handle_key(KeyEvent::from(KeyCode::Esc));

        // MRs/PRs: branches too, cursor on Title
        a.active = 1;
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        let form = a.new_item.as_ref().expect("form opens on MRs/PRs");
        assert_eq!(form.kind, NewKind::Mr);
        assert_eq!(form.fields().len(), 4);
        assert_eq!(form.field, 0, "the cursor starts on Source branch");
        assert!(form.value(1).is_empty(), "empty target means the repo's default branch");
        a.handle_key(KeyEvent::from(KeyCode::Esc));

        // anywhere else: nothing to create
        a.active = 6; // Branches
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        assert!(a.new_item.is_none());
        assert_eq!(a.flash.as_deref(), Some("n: new MR/PR, issue, member, pipeline, or tag — on those tabs"));
    }

    #[test]
    fn n_on_pipelines_opens_a_pipeline_form() {
        let mut a = app();
        a.active = Tab::ALL.iter().position(|t| *t == Tab::Pipelines).unwrap();
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        let form = a.new_item.as_ref().expect("form opens on Pipelines");
        assert_eq!(form.kind, NewKind::Pipeline);
        assert_eq!(form.fields(), ["Branch / Tag"]);
        assert!(form.options_loading, "branches and tags load for the dropdown");
        // nothing is required: an empty branch means the default branch
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(a.new_item.as_ref().unwrap().submitting, "an empty branch submits");
    }

    #[test]
    fn n_on_tags_opens_a_tag_form() {
        let mut a = app();
        a.active = Tab::ALL.iter().position(|t| *t == Tab::Tags).unwrap();
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        let form = a.new_item.as_ref().expect("form opens on Tags");
        assert_eq!(form.kind, NewKind::Tag);
        assert_eq!(form.fields(), ["Tag name", "Branch"]);
        // a tag name is required
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(a.new_item.as_ref().unwrap().error.as_deref(), Some("a tag name is required"));
    }

    #[test]
    fn member_form_offers_roles_and_defaults_to_the_lowest() {
        let mut a = app();
        a.active = Tab::ALL.iter().position(|t| *t == Tab::Members).unwrap();
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        let form = a.new_item.as_ref().expect("form opens on Members");
        assert_eq!(form.kind, NewKind::Member);
        assert_eq!(form.fields(), ["Username", "Role"]);
        // GitLab's ladder, weakest first — the roles come from the active host
        assert_eq!(form.options, ["Guest", "Reporter", "Developer", "Maintainer", "Owner"]);
        assert!(!form.options_loading, "roles are known without a fetch");

        // a username is required
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(a.new_item.as_ref().unwrap().error.as_deref(), Some("a username is required"));

        for c in "alice".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        // the Role field filters like the branch dropdown
        a.handle_key(KeyEvent::from(KeyCode::Tab));
        for c in "dev".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        assert_eq!(a.new_item.as_ref().unwrap().matches(), [&"Developer".to_string()]);
        a.handle_key(KeyEvent::from(KeyCode::Enter)); // accept the role
        assert_eq!(a.new_item.as_ref().unwrap().value(1), "Developer");
        assert!(!a.new_item.as_ref().unwrap().submitting, "picking a role isn't submitting");

        a.handle_key(KeyEvent::from(KeyCode::Enter)); // now submit
        assert!(a.new_item.as_ref().unwrap().submitting);
        a.apply(Msg::Created { result: Ok("alice added as Developer".into()) });
        assert_eq!(a.flash.as_deref(), Some("alice added as Developer"));
        assert!(a.tabs[Tab::ALL.iter().position(|t| *t == Tab::Members).unwrap()].loading);
    }

    #[test]
    fn member_role_left_blank_grants_the_weakest_role() {
        let mut a = app();
        a.active = Tab::ALL.iter().position(|t| *t == Tab::Members).unwrap();
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        for c in "bob".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        // submitted with no role — the form leaves the grant at Guest, not Owner
        assert!(a.new_item.as_ref().unwrap().submitting);
        assert_eq!(crate::backend::member_roles(Kind::Glab)[0], "Guest");
    }

    #[test]
    fn new_issue_submits_through_the_issue_path() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        for c in "Crash on startup".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        assert_eq!(a.new_item.as_ref().unwrap().value(0), "Crash on startup");
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(a.new_item.as_ref().unwrap().submitting);
        a.apply(Msg::Created { result: Ok("https://gitlab.com/o/r/-/issues/9".into()) });
        assert_eq!(a.flash.as_deref(), Some("created https://gitlab.com/o/r/-/issues/9"));
        assert!(a.tabs[0].loading, "the Issues list refreshes");
    }

    #[test]
    fn branch_dropdown_filters_and_picks() {
        let mut a = app();
        a.active = 1;
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        a.apply(Msg::Branches {
            result: Ok(vec!["main".into(), "feat/login".into(), "feat/logout".into(), "hotfix".into()]),
        });
        assert!(!a.new_item.as_ref().unwrap().options_loading);
        // Source has focus from the start, so its dropdown is already live
        assert_eq!(a.new_item.as_ref().unwrap().field, 0);
        assert!(!a.new_item.as_ref().unwrap().matches().is_empty());
        for _ in 0.."main".len() {
            a.handle_key(KeyEvent::from(KeyCode::Backspace)); // clear the prefilled branch
        }
        for c in "log".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let form = a.new_item.as_ref().unwrap();
        assert_eq!(form.matches(), [&"feat/login".to_string(), &"feat/logout".to_string()]);

        // ↓ walks the dropdown (not the fields), Enter takes the branch and advances
        a.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(a.new_item.as_ref().unwrap().option_sel, 1);
        assert_eq!(a.new_item.as_ref().unwrap().field, 0, "↓ stayed in the dropdown");
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        let form = a.new_item.as_ref().expect("Enter picked a branch, it didn't submit");
        assert_eq!(form.value(0), "feat/logout");
        assert_eq!(form.field, 1, "moved on to Target");
        assert!(!form.submitting);
    }

    #[test]
    fn new_item_form_edits_fields_and_swallows_global_keys() {
        let mut a = app();
        a.active = 1;
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        a.handle_key(KeyEvent::from(KeyCode::Tab));
        a.handle_key(KeyEvent::from(KeyCode::Tab)); // Source -> Target -> Title
        // `q` and `j` are global shortcuts — inside the form they're just text
        for c in "quick fix".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        assert!(!a.should_quit);
        assert_eq!(a.new_item.as_ref().unwrap().value(2), "quick fix");
        a.handle_key(KeyEvent::from(KeyCode::Backspace));
        assert_eq!(a.new_item.as_ref().unwrap().value(2), "quick fi");
        // Tab wraps around the four fields
        a.handle_key(KeyEvent::from(KeyCode::Tab));
        assert_eq!(a.new_item.as_ref().unwrap().field, 3);
        a.handle_key(KeyEvent::from(KeyCode::Tab));
        assert_eq!(a.new_item.as_ref().unwrap().field, 0);
        a.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(a.new_item.is_none());
    }

    #[test]
    fn new_mr_requires_a_title() {
        let mut a = app();
        a.active = 1;
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        let form = a.new_item.as_ref().expect("form stays open");
        assert!(!form.submitting);
        assert_eq!(form.error.as_deref(), Some("a title is required"));
    }

    #[test]
    fn new_mr_submits_and_reports_the_url() {
        let mut a = app();
        a.active = 1;
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        a.handle_key(KeyEvent::from(KeyCode::Tab));
        a.handle_key(KeyEvent::from(KeyCode::Tab)); // onto Title
        for c in "Add x".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(a.new_item.as_ref().unwrap().submitting);
        // keys are ignored while the CLI runs
        a.handle_key(KeyEvent::from(KeyCode::Char('z')));
        assert_eq!(a.new_item.as_ref().unwrap().value(2), "Add x");

        a.apply(Msg::Created { result: Ok("https://gitlab.com/o/r/-/merge_requests/12\n".into()) });
        assert!(a.new_item.is_none());
        assert_eq!(a.flash.as_deref(), Some("created https://gitlab.com/o/r/-/merge_requests/12"));
        assert!(a.tabs[1].loading, "the MR list refreshes");
    }

    #[test]
    fn new_mr_failure_stays_in_the_form() {
        let mut a = app();
        a.active = 1;
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        a.handle_key(KeyEvent::from(KeyCode::Tab));
        a.handle_key(KeyEvent::from(KeyCode::Tab)); // onto Title
        for c in "Add x".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        a.apply(Msg::Created { result: Err("source branch does not exist".into()) });
        let form = a.new_item.as_ref().expect("the form stays so the input isn't lost");
        assert!(!form.submitting);
        assert_eq!(form.error.as_deref(), Some("source branch does not exist"));
        assert_eq!(form.value(2), "Add x");
    }

    #[test]
    fn shift_s_switches_host_mid_session() {
        let _globals = globals();
        let mut a = app();
        a.tabs[0].rows = vec![row(&["1", "from the old host"])];
        a.handle_key(KeyEvent::from(KeyCode::Char('S')));
        assert_eq!(a.host_picker, Some(HostPick::Service(0)));
        // Esc backs out without quitting, because there's a repo behind it
        a.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(a.host_picker.is_none());
        assert!(!a.should_quit);

        a.handle_key(KeyEvent::from(KeyCode::Char('S')));
        a.handle_key(KeyEvent::from(KeyCode::Char('2'))); // GitHub
        a.apply(Msg::Hosts { kind: Kind::Gh, hosts: vec!["github.com".into()], origin: "cfg".into() });
        assert_eq!(a.kind, Kind::Gh);
        assert_eq!(crate::backend::host_override().as_deref(), Some("github.com"));
        // the previous host's repo and rows are gone, and we're choosing a new repo
        assert!(!a.has_repo());
        assert!(a.tabs[0].rows.is_empty());
        assert!(a.repo_picker.is_some());
        crate::backend::set_host_override(None);
        crate::backend::set_repo_override(None);
    }

    #[test]
    fn help_scrolls_then_closes() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('?')));
        assert!(a.show_help && a.help_scroll == 0);
        a.handle_key(KeyEvent::from(KeyCode::Char('j')));
        a.handle_key(KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(a.help_scroll, 2);
        a.handle_key(KeyEvent::from(KeyCode::Char('k')));
        assert_eq!(a.help_scroll, 1);
        assert!(a.show_help, "scrolling doesn't dismiss it");
        a.handle_key(KeyEvent::from(KeyCode::Char('x')));
        assert!(!a.show_help);
        assert!(!a.show_cmdlog, "the dismissing key isn't also acted on");
    }

    #[test]
    fn ctrl_keys_are_not_typed_as_text() {
        let ctrl = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);

        // repo picker
        let mut a = app();
        a.repo_picker = Some(RepoPicker {
            all: vec!["o/a".into()],
            filter: String::new(),
            selected: 0,
            loading: false,
            error: None,
        });
        a.handle_key(ctrl('d'));
        assert_eq!(a.repo_picker.as_ref().unwrap().filter, "", "Ctrl-d isn't a `d`");
        // …but Ctrl-n still moves, as it always did
        a.handle_key(ctrl('n'));
        assert_eq!(a.repo_picker.as_ref().unwrap().filter, "");

        // filter prompt
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('/')));
        a.handle_key(ctrl('u'));
        assert_eq!(a.search().map(String::as_str), Some(""));

        // new-item form
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        a.handle_key(ctrl('d'));
        assert_eq!(a.new_item.as_ref().unwrap().value(0), "");
    }

    #[test]
    fn ctrl_d_and_u_move_half_a_screen() {
        let mut a = app();
        a.tabs[0].rows = (0..50).map(|i| row(&[&i.to_string(), "x", "", "", "", ""])).collect();
        a.tabs[0].table_state.select(Some(0));
        a.tabs[0].viewport_h = 20; // as the renderer would report

        let ctrl = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        a.handle_key(ctrl('d'));
        assert_eq!(a.tabs[0].table_state.selected(), Some(10));
        a.handle_key(ctrl('d'));
        assert_eq!(a.tabs[0].table_state.selected(), Some(20));
        a.handle_key(ctrl('u'));
        assert_eq!(a.tabs[0].table_state.selected(), Some(10));

        // the ends clamp instead of wrapping
        for _ in 0..10 {
            a.handle_key(ctrl('u'));
        }
        assert_eq!(a.tabs[0].table_state.selected(), Some(0));
        for _ in 0..10 {
            a.handle_key(ctrl('d'));
        }
        assert_eq!(a.tabs[0].table_state.selected(), Some(49));
    }

    #[test]
    fn plain_d_still_acts_on_the_row() {
        // Ctrl-d scrolls, but `d` alone must still cancel a pipeline
        let mut a = app();
        a.active = 2; // Pipelines
        a.tabs[2].rows = vec![Row {
            id: "7".into(),
            cells: vec!["7".into(), "running".into(), "main".into(), "now".into()],
            web_url: String::new(),
            raw: serde_json::Value::Null,
        }];
        a.tabs[2].table_state.select(Some(0));
        a.handle_key(KeyEvent::from(KeyCode::Char('d')));
        assert!(a.pending_action.is_some(), "d opened the cancel confirm");
    }

    #[test]
    fn half_page_falls_back_before_the_first_render() {
        let mut a = app();
        a.tabs[0].rows = (0..50).map(|i| row(&[&i.to_string(), "x", "", "", "", ""])).collect();
        a.tabs[0].table_state.select(Some(0));
        assert_eq!(a.tabs[0].viewport_h, 0, "nothing has been drawn yet");
        a.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(a.tabs[0].table_state.selected(), Some(10), "sane default jump");
    }

    #[test]
    fn each_tab_keeps_its_own_filter() {
        let mut a = app();
        a.tabs[0].rows = vec![row(&["1", "Fix bug"]), row(&["2", "Docs"])];
        a.tabs[1].rows = vec![row(&["9", "Refactor"])];
        // filter Issues down to one row
        a.handle_key(KeyEvent::from(KeyCode::Char('/')));
        for c in "bug".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(a.visible_rows().len(), 1);
        // switching tabs must not carry the query over
        a.handle_key(KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(a.active, 1);
        assert!(a.search().is_none());
        assert_eq!(a.visible_rows().len(), 1);
        // …and coming back restores it
        a.handle_key(KeyEvent::from(KeyCode::Char('h')));
        assert_eq!(a.search(), Some(&"bug".to_string()));
        assert_eq!(a.visible_rows().len(), 1);
    }

    #[test]
    fn filter_matches_labels_and_resets_the_cursor() {
        let mut a = app();
        a.tabs[0].rows = vec![
            row(&["1", "Fix crash", "bug, ui", "opened", "alice", "1h ago"]),
            row(&["2", "Add docs", "docs", "opened", "bob", "2h ago"]),
        ];
        a.tabs[0].table_state.select(Some(1));
        a.handle_key(KeyEvent::from(KeyCode::Char('/')));
        for c in "ui".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let vis = a.visible_rows();
        assert_eq!(vis.len(), 1, "matched on the Labels cell");
        assert_eq!(vis[0].cells[0], "1");
        assert_eq!(a.tabs[0].table_state.selected(), Some(0), "cursor jumps back to the top");
    }

    #[test]
    fn typed_q_during_search_does_not_quit() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('/')));
        a.handle_key(KeyEvent::from(KeyCode::Char('q')));
        assert!(!a.should_quit);
        assert_eq!(a.search(), Some(&"q".to_string()));
    }

    #[test]
    fn search_commit_then_navigate() {
        let mut a = app();
        a.tabs[0].rows = vec![row(&["1", "Fix bug"]), row(&["2", "Another bug"])];
        a.handle_key(KeyEvent::from(KeyCode::Char('/')));
        for c in "bug".chars() {
            a.handle_key(KeyEvent::from(KeyCode::Char(c)));
        }
        assert!(a.searching);
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(!a.searching);
        assert_eq!(a.search(), Some(&"bug".to_string()));
        assert_eq!(a.visible_rows().len(), 2);
        a.handle_key(KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(a.tabs[0].table_state.selected(), Some(1));
    }

    #[test]
    fn esc_clears_applied_filter() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('/')));
        a.handle_key(KeyEvent::from(KeyCode::Char('x')));
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(a.search().is_some() && !a.searching);
        a.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(a.search().is_none());
    }

    fn app_with_mr() -> App {
        let mut a = app();
        a.active = 1; // MergeRequests
        a.tabs[1].rows = vec![Row { id: "12".into(), cells: vec!["12".into(), "Title".into()], web_url: String::new(), raw: serde_json::Value::Null }];
        a.tabs[1].table_state.select(Some(0));
        a
    }

    #[test]
    fn action_key_opens_confirm_without_dispatch() {
        let mut a = app_with_mr();
        a.handle_key(KeyEvent::from(KeyCode::Char('M')));
        let p = a.pending_action.as_ref().expect("modal open");
        assert!(matches!(p.kind, Pending::Tab(crate::backend::Action::MrMerge)));
        assert_eq!(p.id, "12");
        assert_eq!(p.label, "Merge MR !12?");
    }

    #[test]
    fn n_cancels_pending() {
        let mut a = app_with_mr();
        a.handle_key(KeyEvent::from(KeyCode::Char('M')));
        a.handle_key(KeyEvent::from(KeyCode::Char('n')));
        assert!(a.pending_action.is_none());
    }

    #[test]
    fn q_while_modal_open_does_not_quit() {
        let mut a = app_with_mr();
        a.handle_key(KeyEvent::from(KeyCode::Char('M')));
        a.handle_key(KeyEvent::from(KeyCode::Char('q')));
        assert!(!a.should_quit);
        assert!(a.pending_action.is_none()); // 'q' is not y/n → treated as cancel? see impl note
    }

    #[test]
    fn wrong_tab_action_key_flashes() {
        let mut a = app(); // Issues tab
        a.tabs[0].rows = vec![Row { id: "1".into(), cells: vec!["1".into()], web_url: String::new(), raw: serde_json::Value::Null }];
        a.tabs[0].table_state.select(Some(0));
        a.handle_key(KeyEvent::from(KeyCode::Char('M'))); // merge not on Issues
        assert!(a.flash.is_some());
        assert!(a.pending_action.is_none());
    }

    #[test]
    fn comment_key_sets_request() {
        let mut a = app();
        a.tabs[0].rows = vec![Row { id: "7".into(), cells: vec!["7".into()], web_url: String::new(), raw: serde_json::Value::Null }];
        a.tabs[0].table_state.select(Some(0));
        a.handle_key(KeyEvent::from(KeyCode::Char('C')));
        assert_eq!(a.take_comment_request().as_deref(), Some("7"));
        assert!(a.take_comment_request().is_none()); // taken once
    }

    #[test]
    fn apply_diff_ok_parses_into_view() {
        let mut a = app();
        a.diff = Some(DiffView { title: "t".into(), parsed: Default::default(), scroll: 5, loading: true, error: None });
        a.apply(Msg::Diff { result: Ok("@@ -1 +1 @@\n-a\n+b\n".into()) });
        let dv = a.diff.as_ref().unwrap();
        assert!(!dv.loading);
        assert_eq!(dv.scroll, 0);
        assert!(dv.parsed.rows.iter().any(|r| r.right == "b"));
    }

    #[test]
    fn apply_diff_err_sets_overlay_error() {
        let mut a = app();
        a.diff = Some(DiffView { title: "t".into(), parsed: Default::default(), scroll: 0, loading: true, error: None });
        a.apply(Msg::Diff { result: Err("nope".into()) });
        assert_eq!(a.diff.as_ref().unwrap().error.as_deref(), Some("nope"));
    }

    fn detail_view() -> DetailView {
        DetailView { detail: Default::default(), comments: vec![], scroll: 0, comments_loading: true, comments_error: None }
    }

    #[test]
    fn apply_comments_ok_fills_thread() {
        let mut a = app();
        a.detail = Some(detail_view());
        let json = r#"[{"author":{"username":"alice"},"created_at":"2026-07-31T09:00:00Z","body":"hi"}]"#;
        a.apply(Msg::Comments { result: Ok(json.into()) });
        let dv = a.detail.as_ref().unwrap();
        assert!(!dv.comments_loading);
        assert_eq!(dv.comments.len(), 1);
        assert_eq!(dv.comments[0].author, "alice");
    }

    #[test]
    fn apply_comments_err_sets_error() {
        let mut a = app();
        a.detail = Some(detail_view());
        a.apply(Msg::Comments { result: Err("boom".into()) });
        let dv = a.detail.as_ref().unwrap();
        assert!(!dv.comments_loading);
        assert_eq!(dv.comments_error.as_deref(), Some("boom"));
    }

    #[test]
    fn apply_comments_ignored_when_no_detail() {
        let mut a = app();
        a.apply(Msg::Comments { result: Ok("[]".into()) }); // must not panic
        assert!(a.detail.is_none());
    }

    fn jobs_view() -> JobsView {
        JobsView { title: "t".into(), pipeline_id: "5".into(), jobs: vec![], selected: 0, loading: true, error: None, last_fetch: std::time::Instant::now() }
    }

    fn log_view() -> LogView {
        LogView {
            title: "t".into(),
            job_id: "1".into(),
            rows: vec![],
            scroll: 3,
            follow: false,
            loading: true,
            error: None,
            last_fetch: std::time::Instant::now(),
            viewport_h: 0,
        }
    }

    #[test]
    fn apply_jobs_ok_fills_list() {
        let mut a = app();
        a.jobs_view = Some(jobs_view());
        a.apply(Msg::Jobs { result: Ok(r#"[{"id":1,"name":"build","stage":"b","status":"success"}]"#.into()) });
        let jv = a.jobs_view.as_ref().unwrap();
        assert!(!jv.loading);
        assert_eq!(jv.jobs.len(), 1);
        assert_eq!(jv.jobs[0].name, "build");
    }

    #[test]
    fn apply_joblog_ok_parses_ansi() {
        let mut a = app();
        a.log_view = Some(log_view());
        a.apply(Msg::JobLog { result: Ok("\x1b[32mok\x1b[0m\n".into()) });
        let lv = a.log_view.as_ref().unwrap();
        assert!(!lv.loading);
        assert_eq!(lv.scroll, 0);
        assert!(!lv.rows.is_empty());
    }

    #[test]
    fn apply_joblog_replaces_rows_and_done_clears_loading() {
        let mut a = app();
        a.log_view = Some(log_view());
        // the stream re-sends the whole trace each poll — later sends replace, not append
        a.apply(Msg::JobLog { result: Ok("line one\n".into()) });
        a.apply(Msg::JobLog { result: Ok("line one\nline two\n".into()) });
        assert_eq!(a.log_view.as_ref().unwrap().rows.len(), 2);
        a.apply(Msg::JobLogDone);
        assert!(!a.log_view.as_ref().unwrap().loading);
        a.log_view = None;
        a.apply(Msg::JobLogDone); // no panic without a view
    }

    #[test]
    fn apply_jobs_and_log_ignored_when_no_view() {
        let mut a = app();
        a.apply(Msg::Jobs { result: Ok("[]".into()) }); // no panic
        a.apply(Msg::JobLog { result: Ok("x".into()) });
        a.apply(Msg::JobActed { result: Err("boom".into()) });
        assert!(a.jobs_view.is_none());
        assert!(a.log_view.is_none());
        assert!(a.flash.is_some()); // JobActed err flashes
    }

    #[test]
    fn enter_opens_detail_with_comments_loading() {
        let mut a = app_with_mr();
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        let dv = a.detail.as_ref().expect("detail overlay open");
        assert!(dv.detail.has_comments); // MRs have comments
        assert!(dv.comments_loading);
    }

    #[test]
    fn enter_on_empty_tab_flashes_no_overlay() {
        let mut a = app(); // Issues, no rows selected
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(a.detail.is_none());
        assert!(a.flash.is_some());
    }

    #[test]
    fn detail_esc_closes_and_q_does_not_quit() {
        let mut a = app_with_mr();
        a.detail = Some(detail_view());
        a.handle_key(KeyEvent::from(KeyCode::Char('q'))); // q closes, does not quit
        assert!(!a.should_quit);
        assert!(a.detail.is_none());
    }

    #[test]
    fn detail_keys_do_not_leak_to_list() {
        let mut a = app_with_mr();
        a.detail = Some(detail_view());
        a.handle_key(KeyEvent::from(KeyCode::Char('l'))); // would switch tab if it leaked
        assert_eq!(a.active, 1);
        assert!(a.detail.is_some());
    }

    #[test]
    fn v_in_mr_detail_stacks_diff_then_returns() {
        let mut a = app_with_mr();
        a.detail = Some(detail_view());
        a.handle_key(KeyEvent::from(KeyCode::Char('v'))); // stack the diff
        assert!(a.diff.is_some());
        assert!(a.detail.is_some()); // detail stays underneath
        a.handle_key(KeyEvent::from(KeyCode::Esc)); // close diff → back to detail
        assert!(a.diff.is_none());
        assert!(a.detail.is_some());
    }

    #[test]
    fn approve_from_mr_detail_opens_confirm() {
        let mut a = app_with_mr();
        a.detail = Some(detail_view());
        a.handle_key(KeyEvent::from(KeyCode::Char('a')));
        let p = a.pending_action.as_ref().expect("confirm modal open");
        assert!(matches!(p.kind, Pending::Tab(crate::backend::Action::MrApprove)));
        assert_eq!(p.id, "12");
        assert!(a.detail.is_some()); // overlay stays underneath the modal
        // confirming reaches the modal (not the overlay) and dispatches
        a.handle_key(KeyEvent::from(KeyCode::Char('y')));
        assert!(a.pending_action.is_none());
    }

    #[test]
    fn merge_from_diff_opens_confirm() {
        let mut a = app_with_mr();
        a.diff = Some(DiffView { title: "t".into(), parsed: Default::default(), scroll: 0, loading: false, error: None });
        a.handle_key(KeyEvent::from(KeyCode::Char('M')));
        let p = a.pending_action.as_ref().expect("confirm modal open");
        assert!(matches!(p.kind, Pending::Tab(crate::backend::Action::MrMerge)));
        assert!(a.diff.is_some());
    }

    fn app_with_pipeline() -> App {
        let mut a = app();
        a.active = 2; // Pipelines
        a.tabs[2].rows = vec![Row { id: "5".into(), cells: vec!["5".into(), "running".into()], web_url: String::new(), raw: serde_json::Value::Null }];
        a.tabs[2].table_state.select(Some(0));
        a
    }

    fn jv_with_jobs(a: &mut App) {
        a.jobs_view = Some(JobsView {
            title: "Pipeline #5".into(),
            pipeline_id: "5".into(),
            jobs: vec![
                crate::jobs::Job { id: "11".into(), name: "build".into(), stage: "b".into(), status: "success".into(), downstream: None },
                crate::jobs::Job { id: "12".into(), name: "test".into(), stage: "t".into(), status: "running".into(), downstream: None },
            ],
            selected: 0,
            loading: false,
            error: None,
            last_fetch: std::time::Instant::now(),
        });
    }

    #[test]
    fn t_opens_theme_picker_and_commits() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('t')));
        assert_eq!(a.theme_picker, Some(0));
        a.handle_key(KeyEvent::from(KeyCode::Char('j'))); // preview next
        assert_eq!(a.theme_picker, Some(1));
        a.handle_key(KeyEvent::from(KeyCode::Enter)); // commit
        assert!(a.theme_picker.is_none());
        assert_eq!(a.theme_idx, 1);
        crate::ui::set_theme(0); // reset global for other tests
    }

    #[test]
    fn theme_picker_esc_reverts() {
        let mut a = app();
        a.theme_idx = 2;
        a.handle_key(KeyEvent::from(KeyCode::Char('t')));
        a.handle_key(KeyEvent::from(KeyCode::Char('j'))); // preview 3
        a.handle_key(KeyEvent::from(KeyCode::Esc)); // revert
        assert!(a.theme_picker.is_none());
        assert_eq!(a.theme_idx, 2); // unchanged
        crate::ui::set_theme(0);
    }

    #[test]
    fn x_toggles_cmdlog() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('x')));
        assert!(a.show_cmdlog);
        a.handle_key(KeyEvent::from(KeyCode::Char('x'))); // closes
        assert!(!a.show_cmdlog);
    }

    #[test]
    fn brand_and_repo_label() {
        let a = app(); // Kind::Glab, repo "org/repo"
        assert_eq!(a.brand(), "gitsmith");
        assert_eq!(a.repo_label(), "org/repo");
    }

    #[test]
    fn p_opens_repo_picker_and_filters() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('P')));
        assert!(a.repo_picker.is_some());
        a.handle_key(KeyEvent::from(KeyCode::Char('x'))); // types into filter, not cmdlog
        assert_eq!(a.repo_picker.as_ref().unwrap().filter, "x");
        assert!(!a.show_cmdlog);
        a.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(a.repo_picker.is_none());
    }

    #[test]
    fn fuzzy_and_picker_matches() {
        assert!(fuzzy_match("gtu", "glabtui"));
        assert!(fuzzy_match("org/repo", "org/repo"));
        assert!(!fuzzy_match("zzz", "glabtui"));
        let rp = RepoPicker {
            all: vec!["octo/hello".into(), "octo/world".into(), "acme/api".into()],
            filter: "ow".into(),
            selected: 0,
            loading: false,
            error: None,
        };
        // "ow" is a subsequence of octo/wOrld? o..w -> "octo/world" has o then w: yes
        let m = rp.matches();
        assert!(m.iter().any(|r| r.as_str() == "octo/world"));
        assert!(!m.iter().any(|r| r.as_str() == "acme/api"));
    }

    #[test]
    fn apply_repos_fills_picker() {
        let mut a = app();
        a.repo_picker = Some(RepoPicker { all: vec![], filter: String::new(), selected: 0, loading: true, error: None });
        a.apply(Msg::Repos { result: Ok(vec!["o/a".into(), "o/b".into()]) });
        let rp = a.repo_picker.as_ref().unwrap();
        assert!(!rp.loading);
        assert_eq!(rp.all.len(), 2);
    }

    #[test]
    fn prefetched_cache_opens_picker_instantly() {
        let mut a = app();
        // simulate the background prefetch landing while no picker is open
        a.apply(Msg::Repos { result: Ok(vec!["o/a".into(), "o/b".into()]) });
        assert!(a.repo_picker.is_none());
        assert_eq!(a.repos_cache.len(), 2);
        // opening now is instant (no loading, list already populated)
        a.handle_key(KeyEvent::from(KeyCode::Char('P')));
        let rp = a.repo_picker.as_ref().unwrap();
        assert!(!rp.loading);
        assert_eq!(rp.all.len(), 2);
    }

    #[test]
    fn enter_switches_repo_via_override() {
        let _globals = globals();
        let mut a = app();
        a.repo_picker = Some(RepoPicker {
            all: vec!["o/a".into(), "o/b".into()],
            filter: String::new(),
            selected: 1,
            loading: false,
            error: None,
        });
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(a.repo_picker.is_none());
        assert_eq!(a.repo_label(), "o/b");
        assert_eq!(crate::backend::repo_override().as_deref(), Some("o/b"));
    }

    #[test]
    fn enter_on_commit_opens_diff() {
        let mut a = app();
        a.active = 7; // Commits
        a.tabs[7].rows = vec![Row { id: "deadbeefcafe".into(), cells: vec!["deadbeef".into()], web_url: String::new(), raw: serde_json::Value::Null }];
        a.tabs[7].table_state.select(Some(0));
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        let dv = a.diff.as_ref().expect("commit diff overlay open");
        assert!(dv.loading);
        assert!(dv.title.contains("deadbeef"));
        assert!(a.detail.is_none());
    }

    #[test]
    fn enter_on_pipeline_opens_jobs_not_detail() {
        let mut a = app_with_pipeline();
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(a.jobs_view.is_some());
        assert!(a.detail.is_none());
        assert_eq!(a.jobs_view.as_ref().unwrap().pipeline_id, "5");
    }

    #[test]
    fn jobs_nav_clamps_and_enter_opens_log() {
        let mut a = app_with_pipeline();
        jv_with_jobs(&mut a);
        a.handle_key(KeyEvent::from(KeyCode::Char('k'))); // up at top → stays 0
        assert_eq!(a.jobs_view.as_ref().unwrap().selected, 0);
        a.handle_key(KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(a.jobs_view.as_ref().unwrap().selected, 1);
        a.handle_key(KeyEvent::from(KeyCode::Char('j'))); // clamps at last
        assert_eq!(a.jobs_view.as_ref().unwrap().selected, 1);
        a.handle_key(KeyEvent::from(KeyCode::Enter)); // open log for job 12
        assert!(a.log_view.is_some());
        assert!(a.log_view.as_ref().unwrap().title.contains("test"));
    }

    #[test]
    fn r_in_jobs_opens_job_confirm() {
        let mut a = app_with_pipeline();
        jv_with_jobs(&mut a);
        a.handle_key(KeyEvent::from(KeyCode::Char('r')));
        let p = a.pending_action.as_ref().expect("confirm open");
        assert!(matches!(p.kind, Pending::Job(crate::backend::JobAction::Retry)));
        assert_eq!(p.id, "11");
    }

    #[test]
    fn log_esc_unwinds_to_jobs_then_list() {
        let mut a = app_with_pipeline();
        jv_with_jobs(&mut a);
        a.handle_key(KeyEvent::from(KeyCode::Enter)); // log open
        assert!(a.log_view.is_some());
        a.handle_key(KeyEvent::from(KeyCode::Esc)); // close log → jobs
        assert!(a.log_view.is_none());
        assert!(a.jobs_view.is_some());
        a.handle_key(KeyEvent::from(KeyCode::Esc)); // close jobs → list
        assert!(a.jobs_view.is_none());
    }

    #[test]
    fn enter_on_bridge_job_drills_into_downstream() {
        let mut a = app_with_pipeline();
        a.jobs_view = Some(JobsView {
            title: "Pipeline #5".into(),
            pipeline_id: "5".into(),
            jobs: vec![crate::jobs::Job {
                id: "77".into(),
                name: "enduser".into(),
                stage: "trigger".into(),
                status: "success".into(),
                downstream: Some("999".into()),
            }],
            selected: 0,
            loading: false,
            error: None,
            last_fetch: std::time::Instant::now(),
        });
        a.handle_key(KeyEvent::from(KeyCode::Enter));
        assert!(a.log_view.is_none()); // no log for a bridge
        assert_eq!(a.jobs_view.as_ref().unwrap().pipeline_id, "999"); // drilled into downstream
    }

    #[test]
    fn log_opens_following_and_scroll_toggles_it() {
        let mut a = app_with_pipeline();
        jv_with_jobs(&mut a);
        a.handle_key(KeyEvent::from(KeyCode::Enter)); // open log
        assert!(a.log_view.as_ref().unwrap().follow); // follows by default
        a.handle_key(KeyEvent::from(KeyCode::Char('k'))); // scroll up → stop following
        assert!(!a.log_view.as_ref().unwrap().follow);
        a.handle_key(KeyEvent::from(KeyCode::Char('G'))); // re-follow
        assert!(a.log_view.as_ref().unwrap().follow);
    }

    #[test]
    fn jobs_keys_do_not_leak_to_list() {
        let mut a = app_with_pipeline();
        jv_with_jobs(&mut a);
        a.handle_key(KeyEvent::from(KeyCode::Char('l'))); // 'l' opens log here, not tab-switch
        assert_eq!(a.active, 2);
        assert!(a.log_view.is_some());
    }

    #[test]
    fn v_in_non_mr_detail_does_nothing() {
        let mut a = app(); // Issues tab
        a.tabs[0].rows = vec![Row { id: "1".into(), cells: vec!["1".into()], web_url: String::new(), raw: serde_json::Value::Null }];
        a.tabs[0].table_state.select(Some(0));
        a.detail = Some(detail_view());
        a.handle_key(KeyEvent::from(KeyCode::Char('v')));
        assert!(a.diff.is_none());
        assert!(a.detail.is_some());
    }

    #[test]
    fn v_on_mr_opens_loading_diff() {
        let mut a = app_with_mr();
        a.handle_key(KeyEvent::from(KeyCode::Char('v')));
        let dv = a.diff.as_ref().expect("diff overlay open");
        assert!(dv.loading);
        assert!(dv.title.contains("12"));
    }

    #[test]
    fn v_off_mr_flashes() {
        let mut a = app(); // Issues
        a.tabs[0].rows = vec![Row { id: "1".into(), cells: vec!["1".into()], web_url: String::new(), raw: serde_json::Value::Null }];
        a.tabs[0].table_state.select(Some(0));
        a.handle_key(KeyEvent::from(KeyCode::Char('v')));
        assert!(a.diff.is_none());
        assert!(a.flash.is_some());
    }

    #[test]
    fn bracket_keys_jump_between_files() {
        let mut a = app();
        let mut diff = String::new();
        for f in ["a.rs", "b.rs", "c.rs"] {
            diff.push_str(&format!("diff --git a/{f} b/{f}\n@@ -1,2 +1,2 @@\n one\n two\n"));
        }
        let parsed = crate::diff::parse(&diff);
        let at: Vec<u16> = parsed.file_positions().iter().map(|(_, i)| *i as u16).collect();
        a.diff = Some(DiffView { title: "d".into(), parsed, scroll: 0, loading: false, error: None });
        a.handle_key(KeyEvent::from(KeyCode::Char(']')));
        assert_eq!(a.diff.as_ref().unwrap().scroll, at[1]);
        a.handle_key(KeyEvent::from(KeyCode::Char(']')));
        assert_eq!(a.diff.as_ref().unwrap().scroll, at[2]);
        // past the last file it stays put rather than wrapping
        a.handle_key(KeyEvent::from(KeyCode::Char(']')));
        assert_eq!(a.diff.as_ref().unwrap().scroll, at[2]);
        a.handle_key(KeyEvent::from(KeyCode::Char('[')));
        assert_eq!(a.diff.as_ref().unwrap().scroll, at[1]);
        a.handle_key(KeyEvent::from(KeyCode::Char('[')));
        assert_eq!(a.diff.as_ref().unwrap().scroll, at[0]);
        a.handle_key(KeyEvent::from(KeyCode::Char('[')));
        assert_eq!(a.diff.as_ref().unwrap().scroll, at[0]);
    }

    #[test]
    fn diff_scroll_clamps_and_esc_closes() {
        let mut a = app_with_mr();
        a.diff = Some(DiffView {
            title: "t".into(),
            parsed: crate::diff::parse("@@ -1 +1 @@\n-a\n+b\n"),
            scroll: 0,
            loading: false,
            error: None,
        });
        a.handle_key(KeyEvent::from(KeyCode::Char('k'))); // up at top → stays 0
        assert_eq!(a.diff.as_ref().unwrap().scroll, 0);
        a.handle_key(KeyEvent::from(KeyCode::Char('G'))); // bottom
        let max = (a.diff.as_ref().unwrap().parsed.rows.len() - 1) as u16;
        assert_eq!(a.diff.as_ref().unwrap().scroll, max);
        a.handle_key(KeyEvent::from(KeyCode::Esc)); // close
        assert!(a.diff.is_none());
    }

    #[test]
    fn keys_do_not_leak_to_list_while_diff_open() {
        let mut a = app_with_mr();
        a.diff = Some(DiffView { title: "t".into(), parsed: Default::default(), scroll: 0, loading: false, error: None });
        a.handle_key(KeyEvent::from(KeyCode::Char('q'))); // q closes diff, does NOT quit
        assert!(!a.should_quit);
        assert!(a.diff.is_none());
    }
}

