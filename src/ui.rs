use crate::app::{App, HostPick, NewKind};
use crate::backend::Tab;
use crate::diff::RowKind;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row as UiRow, Table, TableState, Tabs, Wrap};
use ratatui::Frame;

/// A UI theme: the surface colors (backgrounds, text, borders), an accent color
/// (titles/active tab/selection bar) and the two diff tints.
#[derive(Clone, Copy)]
pub struct Theme {
    pub accent: Color,
    /// Main canvas — the table body and everything behind it.
    pub bg: Color,
    /// Chrome: header, tab bar, status line, table header band, overlays.
    pub bg_alt: Color,
    /// Selected row.
    pub bg_sel: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_bright: Color,
    pub border: Color,
    pub add_bg: Color,
    pub del_bg: Color,
}

/// Status colors, shared by every theme so a state word always reads the same.
const GREEN: Color = rgb(0x00e676);
const RED: Color = rgb(0xff5252);
const YELLOW: Color = rgb(0xffab40);
const MAGENTA: Color = rgb(0xff4081);
const CYAN: Color = rgb(0x18ffff);

const fn rgb(h: u32) -> Color {
    Color::Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

#[allow(clippy::too_many_arguments)]
const fn t(
    accent: u32,
    bg: u32,
    bg_alt: u32,
    bg_sel: u32,
    fg: u32,
    fg_dim: u32,
    fg_bright: u32,
    border: u32,
    add_bg: u32,
    del_bg: u32,
) -> Theme {
    Theme {
        accent: rgb(accent),
        bg: rgb(bg),
        bg_alt: rgb(bg_alt),
        bg_sel: rgb(bg_sel),
        fg: rgb(fg),
        fg_dim: rgb(fg_dim),
        fg_bright: rgb(fg_bright),
        border: rgb(border),
        add_bg: rgb(add_bg),
        del_bg: rgb(del_bg),
    }
}

/// Nine terminal themes. Index 0 is the default.
//                       accent      bg        bg_alt    bg_sel    fg        fg_dim    fg_brite  border    add_bg    del_bg
pub const THEMES: [(&str, Theme); 9] = [
    ("Neon",        t(0x00e676, 0x0a0e14, 0x0d1117, 0x131a24, 0xb3b1ad, 0x5c6370, 0xe6e1cf, 0x2a3444, 0x123a28, 0x3a1b22)),
    ("Tokyo Night", t(0x7aa2f7, 0x1a1b26, 0x16161e, 0x292e42, 0xa9b1d6, 0x565f89, 0xc0caf5, 0x2f3549, 0x20362a, 0x3a222a)),
    ("Gruvbox",     t(0xd79921, 0x282828, 0x1d2021, 0x3c3836, 0xebdbb2, 0x928374, 0xfbf1c7, 0x504945, 0x28361e, 0x3c241e)),
    ("Rosé Pine",   t(0xc4a7e7, 0x191724, 0x1f1d2e, 0x26233a, 0xe0def4, 0x6e6a86, 0xf2f0ff, 0x403d52, 0x28342e, 0x3c262e)),
    ("Catppuccin",  t(0xcba6f7, 0x1e1e2e, 0x181825, 0x313244, 0xcdd6f4, 0x6c7086, 0xe6e9ef, 0x45475a, 0x26342c, 0x3a242c)),
    ("Nord",        t(0x88c0d0, 0x2e3440, 0x272c36, 0x3b4252, 0xd8dee9, 0x616e88, 0xeceff4, 0x434c5e, 0x223430, 0x382428)),
    ("Dracula",     t(0xbd93f9, 0x282a36, 0x21222c, 0x44475a, 0xf8f8f2, 0x6272a4, 0xffffff, 0x44475a, 0x223628, 0x3c222c)),
    ("Solarized",   t(0x268bd2, 0x002b36, 0x00212b, 0x073642, 0x839496, 0x586e75, 0xeee8d5, 0x0f4d5c, 0x1e3428, 0x3a2426)),
    ("Everforest",  t(0xa7c080, 0x2d353b, 0x272e33, 0x3d484d, 0xd3c6aa, 0x859289, 0xe7e2cb, 0x4f5b58, 0x283624, 0x3a2624)),
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

/// Highlight style for the selected row: a lifted background, no fg override so
/// per-cell colors survive and the accent selection bar keeps its own color.
fn selection() -> Style {
    Style::new().bg(theme().bg_sel).add_modifier(Modifier::BOLD)
}

/// The accent bar drawn to the left of the selected row.
fn selection_bar() -> Span<'static> {
    Span::styled("▍ ", Style::new().fg(theme().accent))
}

/// A color for a known state/status word (issue/MR state, pipeline/job status);
/// `None` for anything unrecognized so ordinary cells stay default-colored.
fn state_color(s: &str) -> Option<Color> {
    Some(match s {
        "opened" | "open" | "success" | "passed" | "completed" | "active" => GREEN,
        "closed" | "failed" | "failure" | "canceled" | "cancelled" => RED,
        "merged" => MAGENTA,
        "protected" => MAGENTA,
        "running" | "pending" | "created" | "in_progress" | "queued" | "locked" => YELLOW,
        _ => return None,
    })
}

/// Clear `area`, draw a rounded bordered block titled `title`, and return the
/// inner rect for content. Used by every overlay for a consistent framed look.
fn overlay_frame(f: &mut Frame, area: Rect, title: &str, border: Color) -> Rect {
    let t = theme();
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        // Opaque panel background so stacked overlays never bleed through.
        .style(Style::new().bg(t.bg_alt).fg(t.fg))
        .border_style(Style::new().fg(border))
        .title(Span::styled(format!(" {title} "), Style::new().fg(border).add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

/// The keys worth knowing *right now*, most important first. The header shows as
/// many as fit, so what's listed changes with whatever the user is looking at.
fn context_keys(app: &App) -> Vec<(&'static str, &'static str)> {
    // Overlays are checked in the same order they take keys, so the hints always
    // describe the thing on top.
    if app.pending_action.is_some() {
        return vec![("y", "yes"), ("n", "no")];
    }
    if app.show_cmdlog {
        return vec![("x", "close")];
    }
    if app.show_help {
        return vec![("j/k", "scroll"), ("any", "close")];
    }
    if let Some(form) = app.new_item.as_ref() {
        if form.submitting {
            return vec![("Esc", "stop watching")];
        }
        let mut keys = vec![];
        if !form.matches().is_empty() {
            keys.push(("↑/↓", "pick"));
            keys.push(("Enter", "accept"));
        } else {
            keys.push(("Enter", "create"));
        }
        keys.push(("Tab", "field"));
        keys.push(("Esc", "cancel"));
        return keys;
    }
    if app.host_picker.is_some() {
        return vec![("↑/↓", "choose"), ("Enter", "use"), ("q", "quit")];
    }
    if app.tab_manager.is_some() {
        return vec![("J/K", "move"), ("space", "hide"), ("r", "reset"), ("q", "close")];
    }
    if app.theme_picker.is_some() {
        return vec![("↑/↓", "preview"), ("Enter", "keep"), ("Esc", "revert")];
    }
    if app.repo_picker.is_some() {
        return vec![("type", "filter"), ("↑/↓", "move"), ("Enter", "open"), ("Esc", "close")];
    }
    if app.log_view.is_some() {
        return vec![("j/k", "scroll"), ("G", "follow"), ("q", "close")];
    }
    if app.jobs_view.is_some() {
        return vec![("Enter", "log"), ("r", "retry"), ("d", "cancel"), ("q", "close")];
    }
    if app.diff.is_some() {
        return vec![("j/k", "scroll"), ("[ ]", "prev/next file"), ("a/M/c", "act"), ("q", "close")];
    }
    if app.detail.is_some() {
        let mut keys = vec![("j/k", "scroll")];
        if Tab::ALL[app.active] == Tab::MergeRequests {
            keys.push(("v", "diff"));
            keys.push(("a/M/c", "act"));
        }
        keys.push(("q", "close"));
        return keys;
    }
    if app.searching {
        return vec![("Enter", "apply"), ("Esc", "cancel")];
    }
    // The plain list: what this tab can actually do.
    let opens = match Tab::ALL[app.active] {
        Tab::Commits => "diff",
        Tab::Pipelines => "jobs",
        _ => "open",
    };
    let mut keys = vec![("Enter", opens)];
    match Tab::ALL[app.active] {
        Tab::MergeRequests => {
            keys.push(("v", "diff"));
            keys.push(("n", "new MR"));
            keys.push(("a/M/c", "approve/merge/close"));
        }
        Tab::Issues => {
            keys.push(("n", "new issue"));
            keys.push(("c/O", "close/reopen"));
            keys.push(("C", "comment"));
        }
        Tab::Pipelines => {
            keys.push(("n", "new pipeline"));
            keys.push(("r/d", "retry/cancel"));
        }
        Tab::Members => {
            keys.push(("n", "add member"));
        }
        Tab::Tags => {
            keys.push(("n", "new tag"));
        }
        _ => {}
    }
    keys.push(("f", "filter"));
    keys.push(("o", "web"));
    keys.push(("?", "help"));
    keys
}

/// Render `keys` right-aligned in `area`, dropping the least important ones
/// (they're ordered) until what's left fits the terminal.
fn render_context_keys(f: &mut Frame, area: Rect, mut keys: Vec<(&str, &str)>) {
    let t = theme();
    let spans = |keys: &[(&str, &str)]| -> Vec<Span<'static>> {
        keys.iter()
            .flat_map(|(k, label)| {
                [
                    Span::styled(
                        format!(" {k} "),
                        Style::new().bg(t.bg_sel).fg(t.fg_bright).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" {label}  "), Style::new().fg(t.fg_dim)),
                ]
            })
            .collect()
    };
    while !keys.is_empty() && Line::from(spans(&keys)).width() > area.width as usize {
        keys.pop();
    }
    if keys.is_empty() {
        return;
    }
    f.render_widget(
        Paragraph::new(Line::from(spans(&keys))).right_aligned().style(Style::new().bg(t.bg_alt)),
        area,
    );
}

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let t = theme();
    // Paint the whole canvas first so every widget sits on the theme background.
    f.render_widget(Block::default().style(Style::new().bg(t.bg).fg(t.fg)), area);
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
            Style::new().fg(t.bg).bg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(app.repo_label().to_string(), Style::new().fg(t.fg_bright).add_modifier(Modifier::BOLD)),
    ]);
    let left_w = (header.width() as u16 + 2).min(chunks[0].width);
    let head = Layout::horizontal([Constraint::Length(left_w), Constraint::Min(0)]).split(chunks[0]);
    f.render_widget(Paragraph::new(header).style(Style::new().bg(t.bg_alt)), head[0]);
    // Right of the repo name: the keys that do something in this exact view.
    render_context_keys(f, head[1], context_keys(app));

    // Tab bar: user-ordered, hidden tabs left out.
    let vis = app.visible_tabs();
    let titles: Vec<Line> = vis
        .iter()
        .map(|&i| Line::from(format!(" {} ", Tab::ALL[i].title())))
        .collect();
    let tabs = Tabs::new(titles)
        .select(vis.iter().position(|&i| i == app.active).unwrap_or(0))
        .style(Style::new().fg(t.fg_dim).bg(t.bg_alt))
        .highlight_style(Style::new().fg(t.accent).add_modifier(Modifier::BOLD))
        .divider(Span::styled("·", Style::new().fg(t.border)));
    f.render_widget(tabs, chunks[1]);

    // Table
    let tab = Tab::ALL[app.active];
    let header = UiRow::new(
        tab.headers().iter().map(|h| Cell::from(*h)).collect::<Vec<_>>(),
    )
    .style(Style::new().fg(t.fg_dim).bg(t.bg_alt).add_modifier(Modifier::BOLD));
    let visible = app.visible_rows();
    let last = tab.headers().len().saturating_sub(1);
    let body: Vec<UiRow> = visible
        .iter()
        .map(|r| {
            UiRow::new(
                r.cells
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        // State words win; otherwise columns read id → name → … → age.
                        let fg = match state_color(c) {
                            Some(col) => col,
                            None if i == 0 => CYAN,
                            None if i == 1 => t.fg_bright,
                            None if i == last => t.fg_dim,
                            None => t.fg,
                        };
                        Cell::from(Span::styled(c.clone(), Style::new().fg(fg)))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let widths: Vec<Constraint> = tab.weights().iter().map(|&w| Constraint::Fill(w)).collect();
    let title = Span::styled(
        format!(" {} ({}) ", tab.title(), visible.len()),
        Style::new().fg(t.accent).add_modifier(Modifier::BOLD),
    );
    let table = Table::new(body, widths)
        .header(header)
        .row_highlight_style(selection())
        .highlight_symbol(selection_bar())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .style(Style::new().bg(t.bg))
                .border_style(Style::new().fg(t.border))
                .title(title),
        );
    // Report the visible row count back so Ctrl-d/Ctrl-u jump by what's on
    // screen: the block's two borders and the header row aren't rows.
    let active = app.active;
    app.tabs[active].viewport_h = chunks[2].height.saturating_sub(3);
    f.render_stateful_widget(table, chunks[2], &mut app.tabs[active].table_state);

    // Status line
    let status = status_text(app);
    f.render_widget(
        Paragraph::new(status).style(Style::new().fg(t.fg_dim).bg(t.bg_alt)),
        chunks[3],
    );

    // Overlays live inside the table region, so the header (repo + the keys for
    // whatever is on top) and the tab bar stay readable behind them.
    let body = chunks[2];

    if app.show_help {
        render_help(f, body, app);
    }

    if app.detail.is_some() {
        render_detail(f, body, app);
    }

    // Diff stacks on top of a detail (opened via `v`), so it renders after it.
    if app.diff.is_some() {
        render_diff(f, body, app);
    }

    if app.jobs_view.is_some() {
        render_jobs(f, body, app);
    }

    // Log stacks on top of the jobs list.
    if app.log_view.is_some() {
        render_log(f, body, app);
    }

    if app.show_cmdlog {
        render_cmdlog(f, body);
    }

    if app.repo_picker.is_some() {
        render_repo_picker(f, body, app);
    }

    if app.theme_picker.is_some() {
        render_theme_picker(f, body, app);
    }

    if app.tab_manager.is_some() {
        render_tab_manager(f, body, app);
    }

    if app.host_picker.is_some() {
        render_host_picker(f, body, app);
    }

    if app.new_item.is_some() {
        render_new_item(f, body, app);
    }

    // The confirm modal renders last so it sits on top of any open overlay.
    if let Some(p) = &app.pending_action {
        render_confirm(f, body, &p.label);
    }
}

/// The `n` form: new MR/PR (with branch dropdowns), issue, member, pipeline, or tag.
fn render_new_item(f: &mut Frame, area: Rect, app: &App) {
    let Some(form) = app.new_item.as_ref() else { return };
    let th = theme();
    let noun = match form.kind {
        NewKind::Issue => "Issue".to_string(),
        NewKind::Member => "Member".to_string(),
        NewKind::Pipeline => "Pipeline".to_string(),
        NewKind::Tag => "Tag".to_string(),
        NewKind::Mr => (if app.is_gitlab() { "MR" } else { "PR" }).to_string(),
    };
    let matches = form.matches();
    let drop_h = if matches.is_empty() { 0 } else { (matches.len() as u16).min(6) + 1 };
    let w = 72.min(area.width);
    // A CLI error is a paragraph, not a word — give it wrapped lines to live in.
    let err_h = form
        .error
        .as_ref()
        .map(|e| wrapped_height(e, w.saturating_sub(4) as usize).min(ERROR_MAX_LINES) + 1)
        .unwrap_or(0);
    let h = (form.fields().len() as u16 * 2 + 3 + drop_h + err_h).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let title = if form.submitting {
        format!("New {noun} — creating…")
    } else if form.on_option_field() && !matches.is_empty() {
        format!("New {noun}  (↑/↓ pick · Enter accept · Tab field · Esc)")
    } else {
        format!("New {noun}  (Tab field · Enter create · Esc cancel)")
    };
    let inner = overlay_frame(f, rect, &title, th.accent);

    let mut lines: Vec<Line> = Vec::new();
    for (i, label) in form.fields().iter().enumerate() {
        let active = i == form.field && !form.submitting;
        let value = form.value(i);
        // Empty fields say what will happen instead of looking unfinished.
        let placeholder = match (form.kind, i) {
            (NewKind::Mr, 0) => "(current branch)",
            (NewKind::Mr, 1) | (NewKind::Pipeline, 0) | (NewKind::Tag, 1) => "(default branch)",
            (NewKind::Mr, 2) | (NewKind::Issue, 0) | (NewKind::Member, 0) | (NewKind::Tag, 0) => "(required)",
            (NewKind::Member, 1) => "(lowest role)",
            _ => "(none)",
        };
        let shown = if value.is_empty() && !active {
            placeholder.to_string()
        } else {
            // Keep the end of a long value visible — that's where the cursor is.
            fit_tail(value, w.saturating_sub(20) as usize)
        };
        let value_style = if value.is_empty() && !active {
            Style::new().fg(th.fg_dim)
        } else {
            Style::new().fg(th.fg_bright)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:>14}  ", format!("{label}:")),
                Style::new().fg(if active { th.accent } else { th.fg_dim }),
            ),
            Span::styled(shown, value_style),
            Span::styled(if active { "▏" } else { "" }, Style::new().fg(th.accent)),
        ]));

        // The dropdown hangs under the branch field it belongs to.
        if active && form.on_option_field() {
            if form.options_loading {
                lines.push(Line::from(Span::styled(
                    format!("{:>16}", "loading…"),
                    Style::new().fg(th.fg_dim),
                )));
            }
            let start = form.option_sel.saturating_sub(5);
            for (j, b) in matches.iter().enumerate().skip(start).take(6) {
                let picked = j == form.option_sel;
                let style = if picked {
                    Style::new().fg(th.fg_bright).bg(th.bg_sel).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(th.fg)
                };
                lines.push(Line::from(vec![
                    Span::styled(if picked { "              ▍ " } else { "                " }, Style::new().fg(th.accent)),
                    Span::styled((*b).clone(), style),
                ]));
            }
            if matches.len() > 6 {
                lines.push(Line::from(Span::styled(
                    format!("{:>16}{} more", "", matches.len() - 6),
                    Style::new().fg(th.fg_dim),
                )));
            }
        }
        lines.push(Line::from(""));
    }
    let (fields_area, error_area) = if err_h > 0 {
        let c = Layout::vertical([Constraint::Min(0), Constraint::Length(err_h)]).split(inner);
        (c[0], Some(c[1]))
    } else {
        (inner, None)
    };
    f.render_widget(Paragraph::new(lines), fields_area);
    if let (Some(area), Some(e)) = (error_area, form.error.as_ref()) {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(e.clone(), Style::new().fg(RED))))
                .wrap(Wrap { trim: true }),
            area,
        );
    }
}

