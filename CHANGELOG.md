# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added (Phase 7 — diff subcommand + polish)
- **`radisk diff A B` subcommand** compares two snapshots and prints
  the folder-level differences to stdout, sorted by absolute size
  delta descending. Marker glyphs `~`/`+`/`-` for changed/added/
  removed; signed sizes; full `from -> to` so the output is
  immediately scannable. The subcommand is opt-in: existing
  `radisk PATH` invocations continue to work without changes.
- New `diff` module exposes `folder_diff(a, b) -> Vec<DiffEntry>`
  and `format_diff(&entries) -> String` as pure functions, so the
  output can be reused by future tooling (e.g. a GUI viewer or a
  CI bot that watches a tree's growth).
- 6 unit tests cover identical-tree empty diff, added-folder
  detection, signed delta on a growing folder, sort-order
  invariant, formatted-output sanity, and the empty-input message.
- README rewritten to document the streaming scanner benchmarks,
  every Phase 1–7 feature, and the `--export` / `--import` /
  `diff` workflow with a worked example. Links out to
  `ARCHITECTURE.md`, `docs/SNAPSHOT_FORMAT.md`, and
  `docs/config.example.toml` instead of duplicating their content.
- CI gains a `clippy` job (`cargo clippy --all-targets -D warnings`)
  alongside the existing test/fmt/docs jobs. Phase-1 cleanup means
  this gate is already green; CI will keep it that way.

### Added (Phase 6 — trash + snapshot export/import)
- **Trash-aware deletes**. New `delete` module detects `trash-put`
  (trash-cli) or `gio trash` at startup and routes deletes through
  the first one found, falling back to a permanent
  `std::fs::remove_*` only when neither is installed. The status bar
  reports which strategy was used (`Deleted (trash-put): …` vs
  `Deleted (permanent (no trash helper)): …`) so users always know
  whether the action is recoverable.
- The delete path now wraps `symlink_metadata` + an optional inode
  TOCTOU check, so a path that has been swapped out from under the
  user since the dialog opened is refused with a contextful error
  instead of being silently followed.
- **Snapshot export** via `--export PATH`. Runs the streaming scan
  headlessly (no TUI), prints progress to stderr, and writes a
  versioned `.radisk` file: 4-byte magic `RDSK`, 2-byte LE version,
  zstd-compressed postcard arena. ~650× compression in practice
  (1,846 files / 24 MiB on disk → 38 KiB snapshot in our smoke run).
- **Snapshot import** via `--import PATH`. Skips the scan entirely
  and opens the TUI on the loaded arena. Mutually exclusive with
  `--export`. Useful for inspecting a tree on a machine without
  filesystem access to the original target.
- New `snapshot` module owns the wire format — magic + version
  prefix means future radisk versions can refuse old snapshots with
  a "upgrade radisk" error rather than misinterpreting them. Format
  is documented in `docs/SNAPSHOT_FORMAT.md`.
- 4 round-trip tests cover the happy path, missing-magic rejection
  (with path in message), unknown-version rejection (with "upgrade
  radisk" hint), and contextful errors when the destination is
  unwritable.

### Added (Phase 5 — ncdu-parity features)
- **Sort modes**: cycle the sidebar / tree-view ordering with the
  `cycle_sort` action (default chord: `Shift+S`). Available modes:
  size descending (default), size ascending, name (case-insensitive
  ASCII alphabetical). The radial layout always remains size-driven.
  - New `SortMode` enum + `TreeArena::folder_items_sorted` extends
    the existing `folder_items` API without breaking callers.
  - `App.sort_mode` is updated by the action; the status bar shows
    the current label after each cycle.
- **Apparent vs on-disk size toggle**: `toggle_apparent_size` action
  (default chord: `a`, matching ncdu) flips between `metadata.len()`
  ("apparent" — what `ls -l` shows) and `st_blocks * 512` ("on-disk"
  — what `du` shows). Re-runs the streaming scan automatically; the
  status bar reflects the new mode while the rescan is in progress.
- **Exclude patterns** (gitignore-style globs):
  - New `--exclude PATTERN` CLI flag (repeatable). Adds to, never
    replaces, `[scan].exclude` in the config file.
  - Patterns are matched against both the full path and the base
    name, so `--exclude node_modules` and `--exclude '**/target/**'`
    both work.
  - Invalid patterns produce a `ScanEvent::Warning` and the rest of
    the patterns continue to apply — a single bad glob never aborts
    a scan.
  - Built on `globset` (a sub-crate of ripgrep's `ignore`) — fast
    enough to run in the per-entry hot path.
- `[scan].use_apparent_size` and `[scan].exclude` config keys join
  the existing `follow_symlinks` / `max_depth` so both can be
  persisted in the user's TOML file.
- `docs/config.example.toml` documents the new sections, the two
  new actions, and the chord defaults.

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
