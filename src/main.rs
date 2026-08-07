mod action;
mod app;
mod backend;
mod config;
mod ansi;
mod detail;
mod diff;
mod fetch;
mod format;
mod highlight;
mod jobs;
mod ui;

use anyhow::Result;
use app::App;
use ratatui::crossterm::event::{self, Event};
use std::sync::mpsc;
use std::time::Duration;

fn main() -> Result<()> {
    let (tx, rx) = mpsc::channel();
    // Outside a git repo (or without a GitLab/GitHub remote) there's nothing to
    // infer, so start on the host picker instead of exiting.
    let mut app = match backend::detect_kind() {
        Some(kind) => App::new(
            backend::backend_for(kind),
            kind,
            backend::current_repo().unwrap_or_default(),
            tx,
        ),
        None => App::choose_host(tx),
    };
    if let Some(path) = config::path() {
        app.load_config(path);
    }

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, rx);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: mpsc::Receiver<fetch::Msg>,
) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|f| ui::render(f, app))?;

        while let Ok(msg) = rx.try_recv() {
            app.apply(msg);
        }

        // Live-tail an open job log (re-polls on an interval when following).
        app.tick();

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == event::KeyEventKind::Press
        {
            app.handle_key(key);
        }

        if let Some(id) = app.take_comment_request() {
            // Suspend the TUI, edit, post, restore.
            ratatui::restore();
            // Read the backend now — the host picker can have swapped it.
            let backend = app.backend();
            let posted = action::edit_comment().and_then(|body| match body {
                Some(b) => backend.comment(&id, &b).map(|_| true),
                None => Ok(false),
            });
            *terminal = ratatui::init();
            terminal.clear()?;
            match posted {
                Ok(true) => app.refresh_active(),
                Ok(false) => {}
                Err(e) => app.flash = Some(format!("comment failed: {e:#}")),
            }
        }
    }
    Ok(())
}