/// The last `width` characters of `s`, marked with a leading `…` when clipped.
fn fit_tail(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width || width == 0 {
        return s.to_string();
    }
    let skip = n - width.saturating_sub(1);
    format!("…{}", s.chars().skip(skip).collect::<String>())
}

/// Longest line count `text` needs at `width` columns, for sizing an overlay.
fn wrapped_height(text: &str, width: usize) -> u16 {
    if width == 0 {
        return 1;
    }
    text.lines()
        .map(|l| l.chars().count().div_ceil(width).max(1) as u16)
        .sum::<u16>()
        .max(1)
}

/// Cap on how much of a CLI error the form shows before it's cut off.
const ERROR_MAX_LINES: u16 = 5;

/// Shown at startup when gitsmith runs outside a git repo: pick a service, then
/// (only when the CLI has several logins) which instance of it.
fn render_host_picker(f: &mut Frame, area: Rect, app: &App) {
    let Some(pick) = app.host_picker.as_ref() else { return };
    let th = theme();
    let w = 56.min(area.width);
    let h = match pick {
        HostPick::Service(_) => 7,
        HostPick::Instance { hosts, .. } => (hosts.len() as u16 + 5).clamp(7, 16),
    }
    .min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };

    match pick {
        HostPick::Service(sel) => {
            let exit = if app.has_repo() { "Esc closes" } else { "q quits" };
            let inner = overlay_frame(f, rect, &format!("Choose a host  (↑/↓ · Enter · {exit})"), th.accent);
            let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(inner);
            let intro = if app.has_repo() {
                ("Switching host.", "Pick a host, then choose a repo from your account.")
            } else {
                ("Not inside a git repo.", "Pick a host, then choose a repo from your account.")
            };
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(intro.0, Style::new().fg(th.fg))),
                    Line::from(Span::styled(intro.1, Style::new().fg(th.fg_dim))),
                ]),
                chunks[0],
            );
            let hosts = [("1", "GitLab", "glab"), ("2", "GitHub", "gh")];
            let rows: Vec<UiRow> = hosts
                .iter()
                .enumerate()
                .map(|(i, (key, name, cli))| {
                    let fg = if i == *sel { th.accent } else { th.fg_bright };
                    UiRow::new(vec![Cell::from(Line::from(vec![
                        Span::styled(format!("[{key}] "), Style::new().fg(th.fg_dim)),
                        Span::styled((*name).to_string(), Style::new().fg(fg).add_modifier(Modifier::BOLD)),
                        Span::styled(format!("  ({cli})"), Style::new().fg(th.fg_dim)),
                    ]))])
                })
                .collect();
            render_pick_list(f, chunks[1], rows, *sel);
        }
        HostPick::Instance { kind, hosts, selected, loading, origin } => {
            let cli = if *kind == crate::backend::Kind::Glab { "glab" } else { "gh" };
            let title = if *loading {
                format!("Instance — reading {cli}'s config…")
            } else if hosts.is_empty() {
                format!("No {cli} config found — run `{cli} auth login`  (q quits)")
            } else {
                "Instance  (↑/↓ · Enter · ← back · q quits)".to_string()
            };
            let inner = overlay_frame(f, rect, &title, th.accent);
            let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(inner);
            // Naming the file matters when someone keeps several GitLab configs.
            let source = if origin.is_empty() {
                "Which one holds your repos?".to_string()
            } else {
                format!("from {origin}")
            };
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        format!("{cli} is configured for more than one host."),
                        Style::new().fg(th.fg),
                    )),
                    Line::from(Span::styled(source, Style::new().fg(th.fg_dim))),
                ]),
                chunks[0],
            );
            let rows: Vec<UiRow> = hosts
                .iter()
                .enumerate()
                .map(|(i, host)| {
                    let fg = if i == *selected { th.accent } else { th.fg_bright };
                    UiRow::new(vec![Cell::from(Span::styled(host.clone(), Style::new().fg(fg)))])
                })
                .collect();
            render_pick_list(f, chunks[1], rows, *selected);
        }
    }
}

