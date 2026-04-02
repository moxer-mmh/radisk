mod app;
mod color;
mod context_menu;
mod radial;
mod renderer;
mod scanner;
mod tree;
mod ui;

use app::App;
use clap::Parser;
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
#[command(name = "radisk", about = "Terminal-based radial disk usage visualizer")]
struct Cli {
    /// Path to scan
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Number of concentric rings to display
    #[arg(short, long, default_value = "4")]
    depth: usize,
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let path = cli.path.canonicalize()?;
    if !path.is_dir() {
        eprintln!("Error: {} is not a directory", path.display());
        std::process::exit(1);
    }

    // Setup panic hook to restore terminal
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original_hook(info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        cursor::Hide
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create guard to ensure cleanup
    let _guard = TerminalGuard;

    // Clear screen
    terminal.clear()?;

    // Get actual terminal size
    let (term_width, term_height) = size().unwrap_or((80, 24));

    // Create app and run
    let mut app = App::new(path.clone(), cli.depth, term_width, term_height);
    let result = run_app(&mut terminal, &mut app);

    // Proper restore sequence
    terminal.clear()?;
    restore_terminal();
    // Move cursor to bottom after leaving alternate screen
    if let Ok((_, rows)) = size() {
        let _ = execute!(io::stdout(), cursor::MoveTo(0, rows.saturating_sub(1)));
    }

    // Prevent guard from running again
    std::mem::forget(_guard);

    if let Err(err) = result {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    }

    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<(), Box<dyn std::error::Error>>
where
    <B as ratatui::backend::Backend>::Error: 'static,
{
    // Start initial scan
    app.start_scan();

    loop {
        // Update scan progress
        app.update_scan_progress();

        // Draw
        terminal.draw(|f| ui::render(f, app))?;

        // Handle events
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
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
