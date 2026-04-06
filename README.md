# RaDisk

> Terminal-based radial disk usage visualizer inspired by KDE FileLight

[![Build Status](https://github.com/mimobn/radisk/workflows/ci/badge.svg)](https://github.com/mimobn/radisk/actions) [![Crates.io](https://img.shields.io/crates/v/radisk.svg)](https://crates.io/crates/radisk) [![AUR](https://img.shields.io/aur/version/radisk-bin.svg)](https://aur.archlinux.org/packages/radisk-bin) [![License: GPL-3.0](https://img.shields.io/badge/License-GPL%203.0-blue.svg)](LICENSE)
[![Buy me a coffee](https://img.shields.io/badge/☕-Buy%20me%20a%20coffee-FF5E5B?style=for-the-badge)](https://ko-fi.com/mimobn_)
## What is RaDisk?

RaDisk is a terminal-based disk usage analyzer that visualizes your filesystem as an interactive radial map, similar to KDE FileLight. It provides a beautiful, color-coded view of disk space usage with full mouse and keyboard support.

## Screenshots

![RaDisk in foot terminal](assets/radisk-foot.png)

![RaDisk in Kitty](assets/radisk-kitty.png)

![RaDisk in Konsole](assets/radisk-Konsole.png)

## Quick links

* [Usage](#usage)
* [Keyboard Shortcuts](#keyboard-shortcuts)
* [Building](#building)
* [Running tests](#running-tests)
* [Support](#support)


## Installation
> [!IMPORTANT]
> This app has only been tested on Arch Linux.
> If you are using another distribution, please report any issues you encounter.

The binary name for RaDisk is `radisk`.

**[Archives of precompiled binaries for RaDisk are available for Windows, macOS and Linux.](https://github.com/mimobn/radisk/releases)**

### Arch Linux (AUR)

```
$ yay -S radisk
```

### macOS (Homebrew)

```
$ brew install https://raw.githubusercontent.com/mimobn/radisk/main/homebrew/radisk.rb
```

### Rust (crates.io)

```
$ cargo install radisk
```

### Build from Source

```
$ git clone https://github.com/mimobn/radisk.git
$ cd radisk
$ cargo build --release
$ ./target/release/radisk
```

## Usage

```
$ radisk                 # Scan current directory
$ radisk /home/user      # Scan a specific directory
$ radisk -d 6 /var       # Scan with 6 ring levels
$ radisk --help          # Show help and options
```

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `q` / `Esc` | Quit |
| `u` / `Backspace` | Go to parent directory |
| `Enter` | Open selected folder |
| `d` | Delete selected item |
| `+` / `-` | Zoom in/out |
| `r` | Rescan directory |
| `Tab` | Toggle focus (map/sidebar) |
| `j` / `k` | Navigate up/down in sidebar |
| `?` | Show help |

### Mouse Controls

| Action | Description |
|--------|-------------|
| Left click | Open folder / Navigate |
| Right click | Context menu |
| Scroll | Zoom in/out |
| Hover | Highlight segment / Sync sidebar |

## Building

RaDisk is written in Rust, so you'll need to grab a [Rust installation](https://www.rust-lang.org/) in order to compile it. RaDisk compiles with Rust 1.85.0 (stable) or newer.

To build RaDisk:

```
$ git clone https://github.com/mimobn/radisk
$ cd radisk
$ cargo build --release
$ ./target/release/radisk --version
0.1.0
```

## Running tests

RaDisk is relatively well-tested, including both unit tests and integration tests. To run the full test suite, use:

```
$ cargo test --all
```

from the repository root.

## Support

If you find RaDisk useful, consider supporting its development:

[![Buy me a coffee](https://img.shields.io/badge/☕-Buy%20me%20a%20coffee-FF5E5B?style=for-the-badge)](https://ko-fi.com/mimobn_)

## License

GPL-3.0-or-later — See [LICENSE](LICENSE) for details.