/// A selectable list inside a picker overlay.
fn render_pick_list(f: &mut Frame, area: Rect, rows: Vec<UiRow>, selected: usize) {
    let table = Table::new(rows, [Constraint::Percentage(100)])
        .row_highlight_style(selection())
        .highlight_symbol(selection_bar());
    let mut ts = TableState::default();
    ts.select(Some(selected));
    f.render_stateful_widget(table, area, &mut ts);
}

/// The `T` tab manager: the tab bar as a vertical list you can reorder (J/K)
/// and show/hide (space). Hidden tabs are dimmed and marked with an empty box.
fn render_tab_manager(f: &mut Frame, area: Rect, app: &App) {
    let Some(cur) = app.tab_manager else { return };
    let th = theme();
    let w = 46.min(area.width);
    let h = (Tab::ALL.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let inner = overlay_frame(f, rect, "Tabs  (J/K move · space hide · r reset · q)", th.accent);
    let rows: Vec<UiRow> = app
        .tab_order
        .iter()
        .enumerate()
        .map(|(pos, &t)| {
            let hidden = app.tab_hidden[t];
            let (mark, fg) = if hidden {
                ("☐", th.fg_dim)
            } else if t == app.active {
                ("☑", th.accent)
            } else {
                ("☑", th.fg_bright)
            };
            let cell = Cell::from(Line::from(vec![
                Span::styled(format!("{mark} "), Style::new().fg(fg)),
                Span::styled(Tab::ALL[t].title().to_string(), Style::new().fg(fg)),
            ]));
            let row = UiRow::new(vec![cell]);
            if pos == cur { row.style(selection()) } else { row }
        })
        .collect();
    let table = Table::new(rows, [Constraint::Percentage(100)])
        .row_highlight_style(selection())
        .highlight_symbol(selection_bar());
    let mut ts = TableState::default();
    ts.select(Some(cur));
    f.render_stateful_widget(table, inner, &mut ts);
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
    // Before any repo is chosen this is the way in, not a "switch".
    let verb = if app.has_repo() { "Switch repo" } else { "Open a repo" };
    let title = if rp.loading {
        format!("{verb} — loading…")
    } else if let Some(e) = &rp.error {
        format!("{verb} — error: {e}")
    } else {
        format!("{verb}   (type to filter · ↑/↓ · Enter · Esc)")
    };
    let inner = overlay_frame(f, rect, &title, theme().accent);
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);

    // Filter line.
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::new().fg(theme().accent)),
            Span::raw(rp.filter.clone()),
            Span::styled("▏", Style::new().fg(theme().fg_dim)),
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
                Some(true) => ("✓", GREEN),
                Some(false) => ("✗", RED),
                None => ("…", theme().fg_dim),
            };
            Line::from(vec![
                Span::styled(format!("{mark} "), Style::new().fg(color)),
                Span::styled("$ ", Style::new().fg(theme().fg_dim)),
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
                .style(Style::new().fg(theme().fg_dim)),
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
            let sc = state_color(&j.status).unwrap_or(theme().fg);
            // Bridge/trigger jobs drill into a downstream pipeline on Enter.
            let name = if j.downstream.is_some() { format!("{} ↳", j.name) } else { j.name.clone() };
            UiRow::new(vec![
                Cell::from(name),
                Cell::from(Span::styled(j.stage.clone(), Style::new().fg(theme().fg_dim))),
                Cell::from(Span::styled(j.status.clone(), Style::new().fg(sc))),
            ])
        })
        .collect();
    let widths = [Constraint::Percentage(50), Constraint::Percentage(20), Constraint::Percentage(30)];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(selection())
        .highlight_symbol(selection_bar());
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
    let lines: Vec<Line> = lv.rows.iter().map(|spans| log_line(spans)).collect();
    f.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
}

