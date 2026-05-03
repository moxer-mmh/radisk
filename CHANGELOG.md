# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
