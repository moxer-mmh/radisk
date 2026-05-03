mod app;
mod color;
mod config;
mod context_menu;
mod keybinds;
mod radial;
mod renderer;
mod scanner;
mod scanner_streaming;
mod tree;
mod ui;
mod views;

use anyhow::{bail, Context, Result};
use app::App;
use clap::Parser;
use config::Config;
use crossterm::{
    cursor,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, size, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io, panic, path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(
    name = "radisk",
    about = "Terminal-based radial disk usage visualizer",
    after_help = format!(
        "Support this project: \x1b]8;;https://ko-fi.com/mimobn_\x1b\\ko-fi.com/mimobn_\x1b]8;;\x1b\\"
    )
)]
struct Cli {
    /// Path to scan
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Number of concentric rings to display. Overrides
    /// `display.ring_depth` from the config file.
    #[arg(short, long)]
    depth: Option<usize>,

    /// Path to a TOML config file. Defaults to
    /// `$XDG_CONFIG_HOME/radisk/config.toml` (or the platform equivalent).
    #[arg(long)]
    config: Option<PathBuf>,
}

/// Restore terminal to usable state
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        cursor::Show
    );
    // Move cursor to bottom of screen
    if let Ok((_, rows)) = size() {
        let _ = execute!(io::stdout(), cursor::MoveTo(0, rows.saturating_sub(1)));
    }
}

/// Guard to ensure terminal is restored on drop
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let path = cli
        .path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", cli.path.display()))?;
    if !path.is_dir() {
        bail!("{} is not a directory", path.display());
    }

    // Load config: explicit --config wins, else the platform default path
    // (which falls back to compiled-in defaults if missing). CLI flags
    // applied after loading override file values.
    let mut cfg = match cli.config.as_deref() {
        Some(p) => Config::load_from_path(p)?,
        None => Config::load_default()?,
    };
    if let Some(d) = cli.depth {
        cfg.display.ring_depth = d.max(1);
    }

    // Setup panic hook to restore terminal
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original_hook(info);
    }));

    // Setup terminal
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        cursor::Hide
    )
    .context("failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to construct terminal backend")?;

    // Create guard to ensure cleanup
    let _guard = TerminalGuard;

    // Clear screen
    terminal.clear().context("failed to clear terminal")?;

    // Get actual terminal size
    let (term_width, term_height) = size().unwrap_or((80, 24));

    // Create app and run
    let mut app = App::new(path.clone(), cfg, term_width, term_height);
    let result = run_app(&mut terminal, &mut app);

    // Proper restore sequence
    let _ = terminal.clear();
    restore_terminal();
    // Move cursor to bottom after leaving alternate screen
    if let Ok((_, rows)) = size() {
        let _ = execute!(io::stdout(), cursor::MoveTo(0, rows.saturating_sub(1)));
    }

    // Prevent guard from running again
    std::mem::forget(_guard);

    result
}

fn run_app<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()>
where
    <B as ratatui::backend::Backend>::Error: std::error::Error + Send + Sync + 'static,
{
    // Start initial scan
    app.start_scan();

    loop {
        // Update scan progress
        app.update_scan_progress();

        // Draw
        terminal
            .draw(|f| ui::render(f, app))
            .context("failed to draw frame")?;

        // Handle events
        if event::poll(Duration::from_millis(50)).context("failed to poll for events")? {
            match event::read().context("failed to read terminal event")? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.handle_key(key);
                }
                Event::Mouse(mouse) => {
                    app.handle_mouse(mouse);
                }
                Event::Resize(width, height) => {
                    app.resize(width, height);
                }
                _ => {}
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