/// One line of job log. Output the CI colored itself is passed through as-is;
/// anything plain gets read for a severity word and a leading timestamp, so a
/// wall of uncolored output still shows where the failures are.
fn log_line(spans: &[(Color, String)]) -> Line<'static> {
    let t = theme();
    let colored = spans.iter().any(|(c, _)| *c != Color::Reset);
    if colored {
        return Line::from(
            spans.iter().map(|(c, s)| Span::styled(s.clone(), Style::new().fg(*c))).collect::<Vec<_>>(),
        );
    }
    let text: String = spans.iter().map(|(_, s)| s.as_str()).collect();
    let (stamp, rest) = split_timestamp(&text);
    let body_style = match severity(rest) {
        Some(c) => Style::new().fg(c),
        None => Style::new().fg(t.fg),
    };
    let mut out = Vec::new();
    if !stamp.is_empty() {
        out.push(Span::styled(stamp.to_string(), Style::new().fg(t.fg_dim)));
    }
    out.push(Span::styled(rest.to_string(), body_style));
    Line::from(out)
}

/// A leading ISO-8601 or `HH:MM:SS` timestamp, split off so it can be dimmed.
fn split_timestamp(line: &str) -> (&str, &str) {
    let end = line.find(' ').unwrap_or(0);
    let head = &line[..end];
    let stampish = head.len() >= 8
        && head.starts_with(|c: char| c.is_ascii_digit())
        && head.chars().all(|c| c.is_ascii_digit() || matches!(c, ':' | '-' | 'T' | '.' | '+' | 'Z'));
    if stampish { line.split_at(end) } else { ("", line) }
}

