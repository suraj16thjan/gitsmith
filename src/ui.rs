use crate::app::App;
use crate::backend::Tab;
use crate::diff::RowKind;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row as UiRow, Table, TableState, Tabs, Wrap};
use ratatui::Frame;

/// A UI theme: an accent color (borders/titles/active tab) and diff tints.
#[derive(Clone, Copy)]
pub struct Theme {
    pub accent: Color,
    pub add_bg: Color,
    pub del_bg: Color,
}

/// Eight popular terminal themes. Index 0 is the default.
pub const THEMES: [(&str, Theme); 8] = [
    ("Tokyo Night", Theme { accent: Color::Rgb(122, 162, 247), add_bg: Color::Rgb(32, 54, 42), del_bg: Color::Rgb(58, 34, 42) }),
    ("Gruvbox", Theme { accent: Color::Rgb(215, 153, 33), add_bg: Color::Rgb(40, 54, 30), del_bg: Color::Rgb(60, 36, 30) }),
    ("Rosé Pine", Theme { accent: Color::Rgb(196, 167, 231), add_bg: Color::Rgb(40, 52, 46), del_bg: Color::Rgb(60, 38, 46) }),
    ("Catppuccin", Theme { accent: Color::Rgb(203, 166, 247), add_bg: Color::Rgb(38, 52, 44), del_bg: Color::Rgb(58, 36, 44) }),
    ("Nord", Theme { accent: Color::Rgb(136, 192, 208), add_bg: Color::Rgb(34, 52, 48), del_bg: Color::Rgb(56, 36, 40) }),
    ("Dracula", Theme { accent: Color::Rgb(189, 147, 249), add_bg: Color::Rgb(34, 54, 40), del_bg: Color::Rgb(60, 34, 44) }),
    ("Solarized", Theme { accent: Color::Rgb(38, 139, 210), add_bg: Color::Rgb(30, 52, 40), del_bg: Color::Rgb(58, 36, 38) }),
    ("Everforest", Theme { accent: Color::Rgb(167, 192, 128), add_bg: Color::Rgb(40, 54, 36), del_bg: Color::Rgb(58, 38, 36) }),
];

fn theme_cell() -> &'static std::sync::Mutex<Theme> {
    static T: std::sync::OnceLock<std::sync::Mutex<Theme>> = std::sync::OnceLock::new();
    T.get_or_init(|| std::sync::Mutex::new(THEMES[0].1))
}

/// The active theme (single-threaded render access; cheap Copy).
pub fn theme() -> Theme {
    *theme_cell().lock().unwrap_or_else(|e| e.into_inner())
}

/// Switch the active theme by index into `THEMES` (out-of-range is ignored).
pub fn set_theme(i: usize) {
    if let Some((_, t)) = THEMES.get(i) {
        *theme_cell().lock().unwrap_or_else(|e| e.into_inner()) = *t;
    }
}

