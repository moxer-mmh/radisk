# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added (Phase 4 — tree view)
- New **tree view** alt mode: ncdu-style indented list of the focused
  folder's children with a proportional size bar, percentage of the
  largest child, and trailing-slash folder marker. Sorted by size
  descending. Selection cursor is shared with the sidebar so j/k
  navigates both views consistently.
- New `Action::ToggleView` (default chord: `v`) cycles the active
  view. Adding a future split view will not change the keybind —
  `View::next` simply gains another arm.
- New `views` module owns the `View` enum and the tree renderer.
  Pure `build_rows` function builds the row list from an arena +
  items slice; ratatui rendering is a thin shell over it. Seven new
  unit tests cover ordering, percent scaling, bar width invariants,
  folder marker, empty folders, and the size=0 division-by-zero
  guard.
- The help screen and the bottom-of-screen hint bar both list `[v] View`.
- `docs/config.example.toml` documents the new `toggle_view` action
  for users who want to rebind it.

### Added (Phase 3 — config & keybinds)
- **TOML config file** at `$XDG_CONFIG_HOME/radisk/config.toml` (with
  platform fallbacks via the `directories` crate). All keys optional;
  missing files fall back to compiled-in defaults; malformed files
  surface a contextual error instead of silently using defaults.
  - `[display]` — `ring_depth`, `sidebar_percent` (clamped 10..=60)
  - `[scan]` — `follow_symlinks`, `max_depth`
  - `[keybinds]` — per-action chord overrides
  - `[colors]` — parsed and stored verbatim, reserved for the upcoming
    theme integration (Phase 3.1)
- `--config <PATH>` CLI flag to load an explicit config file, useful
  for testing alternative bindings without touching the system config.
- **Rebindable keybinds** in `Viewing` mode. New module `keybinds`:
  - `Action` enum closes the set of bindable verbs the App understands
    (`quit`, `help`, `navigate_up`, `navigate_into`, `zoom_in`,
    `zoom_out`, `rescan`, `delete`, `toggle_focus`, `move_up`,
    `move_down`).
  - `KeyChord` parses a small DSL (`"q"`, `"esc"`, `"ctrl+q"`,
    `"shift+down"`, `"alt+enter"`) so config files stay
    human-friendly. SHIFT is normalised away for letter keys so a
    config of `"q"` matches whether or not the terminal sent SHIFT.
  - User overrides REPLACE every default chord for the action they
    name and add the supplied chord; defaults for other actions are
    preserved.
  - Invalid keybinds in the config print a warning and fall back to
    defaults rather than refusing to start the App.
- `docs/config.example.toml` ships every documented key with
  inline syntax notes, ready to drop into a user's config dir.

### Changed
- `App::new` signature: `(path, config, term_w, term_h)` instead of
  `(path, ring_depth, term_w, term_h)`. Ring depth now flows in via
  `Config::display.ring_depth`; the CLI `--depth` flag overrides the
  file value if present.
- `App::start_scan` reads scanner options (`follow_symlinks`,
  `max_depth`) from the loaded config rather than `ScanConfig::default`.
- `App::handle_viewing_key` no longer hard-codes the chord→behaviour
  mapping; it looks up an `Action` via `Keybinds::action_for` and
  dispatches through `App::dispatch_action`, a single fan-out point
  that future input sources can reuse.

### Performance
- Initial scan is **9–14× faster** vs. the legacy synchronous walker on
  representative trees (release-mode benchmarks, single laptop):

  | target           | files   | legacy  | streaming | speedup |
  | ---------------- | ------- | ------- | --------- | ------- |
  | `/usr/share`     | 215 039 | 1.951 s |  0.203 s  | 9.63×   |
  | `/usr/lib`       | 181 730 | 2.510 s |  0.185 s  | 13.54×  |
  | `~/.cargo`       |  15 166 | 0.338 s |  0.027 s  | 12.31×  |

  Reproduce with `cargo run --release --example bench_scan -- <path>`.

### Added
- **Streaming parallel scanner** (`scanner_streaming` module): replaces the
  blocking `std::fs::read_dir` recursion with a [`jwalk`]-based parallel
  walker that runs on its own thread and emits coalesced
  `ScanEvent::Progress` updates every ~80ms. The UI loop drains events
  each frame, so the file/size counters and the currently-scanned path
  advance live in the status bar instead of freezing during the scan.
  - `ScanEvent::{Progress, Warning, Complete, Failed}` enum.
  - `ScanHandle` joins the worker thread on drop so cancelling the app
    cannot leak threads.
  - Permission-denied subdirectories now surface as `Warning` events
    (drawn from `jwalk::DirEntry::read_children_error`) and the scan
    continues; previously they were silently skipped.
- `anyhow` and `thiserror` for structured error handling.
- `CHANGELOG.md` (this file) following the Keep a Changelog format.
- `ARCHITECTURE.md` describing the current module layout, data flow, and
  invariants — kept up to date as the codebase evolves.
- Configurable scanner recursion ceiling (`ScanConfig::max_depth` now defaults
  to a safe cap) to prevent stack-overflow on pathological filesystems.
- Unit test covering the scanner's behaviour when a sub-directory is
  permission-denied — the scan must continue and report a partial result rather
  than aborting.

### Changed
- `App::start_scan` no longer performs a synchronous walk. It now spawns
  the streaming scanner and stores a `ScanHandle`; `update_scan_progress`
  drains the event channel each frame and installs the completed arena
  the moment it arrives.
- The status bar in `Scanning` mode shows a truncated trailing path of
  the entry the walker most recently processed, so users can tell the
  scan is alive even on very large trees.
- The legacy synchronous walker (`scan_directory` / `scan_recursive`) is
  retained as the test oracle and an emergency fallback, but it is no
  longer wired into the App. Marked `#[allow(dead_code)]` with a doc
  comment that points readers at `scanner_streaming::scan_streaming`.
- `main` now returns `anyhow::Result<()>` and uses `.context()` to attach
  human-readable messages to setup failures (terminal init, path canonicalize,
  etc.).
- Delete confirmation now defaults to **No** (previously **Yes**) so a stray
  `Enter` keypress can no longer trigger an irreversible deletion.
- Replaced unchecked `unwrap()` calls on `Option`-typed application state
  (`App::arena`, `App::radial_map`) with explicit `if let` / early-return
  guards so an out-of-mode access can no longer panic the UI.
- Removed unused `walkdir` dependency.

### Removed
- `scanner::scan_with_progress` and the recursive worker behind it. The
  threaded variant of the legacy walker had no remaining production
  callers once the streaming scanner shipped, and the corresponding
  `test_scan_progress_reporting` test moved to coverage of
  `scanner_streaming` instead.

### Fixed
- Scanner thread no longer panics when the arena root is missing on an empty
  scan — the error is surfaced to the status bar instead.

[Unreleased]: https://github.com/moxer-mmh/radisk/compare/master...HEAD