/// The severity a plain log line reports, if any.
fn severity(line: &str) -> Option<Color> {
    // Only the start of a line carries the level; a later "error" is prose.
    let head: String = line.chars().take(48).collect::<String>().to_uppercase();
    for (word, color) in [
        ("ERROR", RED),
        ("FATAL", RED),
        ("PANIC", RED),
        ("FAILED", RED),
        ("FAILURE", RED),
        ("WARN", YELLOW),
        ("INFO", CYAN),
        ("NOTICE", CYAN),
        ("DEBUG", theme().fg_dim),
        ("TRACE", theme().fg_dim),
    ] {
        if head.contains(word) {
            return Some(color);
        }
    }
    // GitLab's trace echoes each script line with `$` — worth spotting.
    line.trim_start().starts_with("$ ").then(|| theme().accent)
}

fn render_detail(f: &mut Frame, area: Rect, app: &App) {
    let dv = app.detail.as_ref().unwrap();
    let keys = if Tab::ALL[app.active] == Tab::MergeRequests {
        "j/k · v diff · a/M/c act · q close"
    } else {
        "j/k · q close"
    };
    let inner = overlay_frame(f, area, &format!("{}   ·   {keys}", dv.detail.title), theme().accent);

    let dim = Style::new().fg(theme().fg_dim);
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
            lines.push(Line::from(Span::styled(format!("comments: {e}"), Style::new().fg(RED))));
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
        format!("{}  ·  {} files  ·  j/k scroll · [ ] file · a/M/c act · q close", dv.title, dv.parsed.files)
    };
    let body = overlay_frame(f, area, &title, theme().accent);
    let rows = &dv.parsed.rows;
    if rows.is_empty() {
        return; // loading/empty is shown in the title
    }

    let start = (dv.scroll as usize).min(rows.len() - 1);
    // A file list down the side, like the web diff: every file in the change,
    // with the one you're reading marked — and it follows the scroll.
    let files = dv.parsed.file_positions();
    let body = if files.len() > 1 && body.width >= 80 {
        let cols = Layout::horizontal([Constraint::Length(26), Constraint::Min(0)]).split(body);
        render_diff_files(f, cols[0], &files, dv.parsed.file_at(start));
        cols[1]
    } else {
        body
    };

    // Sticky file header: the file the top visible row belongs to, pinned to the
    // top of the body so the filename stays visible while scrolling within a file.
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

