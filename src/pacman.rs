//! Optional Arch-Linux package-ownership lookup.
//!
//! Inspired by Revo Uninstaller's "leftovers detection" — answers
//! "which package put this file here, and is the package even
//! installed any more?" for a hovered/selected entry. Useful for
//! identifying genuine orphans (files left over from an uninstalled
//! app) vs packaged files that should not be deleted by hand.
//!
//! ## Implementation
//!
//! We shell out to `pacman -Qo <path>` lazily — once per unique
//! path, cached for the lifetime of the App. Parsing pacman's local
//! db directly would be faster on a "annotate every row" pass, but
//! Phase 13 only annotates on demand (the user presses `o` on a
//! row), so the per-row cost dominates and shelling out is simpler.
//!
//! ## Output shape
//!
//! `pacman -Qo /usr/bin/ls` prints
//!     `/usr/bin/ls is owned by coreutils 9.7-1`
//! and exits 0. For an unowned path it prints to stderr and exits
//! non-zero. We map the two to:
//! - `Owner::Package(name, version)` — owned by an installed package
//! - `Owner::None` — unowned (orphan, user-created file, …)
//! - `Owner::PacmanUnavailable` — `pacman` binary not on PATH; we
//!   only check once and short-circuit subsequent queries.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Result of an ownership lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// File belongs to an installed pacman package.
    Package { name: String, version: String },
    /// pacman knows about no owner — an orphan, user-created, or
    /// non-package file.
    None,
    /// `pacman` is not available on this host (not Arch, or not in
    /// PATH). Reported once so the App can show a single
    /// "pacman not available" status instead of querying forever.
    PacmanUnavailable,
}

impl Owner {
    /// Short label for the status bar.
    pub fn label(&self) -> String {
        match self {
            Owner::Package { name, version } => format!("{} {}", name, version),
            Owner::None => "(no package owns this path)".to_string(),
            Owner::PacmanUnavailable => "pacman not available".to_string(),
        }
    }
}

/// Returns true if `pacman` is on the user's PATH. Cached after
/// the first probe.
pub fn pacman_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let Some(paths) = std::env::var_os("PATH") else {
            return false;
        };
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("pacman");
            if candidate.is_file() {
                return true;
            }
        }
        false
    })
}

/// Query the owner of a single path. Synchronous; intended for
/// on-demand lookup driven by a keybind, not for bulk annotation.
pub fn query(path: &Path) -> Owner {
    if !pacman_available() {
        return Owner::PacmanUnavailable;
    }
    let output = match Command::new("pacman")
        .arg("-Qo")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(o) => o,
        Err(_) => return Owner::PacmanUnavailable,
    };
    if !output.status.success() {
        // pacman -Qo exits non-zero for unowned paths.
        return Owner::None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_owner_line(&stdout).unwrap_or(Owner::None)
}

/// Parse a single line of `pacman -Qo` output into an [`Owner`].
/// Public for testing without needing pacman installed.
///
/// Example input:
///     `/usr/bin/ls is owned by coreutils 9.7-1`
pub fn parse_owner_line(text: &str) -> Option<Owner> {
    let line = text.lines().next()?.trim();
    // Anchor on the literal phrase " is owned by " — pacman's
    // output is locale-aware so this could in theory differ, but
    // every reported package translation we've seen keeps this
    // exact English phrase.
    let (_, after) = line.split_once(" is owned by ")?;
    let mut parts = after.split_whitespace();
    let name = parts.next()?.to_string();
    let version = parts.next().unwrap_or("?").to_string();
    Some(Owner::Package { name, version })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_pacman_output() {
        let line = "/usr/bin/ls is owned by coreutils 9.7-1";
        let o = parse_owner_line(line).unwrap();
        assert_eq!(
            o,
            Owner::Package {
                name: "coreutils".to_string(),
                version: "9.7-1".to_string(),
            }
        );
    }

    #[test]
    fn handles_path_with_spaces() {
        // pacman quotes paths with spaces in `is owned by` line.
        let line = "/usr/share/My Folder/file is owned by my-pkg 1.0-1";
        let o = parse_owner_line(line).unwrap();
        assert!(matches!(o, Owner::Package { name, .. } if name == "my-pkg"));
    }

    #[test]
    fn rejects_lines_without_anchor() {
        assert!(parse_owner_line("error: no package owns this").is_none());
        assert!(parse_owner_line("").is_none());
    }

    #[test]
    fn label_text_is_human_readable() {
        let p = Owner::Package {
            name: "coreutils".into(),
            version: "9.7-1".into(),
        };
        assert_eq!(p.label(), "coreutils 9.7-1");
        assert!(Owner::None.label().contains("no package"));
        assert!(Owner::PacmanUnavailable.label().contains("pacman"));
    }
}
