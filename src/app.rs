use crate::backend::{action_for, is_action_key, next_index, prev_index, Action, Backend, Kind, Row, Tab};
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
    /// Consecutive polls with no new lines — used to stop tailing a finished job.
    pub stable_polls: u8,
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
    pub active: usize,
    pub tabs: Vec<TabState>,
    pub search: Option<String>,
    pub searching: bool,
    pub show_help: bool,
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
        let tabs = Tab::ALL.iter().map(|_| TabState::default()).collect();
        let mut app = App {
            active: 0,
            tabs,
            search: None,
            searching: false,
            show_help: false,
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
        };
        app.refresh_active();
        // Prefetch the repo list in the background so `P` opens instantly.
        fetch::spawn_repos(app.backend.clone(), app.tx.clone());
        app
    }

    /// Product name shown in the header. The backend (GitLab/GitHub) is conveyed
    /// by the repo name beside it, so the brand stays constant.
    pub fn brand(&self) -> &'static str {
        let _ = self.kind;
        "gitsmith"
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
        self.tabs[self.active].loading = true;
        self.tabs[self.active].error = None;
        fetch::spawn(self.backend.clone(), tab, self.tx.clone());
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
        self.active = 0;
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
            KeyCode::Char(c) => {
                rp.filter.push(c);
                rp.selected = 0;
            }
            _ => {}
        }
    }

    pub fn visible_rows(&self) -> Vec<&Row> {
        let rows = &self.tabs[self.active].rows;
        match &self.search {
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
            Msg::Fetched { tab, result } => {
                let i = Self::index_of(tab);
                let ts = &mut self.tabs[i];
                ts.loading = false;
                match result {
                    Ok(rows) => {
                        ts.error = None;
                        ts.rows = rows;
                        ts.table_state.select(if ts.rows.is_empty() { None } else { Some(0) });
                    }
                    Err(e) => ts.error = Some(e),
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
                            let rows = crate::ansi::parse(&text);
                            // track "no new lines" to stop tailing a finished job
                            if rows.len() == lv.rows.len() {
                                lv.stable_polls = lv.stable_polls.saturating_add(1);
                            } else {
                                lv.stable_polls = 0;
                            }
                            lv.rows = rows;
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

        // 2. Search typing mode (unchanged from M1).
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.search = None;
                    self.searching = false;
                }
                KeyCode::Enter => self.searching = false, // commit: keep query, resume navigation
                KeyCode::Backspace => {
                    if let Some(q) = self.search.as_mut() {
                        q.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(q) = self.search.as_mut() {
                        q.push(c);
                    }
                }
                _ => {}
            }
            return;
        }

        if self.show_help {
            self.show_help = false;
            return;
        }

        // 3. Global keys.
        match key.code {
            KeyCode::Esc => self.search = None,
            KeyCode::Char('l') | KeyCode::Right => {
                self.active = next_index(self.active);
                self.refresh_active();
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.active = prev_index(self.active);
                self.refresh_active();
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('f') | KeyCode::Char('/') => {
                self.search = Some(String::new());
                self.searching = true;
            }
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('R') => self.refresh_active(),
            KeyCode::Char('P') => self.open_repo_picker(),
            KeyCode::Char('o') => self.open_selected(),
            KeyCode::Char('v') => self.open_diff(),
            KeyCode::Char('x') => self.show_cmdlog = true,
            KeyCode::Char('t') => self.theme_picker = Some(self.theme_idx),
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
            stable_polls: 0,
            last_fetch: std::time::Instant::now(),
            viewport_h: 0,
        });
        fetch::spawn_job_log(self.backend.clone(), job_id, self.tx.clone());
    }

    /// Live-refresh open overlays: the job log tails, and the jobs list re-polls so
    /// statuses (e.g. after a retry) update on their own. Called each event loop.
    pub fn tick(&mut self) {
        self.tick_log();
        self.tick_jobs();
    }

    fn tick_log(&mut self) {
        let due = self.log_view.as_ref().is_some_and(|lv| {
            lv.follow
                && lv.error.is_none()
                && !lv.loading
                && lv.stable_polls < 4
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
                lv.stable_polls = 0;
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
        fn list(&self, _t: Tab) -> anyhow::Result<Vec<Row>> {
            Ok(vec![])
        }
    }

    fn app() -> App {
        let (tx, _rx) = mpsc::channel();
        App::new(Arc::new(Fake), Kind::Glab, "org/repo".into(), tx)
    }

    fn row(cells: &[&str]) -> Row {
        Row { id: String::new(), cells: cells.iter().map(|s| s.to_string()).collect(), web_url: String::new(), raw: serde_json::Value::Null }
    }

    #[test]
    fn tab_nav_wraps() {
        let mut a = app();
        assert_eq!(a.active, 0);
        a.handle_key(KeyEvent::from(KeyCode::Char('h')));
        assert_eq!(a.active, 9);
        a.handle_key(KeyEvent::from(KeyCode::Char('l')));
        assert_eq!(a.active, 0);
    }

    #[test]
    fn search_filters_case_insensitively() {
        let mut a = app();
        a.tabs[0].rows = vec![row(&["1", "Fix Bug"]), row(&["2", "Docs"])];
        a.search = Some("bug".into());
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
        a.apply(Msg::Fetched { tab: Tab::Issues, result: Err("nope".into()) });
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
    fn typed_q_during_search_does_not_quit() {
        let mut a = app();
        a.handle_key(KeyEvent::from(KeyCode::Char('/')));
        a.handle_key(KeyEvent::from(KeyCode::Char('q')));
        assert!(!a.should_quit);
        assert_eq!(a.search, Some("q".to_string()));
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
        assert_eq!(a.search, Some("bug".to_string()));
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
        assert!(a.search.is_some() && !a.searching);
        a.handle_key(KeyEvent::from(KeyCode::Esc));
        assert!(a.search.is_none());
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
            stable_polls: 0,
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
        crate::backend::set_repo_override(None); // reset global for other tests
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