/// Highlight style for the selected row.
fn selection() -> Style {
    Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

/// A color for a known state/status word (issue/MR state, pipeline/job status);
/// `None` for anything unrecognized so ordinary cells stay default-colored.
fn state_color(s: &str) -> Option<Color> {
    Some(match s {
        "opened" | "open" | "success" | "passed" | "completed" | "active" => Color::Green,
        "closed" | "failed" | "failure" | "canceled" | "cancelled" => Color::Red,
        "merged" => Color::Magenta,
        "protected" => Color::Magenta,
        "running" | "pending" | "created" | "in_progress" | "queued" | "locked" => Color::Yellow,
        _ => return None,
    })
}

/// Clear `area`, draw a rounded bordered block titled `title`, and return the
/// inner rect for content. Used by every overlay for a consistent framed look.
fn overlay_frame(f: &mut Frame, area: Rect, title: &str, border: Color) -> Rect {
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border))
        .title(Span::styled(format!(" {title} "), Style::new().fg(border).add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    // Header: brand badge (gitsmith) + repo name.
    let header = Line::from(vec![
        Span::styled(
            format!(" {} ", app.brand()),
            Style::new().fg(Color::Black).bg(theme().accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(app.repo_label().to_string(), Style::new().fg(Color::White).add_modifier(Modifier::BOLD)),
    ]);
    f.render_widget(Paragraph::new(header), chunks[0]);

    // Tab bar
    let titles: Vec<Line> = Tab::ALL.iter().map(|t| Line::from(format!(" {} ", t.title()))).collect();
    let tabs = Tabs::new(titles)
        .select(app.active)
        .style(Style::new().fg(Color::Gray))
        .highlight_style(Style::new().fg(theme().accent).add_modifier(Modifier::BOLD))
        .divider(Span::styled("·", Style::new().fg(Color::DarkGray)));
    f.render_widget(tabs, chunks[1]);

    // Table
    let tab = Tab::ALL[app.active];
    let header = UiRow::new(
        tab.headers().iter().map(|h| Cell::from(*h)).collect::<Vec<_>>(),
    )
    .style(Style::new().fg(theme().accent).add_modifier(Modifier::BOLD));
    let visible = app.visible_rows();
    let body: Vec<UiRow> = visible
        .iter()
        .map(|r| {
            UiRow::new(
                r.cells
                    .iter()
                    .map(|c| match state_color(c) {
                        Some(col) => Cell::from(Span::styled(c.clone(), Style::new().fg(col))),
                        None => Cell::from(c.clone()),
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let ncols = tab.headers().len().max(1);
    let widths = vec![Constraint::Ratio(1, ncols as u32); ncols];
    let title = Span::styled(
        format!(" {} ({}) ", tab.title(), visible.len()),
        Style::new().fg(theme().accent).add_modifier(Modifier::BOLD),
    );
    let table = Table::new(body, widths)
        .header(header)
        .row_highlight_style(selection())
        .highlight_symbol("▍ ")
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::DarkGray))
                .title(title),
        );
    f.render_stateful_widget(table, chunks[2], &mut app.tabs[app.active].table_state);

    // Status line
    let status = status_text(app);
    f.render_widget(Paragraph::new(status).style(Style::new().fg(Color::Gray)), chunks[3]);

    if app.show_help {
        render_help(f, area);
    }

    if app.detail.is_some() {
        render_detail(f, area, app);
    }

    // Diff stacks on top of a detail (opened via `v`), so it renders after it.
    if app.diff.is_some() {
        render_diff(f, area, app);
    }

    if app.jobs_view.is_some() {
        render_jobs(f, area, app);
    }

    // Log stacks on top of the jobs list.
    if app.log_view.is_some() {
        render_log(f, area, app);
    }

    if app.show_cmdlog {
        render_cmdlog(f, area);
    }

    if app.repo_picker.is_some() {
        render_repo_picker(f, area, app);
    }

    if app.theme_picker.is_some() {
        render_theme_picker(f, area, app);
    }

    // The confirm modal renders last so it sits on top of any open overlay.
    if let Some(p) = &app.pending_action {
        render_confirm(f, area, &p.label);
    }
}

/// The `t` theme picker: a small list with a live color preview.
fn render_theme_picker(f: &mut Frame, area: Rect, app: &App) {
    let Some(sel) = app.theme_picker else { return };
    let w = 40.min(area.width);
    let h = (THEMES.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let inner = overlay_frame(f, rect, "Theme  (↑/↓ preview · Enter · Esc)", theme().accent);
    let rows: Vec<UiRow> = THEMES
        .iter()
        .enumerate()
        .map(|(i, (name, t))| {
            let cell = Cell::from(Line::from(vec![
                Span::styled("● ", Style::new().fg(t.accent)),
                Span::raw(*name),
            ]));
            let row = UiRow::new(vec![cell]);
            if i == sel { row.style(selection()) } else { row }
        })
        .collect();
    f.render_widget(Table::new(rows, [Constraint::Percentage(100)]), inner);
}

/// The `P` fuzzy repo picker: a filter line over a scrolling, selectable list.
fn render_repo_picker(f: &mut Frame, area: Rect, app: &App) {
    let rp = app.repo_picker.as_ref().unwrap();
    // A centered box, not full-frame — feels more like a palette.
    let w = 70.min(area.width);
    let h = 20.min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let title = if rp.loading {
        "Switch repo — loading…".to_string()
    } else if let Some(e) = &rp.error {
        format!("Switch repo — error: {e}")
    } else {
        "Switch repo   (type to filter · ↑/↓ · Enter · Esc)".to_string()
    };
    let inner = overlay_frame(f, rect, &title, theme().accent);
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);

    // Filter line.
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::new().fg(theme().accent)),
            Span::raw(rp.filter.clone()),
            Span::styled("▏", Style::new().fg(Color::DarkGray)),
        ])),
        chunks[0],
    );

    // Matching repos, selected row highlighted, scrolled to keep it visible.
    let matches = rp.matches();
    let body_h = chunks[1].height as usize;
    let start = rp.selected.saturating_sub(body_h.saturating_sub(1));
    let rows: Vec<UiRow> = matches
        .iter()
        .enumerate()
        .skip(start)
        .take(body_h)
        .map(|(i, name)| {
            let cell = Cell::from((*name).clone());
            let row = UiRow::new(vec![cell]);
            if i == rp.selected {
                row.style(selection())
            } else {
                row
            }
        })
        .collect();
    let table = Table::new(rows, [Constraint::Percentage(100)]);
    f.render_widget(table, chunks[1]);
}

