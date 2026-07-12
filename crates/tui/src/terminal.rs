//! The terminal driver — the *only* module that touches crossterm and the real terminal.
//! Raw mode, the alternate screen, the panic hook, and the blocking event loop live here and
//! nowhere else, so the rest of the crate stays driver-agnostic and testable.

use std::io::{self, Stdout};

use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::App;
use crate::ui;

/// The concrete terminal type the app draws to.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Enter raw mode + the alternate screen and build the ratatui terminal.
pub fn init() -> io::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

/// Restore the user's terminal: leave the alternate screen and disable raw mode. Safe to call
/// more than once and on any exit path.
pub fn restore() -> io::Result<()> {
    execute!(io::stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()
}

/// Install a panic hook that restores the terminal *before* the default hook prints the panic,
/// so a crash never leaves the user in a raw alternate screen. The single most important
/// correctness detail of the shell.
pub fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        original(info);
    }));
}

/// The blocking event loop: draw, wait for input, apply it, repeat until the app asks to quit.
/// Blocking `event::read` means zero idle CPU. `handle_key` filters key kind itself, and any
/// other event (e.g. a resize) simply falls through to a redraw on the next iteration.
pub fn run(terminal: &mut Tui, app: &mut App) -> io::Result<()> {
    while !app.should_quit() {
        let body_height = terminal.size()?.height.saturating_sub(1) as usize;
        app.set_page(body_height);
        app.update_scroll(body_height);
        terminal.draw(|frame| ui::render(frame, app))?;

        if let Event::Key(key) = event::read()? {
            app.handle_key(key);
        }
    }
    Ok(())
}