/// The diff's file list: every changed file, the one being read highlighted.
fn render_diff_files(f: &mut Frame, area: Rect, files: &[(&str, usize)], current: usize) {
    let t = theme();
    let width = area.width.saturating_sub(3) as usize;
    // Keep the current file on screen when a change touches more files than fit.
    let rows_visible = area.height.saturating_sub(1) as usize;
    let start = current.saturating_sub(rows_visible.saturating_sub(1));
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("{:<w$}", " FILES", w = area.width as usize),
        Style::new().fg(t.fg_dim).bg(t.bg_alt).add_modifier(Modifier::BOLD),
    ))];
    for (i, (path, _)) in files.iter().enumerate().skip(start).take(rows_visible) {
        let name = path.rsplit('/').next().unwrap_or(path);
        let shown = fit_tail(name, width);
        let style = if i == current {
            Style::new().fg(t.accent).bg(t.bg_sel).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(t.fg_dim)
        };
        lines.push(Line::from(vec![
            Span::styled(if i == current { "▍ " } else { "  " }, Style::new().fg(t.accent)),
            Span::styled(shown, style),
        ]));
    }
    if files.len() > start + rows_visible {
        lines.push(Line::from(Span::styled(
            format!("  +{} more", files.len() - start - rows_visible),
            Style::new().fg(t.fg_dim),
        )));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// A prominent full-width file bar (`📄 path`) for the diff viewer.
fn file_bar(path: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" 📄 {path} "),
        Style::new().fg(theme().bg).bg(theme().accent).add_modifier(Modifier::BOLD),
    ))
}

/// A right-aligned line-number gutter span (dim); blank when there's no number
/// (added line on the old side, or a removed line on the new side).
fn gutter(lnum: Option<u32>) -> Span<'static> {
    let text = match lnum {
        Some(n) => format!("{n:>4} "),
        None => "     ".to_string(),
    };
    Span::styled(text, Style::new().fg(theme().fg_dim))
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
    if let Some(q) = app.search() {
        format!("/{q}")
    } else if ts.loading {
        "working…".into()
    } else if let Some(f) = &app.flash {
        f.clone()
    } else if let Some(e) = &ts.error {
        format!("error: {e}")
    } else if !app.has_repo() {
        "no repo selected — press P to choose one   (? help · ^C quit)".into()
    } else if let Some((line, ok)) = crate::backend::recent_cmds().pop() {
        let mark = match ok {
            Some(true) => "✓",
            Some(false) => "✗",
            None => "…",
        };
        format!("$ {line}  {mark}    (x cmds · ? help)")
    } else {
        "h/l tabs · j/k move · P repo · S host · T tabs · t theme · R refresh · x cmds · ? help · ^C quit".into()
    }
}

