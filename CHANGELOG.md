# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
- `main` now returns `anyhow::Result<()>` and uses `.context()` to attach
  human-readable messages to setup failures (terminal init, path canonicalize,
  etc.).
- Delete confirmation now defaults to **No** (previously **Yes**) so a stray
  `Enter` keypress can no longer trigger an irreversible deletion.
- Replaced unchecked `unwrap()` calls on `Option`-typed application state
  (`App::arena`, `App::radial_map`) with explicit `if let` / early-return
  guards so an out-of-mode access can no longer panic the UI.
- Removed unused `walkdir` dependency.

### Fixed
- Scanner thread no longer panics when the arena root is missing on an empty
  scan — the error is surfaced to the status bar instead.

[Unreleased]: https://github.com/moxer-mmh/radisk/compare/master...HEAD