/// The executed-command inspector (toggled with `x`): recent glab/gh commands,
/// newest last, with ✓/✗ markers.
fn render_cmdlog(f: &mut Frame, area: Rect) {
    let inner = overlay_frame(f, area, "Commands run — newest last   (x/q close)", theme().accent);
    let cmds = crate::backend::recent_cmds();
    let start = cmds.len().saturating_sub(inner.height as usize); // show the tail that fits
    let lines: Vec<Line> = cmds[start..]
        .iter()
        .map(|(line, ok)| {
            let (mark, color) = match ok {
                Some(true) => ("✓", Color::Green),
                Some(false) => ("✗", Color::Red),
                None => ("…", Color::DarkGray),
            };
            Line::from(vec![
                Span::styled(format!("{mark} "), Style::new().fg(color)),
                Span::styled("$ ", Style::new().fg(Color::DarkGray)),
                Span::raw(line.clone()),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_jobs(f: &mut Frame, area: Rect, app: &App) {
    let jv = app.jobs_view.as_ref().unwrap();
    let title = if jv.loading {
        format!("{} — loading jobs…", jv.title)
    } else if let Some(e) = &jv.error {
        format!("{} — error: {e}", jv.title)
    } else {
        format!("{}  ·  {} jobs  ·  Enter log · r retry · d cancel · q close", jv.title, jv.jobs.len())
    };
    let inner = overlay_frame(f, area, &title, theme().accent);

    if !jv.loading && jv.error.is_none() && jv.jobs.is_empty() {
        f.render_widget(
            Paragraph::new("No jobs for this pipeline.\n(press x to see the exact API call that ran)")
                .style(Style::new().fg(Color::DarkGray)),
            inner,
        );
        return;
    }

    let header = UiRow::new(vec![Cell::from("Job"), Cell::from("Stage"), Cell::from("Status")])
        .style(Style::new().fg(theme().accent).add_modifier(Modifier::BOLD));
    let rows: Vec<UiRow> = jv
        .jobs
        .iter()
        .map(|j| {
            let sc = state_color(&j.status).unwrap_or(Color::Reset);
            // Bridge/trigger jobs drill into a downstream pipeline on Enter.
            let name = if j.downstream.is_some() { format!("{} ↳", j.name) } else { j.name.clone() };
            UiRow::new(vec![
                Cell::from(name),
                Cell::from(Span::styled(j.stage.clone(), Style::new().fg(Color::DarkGray))),
                Cell::from(Span::styled(j.status.clone(), Style::new().fg(sc))),
            ])
        })
        .collect();
    let widths = [Constraint::Percentage(50), Constraint::Percentage(20), Constraint::Percentage(30)];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(selection())
        .highlight_symbol("▍ ");
    let mut ts = TableState::default();
    ts.select(Some(jv.selected));
    f.render_stateful_widget(table, inner, &mut ts);
}

fn render_log(f: &mut Frame, area: Rect, app: &mut App) {
    // Title first (immutable borrow), then record viewport height (mutable).
    let title = {
        let lv = app.log_view.as_ref().unwrap();
        if let Some(e) = &lv.error {
            format!("{} — error: {e}", lv.title)
        } else {
            let tail = if lv.follow { "● live · G to re-follow" } else { "paused · G to follow" };
            let load = if lv.loading { " ·  refreshing…" } else { "" };
            format!("{}  ·  {tail}{load} · j/k scroll · q close", lv.title)
        }
    };
    let inner = overlay_frame(f, area, &title, theme().accent);

    let lv = app.log_view.as_mut().unwrap();
    lv.viewport_h = inner.height;
    // Follow mode pins the newest lines to the bottom (like GitLab's live trace).
    let offset = if lv.follow {
        (lv.rows.len() as u16).saturating_sub(inner.height)
    } else {
        lv.scroll
    };
    let lines: Vec<Line> = lv
        .rows
        .iter()
        .map(|spans| {
            Line::from(
                spans
                    .iter()
                    .map(|(c, t)| Span::styled(t.clone(), Style::new().fg(*c)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    f.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
}

fn render_detail(f: &mut Frame, area: Rect, app: &App) {
    let dv = app.detail.as_ref().unwrap();
    let keys = if Tab::ALL[app.active] == Tab::MergeRequests {
        "j/k · v diff · a/M/c act · q close"
    } else {
        "j/k · q close"
    };
    let inner = overlay_frame(f, area, &format!("{}   ·   {keys}", dv.detail.title), theme().accent);

    let dim = Style::new().fg(Color::DarkGray);
    let mut lines: Vec<Line> = Vec::new();

    // Metadata fields.
    for (label, value) in &dv.detail.fields {
        lines.push(Line::from(vec![
            Span::styled(format!("{label}: "), dim),
            Span::raw(value.clone()),
        ]));
    }

    // Description / body.
    lines.push(Line::from(""));
    if dv.detail.body.is_empty() {
        lines.push(Line::from(Span::styled("(no description)", dim)));
    } else {
        for l in dv.detail.body.lines() {
            lines.push(Line::from(l.to_string()));
        }
    }

    // Comment thread (Issues/MRs).
    if dv.detail.has_comments {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("── Comments ({}) ──", dv.comments.len()),
            Style::new().fg(theme().accent),
        )));
        if dv.comments_loading {
            lines.push(Line::from(Span::styled("loading comments…", dim)));
        }
        if let Some(e) = &dv.comments_error {
            lines.push(Line::from(Span::styled(format!("comments: {e}"), Style::new().fg(Color::Red))));
        }
        for c in &dv.comments {
            lines.push(Line::from(Span::styled(format!("@{} · {}", c.author, c.when), dim)));
            for l in c.body.lines() {
                lines.push(Line::from(l.to_string()));
            }
            lines.push(Line::from(""));
        }
    }

    let body = Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((dv.scroll, 0));
    f.render_widget(body, inner);
}

fn render_diff(f: &mut Frame, area: Rect, app: &App) {
    let dv = app.diff.as_ref().unwrap();
    let title = if dv.loading {
        format!("{} — loading diff…", dv.title)
    } else if let Some(e) = &dv.error {
        format!("{} — error: {e}", dv.title)
    } else {
        format!("{}  ·  {} files  ·  j/k scroll · a/M/c act · q close", dv.title, dv.parsed.files)
    };
    let body = overlay_frame(f, area, &title, theme().accent);
    let rows = &dv.parsed.rows;
    if rows.is_empty() {
        return; // loading/empty is shown in the title
    }

    // Sticky file header: the file the top visible row belongs to, pinned to the
    // top of the body so the filename stays visible while scrolling within a file.
    let start = (dv.scroll as usize).min(rows.len() - 1);
    let current_file = rows[..=start].iter().rev().find(|r| r.kind == RowKind::FileHeader);
    let (sticky, body) = if current_file.is_some() {
        let c = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(body);
        (Some(c[0]), c[1])
    } else {
        (None, body)
    };
    if let (Some(sa), Some(cf)) = (sticky, current_file) {
        f.render_widget(Paragraph::new(file_bar(&cf.left)), sa);
    }

    let side_by_side = body.width >= 100; // ponytail: collapse to one column when narrow
    // Don't redraw the current file's own header in the content — it's the sticky bar.
    let content_start = if rows[start].kind == RowKind::FileHeader { start + 1 } else { start };
    let visible = rows.iter().skip(content_start).take(body.height as usize);

    let hl = crate::highlight::shared();
    let mut left_lines: Vec<Line> = Vec::new();
    let mut right_lines: Vec<Line> = Vec::new();
    let mut uni_lines: Vec<Line> = Vec::new();

    for r in visible {
        match r.kind {
            RowKind::FileHeader => {
                // A prominent full-width file bar (accent background) so the current
                // file stays identifiable as you scroll through a multi-file diff.
                let l = file_bar(&r.left);
                if side_by_side {
                    left_lines.push(l.clone());
                    right_lines.push(l);
                } else {
                    uni_lines.push(l);
                }
            }
            RowKind::HunkHeader => {
                let l = Line::from(Span::styled(r.left.clone(), Style::new().fg(theme().accent)));
                if side_by_side {
                    left_lines.push(l.clone());
                    right_lines.push(l);
                } else {
                    uni_lines.push(l);
                }
            }
            _ => {
                let (lbg, rbg) = match r.kind {
                    RowKind::Change => (Some(theme().del_bg), Some(theme().add_bg)),
                    RowKind::Removed => (Some(theme().del_bg), None),
                    RowKind::Added => (None, Some(theme().add_bg)),
                    _ => (None, None),
                };
                if side_by_side {
                    left_lines.push(hl_line(hl, &r.lang, &r.left, lbg, r.lnum_left));
                    right_lines.push(hl_line(hl, &r.lang, &r.right, rbg, r.lnum_right));
                } else {
                    // unified: removed then added; context once. Gutter shows the
                    // line's own side number (old for removed, new for added/context).
                    match r.kind {
                        RowKind::Context => uni_lines.push(hl_line(hl, &r.lang, &r.left, None, r.lnum_right)),
                        RowKind::Change => {
                            uni_lines.push(hl_line(hl, &r.lang, &format!("- {}", r.left), lbg, r.lnum_left));
                            uni_lines.push(hl_line(hl, &r.lang, &format!("+ {}", r.right), rbg, r.lnum_right));
                        }
                        RowKind::Removed => uni_lines.push(hl_line(hl, &r.lang, &format!("- {}", r.left), lbg, r.lnum_left)),
                        RowKind::Added => uni_lines.push(hl_line(hl, &r.lang, &format!("+ {}", r.right), rbg, r.lnum_right)),
                        _ => {}
                    }
                }
            }
        }
    }

    if side_by_side {
        let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(body);
        f.render_widget(Paragraph::new(left_lines), cols[0]);
        f.render_widget(Paragraph::new(right_lines), cols[1]);
    } else {
        f.render_widget(Paragraph::new(uni_lines), body);
    }
}

/// A prominent full-width file bar (`📄 path`) for the diff viewer.
fn file_bar(path: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" 📄 {path} "),
        Style::new().fg(Color::Black).bg(theme().accent).add_modifier(Modifier::BOLD),
    ))
}

/// A right-aligned line-number gutter span (dim); blank when there's no number
/// (added line on the old side, or a removed line on the new side).
fn gutter(lnum: Option<u32>) -> Span<'static> {
    let text = match lnum {
        Some(n) => format!("{n:>4} "),
        None => "     ".to_string(),
    };
    Span::styled(text, Style::new().fg(Color::DarkGray))
}

/// Build a ratatui Line: a line-number gutter followed by syntect spans, with an
/// optional background tint applied to the code (not the gutter).
fn hl_line(hl: &crate::highlight::Highlighter, lang: &str, text: &str, bg: Option<Color>, lnum: Option<u32>) -> Line<'static> {
    let mut spans = vec![gutter(lnum)];
    if text.is_empty() {
        let mut style = Style::new();
        if let Some(b) = bg {
            style = style.bg(b);
        }
        spans.push(Span::styled(String::new(), style));
    } else {
        spans.extend(hl.line(lang, text).into_iter().map(|(fg, t)| {
            let mut style = Style::new().fg(fg);
            if let Some(b) = bg {
                style = style.bg(b);
            }
            Span::styled(t, style)
        }));
    }
    Line::from(spans)
}

fn status_text(app: &App) -> String {
    let ts = &app.tabs[app.active];
    if let Some(q) = &app.search {
        format!("/{q}")
    } else if ts.loading {
        "working…".into()
    } else if let Some(f) = &app.flash {
        f.clone()
    } else if let Some(e) = &ts.error {
        format!("error: {e}")
    } else if let Some((line, ok)) = crate::backend::recent_cmds().pop() {
        let mark = match ok {
            Some(true) => "✓",
            Some(false) => "✗",
            None => "…",
        };
        format!("$ {line}  {mark}    (x cmds · ? help)")
    } else {
        "h/l tabs  j/k move  Enter open  v diff  f find  o web  P repo  t theme  R refresh  x cmds  ? help  ^C quit".into()
    }
}

fn render_help(f: &mut Frame, area: Rect) {
    let w = 44.min(area.width);
    let h = 15.min(area.height);
    let rect = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    let text = "\
  h / l   previous / next tab
  j / k   move selection
  Enter   detail / jobs
  v       diff (MRs)
  f, /    search
  o       open in browser
  P       switch repo
  t       theme picker
  R       refresh
  x       commands run
  ?       toggle this help
  Ctrl-C  quit";
    let inner = overlay_frame(f, rect, "Help", theme().accent);
    f.render_widget(Paragraph::new(text), inner);
}

fn render_confirm(f: &mut Frame, area: Rect, label: &str) {
    let w = (label.len() as u16 + 8).min(area.width).max(20);
    let h = 5.min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let inner = overlay_frame(f, rect, "Confirm", Color::Yellow);
    let text = Line::from(vec![
        Span::raw(label.to_string()),
    ]);
    let keys = Line::from(vec![
        Span::styled("[y]", Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("es  "),
        Span::styled("[n]", Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::raw("o"),
    ]);
    f.render_widget(Paragraph::new(vec![text, Line::from(""), keys]), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::backend::{Backend, Kind, Row, Tab};
    use ratatui::{backend::TestBackend, Terminal};
    use std::sync::mpsc;
    use std::sync::Arc;

    struct Fake;
    impl Backend for Fake {
        fn list(&self, _t: Tab) -> anyhow::Result<Vec<Row>> {
            Ok(vec![])
        }
    }

    #[test]
    fn renders_tab_bar_and_rows() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.tabs[0].rows = vec![Row {
            id: "7".into(),
            cells: vec!["7".into(), "Bug in parser".into(), "opened".into(), "alice".into(), "3h ago".into()],
            web_url: String::new(),
            raw: serde_json::Value::Null,
        }];
        app.tabs[0].loading = false;

        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();

        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Issues"));
        assert!(text.contains("Bug in parser"));
        // header shows the brand + repo
        assert!(text.contains("gitsmith"));
        assert!(text.contains("o/r"));
    }

    #[test]
    fn renders_confirm_modal() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.pending_action = Some(crate::app::PendingAction {
            kind: crate::app::Pending::Tab(crate::backend::Action::MrMerge),
            id: "12".into(),
            label: "Merge MR !12?".into(),
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Merge MR !12?"));
        assert!(text.contains("[y]") && text.contains("[n]"));
    }

    #[test]
    fn renders_diff_overlay() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.diff = Some(crate::app::DiffView {
            title: "MR !12 diff".into(),
            parsed: crate::diff::parse("diff --git a/src/main.rs b/src/main.rs\n@@ -1 +1 @@\n-old_line\n+new_line\n"),
            scroll: 0,
            loading: false,
            error: None,
        });
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("MR !12 diff"));
        assert!(text.contains("new_line"));
        assert!(text.contains("old_line"));
    }

    #[test]
    fn diff_sticky_file_header_stays_on_scroll() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        let mut diff = String::from("diff --git a/foo/bar.rs b/foo/bar.rs\n@@ -1,6 +1,6 @@\n");
        for i in 0..6 {
            diff.push_str(&format!(" line{i}\n"));
        }
        app.diff = Some(crate::app::DiffView {
            title: "commit x".into(),
            parsed: crate::diff::parse(&diff),
            scroll: 5, // scrolled well past the file-header row
            loading: false,
            error: None,
        });
        let mut terminal = Terminal::new(TestBackend::new(120, 10)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        // filename still shows via the sticky bar even though its row scrolled off
        assert!(text.contains("foo/bar.rs"));
    }

    #[test]
    fn diff_overlay_shows_line_numbers() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.diff = Some(crate::app::DiffView {
            title: "MR !9 diff".into(),
            parsed: crate::diff::parse("diff --git a/a.rs b/a.rs\n@@ -41,2 +41,2 @@\n ctx\n-was\n+now\n"),
            scroll: 0,
            loading: false,
            error: None,
        });
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("41"), "context line number 41 in gutter");
        assert!(text.contains("42"), "changed line number 42 in gutter");
    }

    #[test]
    fn renders_detail_overlay() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.detail = Some(crate::app::DetailView {
            detail: crate::detail::Detail {
                title: "#7 · Bug".into(),
                fields: vec![("State".into(), "opened".into())],
                body: "It broke here.".into(),
                has_comments: true,
            },
            comments: vec![crate::detail::Comment {
                author: "alice".into(),
                when: "3h ago".into(),
                body: "looking into it".into(),
            }],
            scroll: 0,
            comments_loading: false,
            comments_error: None,
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("#7 · Bug"));
        assert!(text.contains("opened"));
        assert!(text.contains("It broke here."));
        assert!(text.contains("Comments (1)"));
        assert!(text.contains("alice"));
    }

    #[test]
    fn renders_jobs_overlay() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.jobs_view = Some(crate::app::JobsView {
            title: "Pipeline #5 · running".into(),
            pipeline_id: "5".into(),
            jobs: vec![crate::jobs::Job { id: "11".into(), name: "build".into(), stage: "b".into(), status: "success".into(), downstream: None }],
            selected: 0,
            loading: false,
            error: None,
            last_fetch: std::time::Instant::now(),
        });
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Pipeline #5"));
        assert!(text.contains("build"));
        assert!(text.contains("success"));
    }

    #[test]
    fn renders_log_overlay() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.log_view = Some(crate::app::LogView {
            title: "job build log".into(),
            job_id: "1".into(),
            rows: crate::ansi::parse("\x1b[32mcompiling\x1b[0m done\n"),
            scroll: 0,
            follow: false,
            loading: false,
            error: None,
            stable_polls: 0,
            last_fetch: std::time::Instant::now(),
            viewport_h: 0,
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("job build log"));
        assert!(text.contains("compiling"));
        assert!(text.contains("done"));
    }

    fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
        buf.content().iter().map(|c| c.symbol()).collect()
    }
}