/// The `?` manual: keys grouped by what they're for, with the tab-specific
/// actions spelled out. Scrolls with j/k when the terminal is short.
fn render_help(f: &mut Frame, area: Rect, app: &App) {
    let th = theme();
    let sections: [(&str, &[(&str, &str)]); 5] = [
        (
            "Navigation",
            &[
                ("h  l", "previous / next tab (← →; hidden tabs are skipped)"),
                ("j  k", "move selection (↓ ↑)"),
                ("^d  ^u", "half a screen down / up"),
                ("Enter", "open detail — jobs on Pipelines, diff on Commits"),
                ("Esc", "close an overlay, or clear this tab's filter"),
                ("q", "close the top overlay"),
            ],
        ),
        (
            "Viewing",
            &[
                ("v", "side-by-side diff (MRs/PRs)"),
                ("f  /", "filter this tab — each tab keeps its own query"),
                ("o", "open the selected item in the browser"),
                ("x", "commands gitsmith has run, with ✓/✗"),
                ("R", "refresh the current tab"),
            ],
        ),
        (
            "Changing things",
            &[
                ("n", "new MR/PR, issue, member, pipeline, or tag — on those tabs"),
                ("a  M  c", "approve · merge · close (MRs/PRs)"),
                ("c  O  C", "close · reopen · comment (Issues)"),
                ("r  d", "retry · cancel (Pipelines, and jobs inside one)"),
                ("y  n", "confirm / cancel the [y/n] prompt"),
            ],
        ),
        (
            "Where you are",
            &[
                ("P", "switch repo — fuzzy-find across your account"),
                ("S", "switch host — GitLab / GitHub, and which instance"),
            ],
        ),
        (
            "Appearance",
            &[
                ("T", "tab manager — J/K reorder, space hides, r resets"),
                ("t", "theme picker (9 themes, live preview)"),
                ("?", "open or close this manual"),
                ("Ctrl-C", "quit"),
            ],
        ),
    ];

    let mut lines: Vec<Line> = Vec::new();
    for (name, rows) in sections {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            name.to_uppercase(),
            Style::new().fg(th.accent).add_modifier(Modifier::BOLD),
        )));
        for (keys, desc) in rows {
            let mut spans = vec![Span::raw(" ")];
            let mut width = 1;
            for k in keys.split("  ") {
                spans.push(Span::styled(
                    format!(" {k} "),
                    Style::new().bg(th.bg_sel).fg(th.fg_bright).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
                width += k.chars().count() + 3;
            }
            // Pad so every description starts in the same column.
            spans.push(Span::raw(" ".repeat(KEY_COL.saturating_sub(width))));
            spans.push(Span::styled((*desc).to_string(), Style::new().fg(th.fg)));
            lines.push(Line::from(spans));
        }
    }

    let w = 72.min(area.width);
    let body_h = lines.len() as u16;
    let h = (body_h + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    // Only scrollable when it doesn't fit; keep the tail reachable, no further.
    let max_scroll = body_h.saturating_sub(rect.height.saturating_sub(2));
    let scroll = app.help_scroll.min(max_scroll);
    let hint = if max_scroll > 0 { "j/k scroll · any key closes" } else { "any key closes" };
    let inner = overlay_frame(f, rect, &format!("gitsmith — keys  ({hint})"), th.accent);
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
}

/// Column where help descriptions line up, in cells.
const KEY_COL: usize = 14;

fn render_confirm(f: &mut Frame, area: Rect, label: &str) {
    let w = (label.len() as u16 + 8).min(area.width).max(20);
    let h = 5.min(area.height);
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let inner = overlay_frame(f, rect, "Confirm", YELLOW);
    let text = Line::from(vec![
        Span::raw(label.to_string()),
    ]);
    let keys = Line::from(vec![
        Span::styled("[y]", Style::new().fg(GREEN).add_modifier(Modifier::BOLD)),
        Span::raw("es  "),
        Span::styled("[n]", Style::new().fg(RED).add_modifier(Modifier::BOLD)),
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
        fn list(&self, _t: Tab, _page: u32) -> anyhow::Result<Vec<Row>> {
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
    fn tab_bar_reflects_order_and_hidden_tabs() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.tab_order.swap(0, 2); // Pipelines first
        app.tab_hidden[1] = true; // hide MRs/PRs
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let bar: String = (0..80u16)
            .map(|x| terminal.backend().buffer()[(x, 1u16)].symbol())
            .collect();
        assert!(bar.trim_start().starts_with("Pipelines"), "reordered bar: {bar:?}");
        assert!(!bar.contains("MRs/PRs"), "hidden tab still drawn: {bar:?}");
    }

    #[test]
    fn renders_host_picker_outside_a_repo() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::choose_host(tx);
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("Choose a host"));
        assert!(text.contains("Not inside a git repo."));
        assert!(text.contains("GitLab") && text.contains("GitHub"));
        assert!(text.contains("press P to choose one"), "status line explains the next step");
    }

    #[test]
    fn renders_new_mr_form() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.active = 1;
        app.handle_key(ratatui::crossterm::event::KeyEvent::from(
            ratatui::crossterm::event::KeyCode::Char('n'),
        ));
        let mut terminal = Terminal::new(TestBackend::new(90, 20)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("New MR"), "GitLab wording");
        assert!(text.contains("Source branch") && text.contains("Target branch"));
        assert!(text.contains("Title") && text.contains("Description"));
        // empty, unfocused fields explain what the CLI will default to
        assert!(text.contains("(default branch)"));
        assert!(text.contains("(none)"));
    }

    #[test]
    fn long_cli_errors_are_shown_in_full() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.active = 1;
        app.handle_key(ratatui::crossterm::event::KeyEvent::from(
            ratatui::crossterm::event::KeyCode::Char('n'),
        ));
        let msg = "`glab` failed: Failed to create merge request. Created record could not be \
                   found. Check that the source branch exists on the remote and that no open \
                   merge request already targets it.";
        app.new_item.as_mut().unwrap().error = Some(msg.to_string());
        let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        // the tail of the message survives, i.e. it wrapped instead of being clipped
        assert!(text.contains("Failed to create merge request"));
        assert!(text.contains("already targets it"), "the end of the error is visible");
    }

    #[test]
    fn long_field_values_show_their_tail() {
        assert_eq!(fit_tail("short", 20), "short");
        assert_eq!(fit_tail("abcdefghij", 5), "…ghij");
        // a pasted path keeps the part you're typing at
        assert!(fit_tail("/var/folders/_w/7xdq/T/TemporaryItems/NSIRD_x/Screenshot.png", 20)
            .ends_with("Screenshot.png"));
    }

    #[test]
    fn header_hints_follow_the_current_view() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        let mut terminal = Terminal::new(TestBackend::new(120, 12)).unwrap();
        let header = |t: &Terminal<TestBackend>| -> String {
            (0..120u16).map(|x| t.backend().buffer()[(x, 0u16)].symbol()).collect()
        };

        // Issues: what you can do to an issue
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let line = header(&terminal);
        assert!(line.contains("new issue") && line.contains("close/reopen"));
        assert!(!line.contains("diff"), "MR-only keys stay on the MR tab");

        // MRs: diff and the approve/merge/close trio
        app.active = 1;
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let line = header(&terminal);
        assert!(line.contains("diff") && line.contains("approve/merge/close"));

        // inside an overlay the hints describe the overlay, and the header
        // itself stays visible above it
        app.handle_key(ratatui::crossterm::event::KeyEvent::from(
            ratatui::crossterm::event::KeyCode::Char('T'),
        ));
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let line = header(&terminal);
        assert!(line.contains("gitsmith"), "the header survives an overlay");
        assert!(line.contains("hide") && line.contains("reset"));
        assert!(!line.contains("new issue"));
    }

    #[test]
    fn header_hints_shrink_before_they_overflow() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.active = 1;
        for width in [40u16, 60, 80, 120] {
            let mut terminal = Terminal::new(TestBackend::new(width, 10)).unwrap();
            terminal.draw(|f| render(f, &mut app)).unwrap();
            let line: String =
                (0..width).map(|x| terminal.backend().buffer()[(x, 0u16)].symbol()).collect();
            assert_eq!(line.chars().count(), width as usize, "never wraps at {width} cols");
            assert!(line.contains("gitsmith"), "the brand is never pushed out at {width} cols");
        }
    }

    #[test]
    fn diff_shows_a_file_list_that_follows_the_scroll() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        let mut diff = String::new();
        for f in ["src/alpha.rs", "src/beta.rs", "src/gamma.rs"] {
            diff.push_str(&format!("diff --git a/{f} b/{f}\n@@ -1,3 +1,3 @@\n one\n two\n three\n"));
        }
        let parsed = crate::diff::parse(&diff);
        let second = parsed.file_positions()[1].1 as u16;
        app.diff = Some(crate::app::DiffView {
            title: "MR !1 diff".into(),
            parsed,
            scroll: 0,
            loading: false,
            error: None,
        });
        let mut terminal = Terminal::new(TestBackend::new(110, 16)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        // every file is listed, and the sticky bar names the one in view
        assert!(text.contains("FILES"));
        for name in ["alpha.rs", "beta.rs", "gamma.rs"] {
            assert!(text.contains(name), "file list is missing {name}");
        }
        // the marker sits on the first file at the top of the diff
        let marked_first: String = (0..110u16)
            .map(|x| terminal.backend().buffer()[(x, 4u16)].symbol())
            .collect();
        assert!(marked_first.contains("▍ alpha.rs"), "got {marked_first:?}");

        // scroll into the second file and the marker moves with it
        app.diff.as_mut().unwrap().scroll = second;
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let marked: String = (0..110u16)
            .map(|x| terminal.backend().buffer()[(x, 5u16)].symbol())
            .collect();
        assert!(marked.contains("▍ beta.rs"), "got {marked:?}");
    }

    #[test]
    fn plain_log_lines_are_colored_by_severity() {
        let red = severity("ERROR: build failed");
        assert_eq!(red, Some(RED));
        assert_eq!(severity("2026-08-05 WARN retrying"), Some(YELLOW));
        assert_eq!(severity("INFO starting"), Some(CYAN));
        assert_eq!(severity("$ cargo build"), Some(theme().accent));
        assert_eq!(severity("just some output"), None);
        // "error" late in a sentence is prose, not a level
        let long = format!("{} and then an error happened", "x".repeat(60));
        assert_eq!(severity(&long), None);
    }

    #[test]
    fn log_timestamps_are_split_off() {
        assert_eq!(split_timestamp("2026-08-05T10:00:00Z building"), ("2026-08-05T10:00:00Z", " building"));
        assert_eq!(split_timestamp("10:00:00 building"), ("10:00:00", " building"));
        assert_eq!(split_timestamp("building things"), ("", "building things"));
        assert_eq!(split_timestamp(""), ("", ""));
    }

    #[test]
    fn cli_colors_survive_our_log_coloring() {
        // a line the CI colored itself is passed through untouched
        let spans = vec![(GREEN, "ok".to_string()), (Color::Reset, " done".to_string())];
        let line = log_line(&spans);
        assert_eq!(line.spans[0].style.fg, Some(GREEN));
        // an uncolored ERROR line gets ours
        let plain = vec![(Color::Reset, "ERROR boom".to_string())];
        let line = log_line(&plain);
        assert_eq!(line.spans.last().unwrap().style.fg, Some(RED));
    }

    #[test]
    fn the_table_reports_its_height_back() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.tabs[0].rows = (0..40)
            .map(|i| Row {
                id: i.to_string(),
                cells: vec![i.to_string(), "row".into(), String::new(), String::new(), String::new(), String::new()],
                web_url: String::new(),
                raw: serde_json::Value::Null,
            })
            .collect();
        // 24 rows tall: header + tabs + status take 3, the block's borders 2,
        // the column header 1 — leaving 18 rows of table
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        assert_eq!(app.tabs[0].viewport_h, 18);

        // a shorter terminal reports less, so a half-page shrinks with it
        let mut small = Terminal::new(TestBackend::new(80, 12)).unwrap();
        small.draw(|f| render(f, &mut app)).unwrap();
        assert_eq!(app.tabs[0].viewport_h, 6);
    }

    #[test]
    fn renders_the_key_manual() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.show_help = true;
        let mut terminal = Terminal::new(TestBackend::new(90, 40)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        for section in ["NAVIGATION", "VIEWING", "CHANGING THINGS", "WHERE YOU ARE", "APPEARANCE"] {
            assert!(text.contains(section), "missing section {section}");
        }
        assert!(text.contains("new MR/PR"));
        assert!(text.contains("switch host"));
        assert!(text.contains("Ctrl-C"));
    }

    #[test]
    fn renders_tab_manager() {
        let (tx, _rx) = mpsc::channel();
        let mut app = App::new(Arc::new(Fake), Kind::Glab, "o/r".into(), tx);
        app.tab_hidden[1] = true;
        app.tab_manager = Some(1);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let text = buffer_to_string(terminal.backend().buffer());
        assert!(text.contains("space hide"));
        assert!(text.contains("☐ MRs/PRs"), "hidden tab shows an empty box");
        assert!(text.contains("☑ Issues"));
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









