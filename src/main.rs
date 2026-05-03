mod app;
mod color;
mod config;
mod context_menu;
mod delete;
mod keybinds;
mod radial;
mod renderer;
mod scanner;
mod scanner_streaming;
mod snapshot;
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
    /// Path to scan. Ignored when `--import` is set.
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

    /// Glob pattern to skip while walking. May be repeated. Patterns
    /// are matched against both the full path and the base name, so
    /// `--exclude node_modules` and `--exclude '**/.cache/**'` both
    /// work. Adds to (does not replace) `[scan].exclude` from the
    /// config file.
    #[arg(long = "exclude", value_name = "PATTERN")]
    exclude: Vec<String>,

    /// Write a snapshot of the completed scan to PATH and exit
    /// without entering the TUI. Useful for archiving the state of a
    /// large filesystem or sharing it with someone else for offline
    /// analysis.
    #[arg(long, value_name = "PATH")]
    export: Option<PathBuf>,

    /// Open an existing snapshot instead of scanning the filesystem.
    /// `path` is ignored. Useful for inspecting an exported tree on a
    /// machine without filesystem access to the original target.
    #[arg(long, value_name = "PATH", conflicts_with = "export")]
    import: Option<PathBuf>,
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
    if !cli.exclude.is_empty() {
        cfg.scan.exclude.extend(cli.exclude);
    }

    // --export PATH: scan headlessly and write a snapshot. No TUI.
    if let Some(out) = cli.export.as_deref() {
        return run_headless_export(&cli.path, &cfg, out);
    }

    // --import PATH: load a snapshot and open the TUI on it. The
    // positional `path` is ignored; we keep it accepted because clap
    // makes it required-by-default.
    let import_arena = match cli.import.as_deref() {
        Some(snap) => Some(snapshot::load(snap)?),
        None => None,
    };

    // For scan mode (the default), validate the path up front so the
    // user gets a clean error before we touch the terminal.
    let (path, import_label) = if let Some(snap) = cli.import.as_deref() {
        // The arena's stored root path is what the App should display.
        let label = snap.display().to_string();
        let path = import_arena
            .as_ref()
            .and_then(|a| a.root().map(|root| a.folder(root).file.path.clone()))
            .unwrap_or_else(|| PathBuf::from("/"));
        (path, Some(label))
    } else {
        let p = cli
            .path
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", cli.path.display()))?;
        if !p.is_dir() {
            bail!("{} is not a directory", p.display());
        }
        (p, None)
    };

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
    if let (Some(arena), Some(label)) = (import_arena, import_label) {
        app.adopt_loaded_arena(arena, label);
    }
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

/// Run a scan with no UI and write the resulting arena to `out`. Used
/// by `--export` so users can snapshot huge filesystems on a
/// headless box, then open the resulting `.radisk` file with
/// `--import` on a workstation.
fn run_headless_export(path: &std::path::Path, cfg: &Config, out: &std::path::Path) -> Result<()> {
    use scanner_streaming::{scan_streaming, ScanEvent};

    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", path.display()))?;
    if !canonical.is_dir() {
        bail!("{} is not a directory", canonical.display());
    }

    let scan_cfg = cfg.to_scan_config();
    let handle = scan_streaming(&canonical, &scan_cfg);

    eprintln!("scanning {} ...", canonical.display());

    let mut last_files = 0u64;
    let mut last_size = 0u64;
    let arena = loop {
        match handle.rx.recv().context("scanner channel closed early")? {
            ScanEvent::Progress {
                files, total_size, ..
            } => {
                last_files = files;
                last_size = total_size;
            }
            ScanEvent::Warning(msg) => eprintln!("warn: {}", msg),
            ScanEvent::Complete(arena) => break *arena,
            ScanEvent::Failed(reason) => bail!("scan failed: {}", reason),
        }
    };

    let bytes = snapshot::save(&arena, out)
        .with_context(|| format!("failed to write snapshot {}", out.display()))?;
    eprintln!(
        "wrote {} ({} files, {} bytes scanned, {} bytes on disk)",
        out.display(),
        last_files.max(arena_file_count(&arena)),
        last_size.max(arena_root_size(&arena)),
        bytes
    );
    Ok(())
}

fn arena_file_count(arena: &tree::TreeArena) -> u64 {
    arena
        .root()
        .map(|root| arena.total_file_count(root))
        .unwrap_or(0)
}

fn arena_root_size(arena: &tree::TreeArena) -> u64 {
    arena
        .root()
        .map(|root| arena.folder(root).file.size)
        .unwrap_or(0)
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
