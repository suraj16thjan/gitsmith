mod action;
mod app;
mod backend;
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
use backend::Backend;
use ratatui::crossterm::event::{self, Event};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

fn main() -> Result<()> {
    let (backend, kind, repo) = backend::detect()?;
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(backend.clone(), kind, repo.unwrap_or_default(), tx);

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, rx, &backend);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: mpsc::Receiver<fetch::Msg>,
    backend: &Arc<dyn Backend>,
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
