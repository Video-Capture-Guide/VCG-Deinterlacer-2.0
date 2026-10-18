//! What is currently installed.
//!
//! Ported from `deps_ready`, which answers only true or false. Here the
//! distinct failures are distinguishable, because they call for different
//! messages: "nothing installed yet" is a first run, "version 9 found" is an
//! upgrade, and "version file says 10 but ffmpeg.exe is missing" is a broken
//! install that the atomic installer is specifically meant to prevent.

use std::path::{Path, PathBuf};

use crate::manifest::Manifest;

/// The state of a `_deps` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Installed, correct version, every required file present.
    Ready,
    /// No directory, or no version marker at all.
    Missing,
    /// A different version is installed.
    VersionMismatch { found: String },
    /// The version marker matches but required files are absent — a partial
    /// extract, or someone deleted something.
    Incomplete { missing: Vec<String> },
}

impl State {
    /// Whether the pipeline can run as-is.
    pub fn is_ready(&self) -> bool {
        matches!(self, State::Ready)
    }

    /// A short description for a setup screen or log line.
    pub fn describe(&self, manifest: &Manifest) -> String {
        match self {
            State::Ready => format!("ready (v{})", manifest.version),
            State::Missing => "not installed".to_string(),
            State::VersionMismatch { found } => {
                format!("v{found} installed, v{} expected", manifest.version)
            }
            State::Incomplete { missing } => {
                format!("incomplete — missing {}", missing.join(", "))
            }
        }
    }
}

/// Read the installed version marker, if there is one.
pub fn installed_version(deps_dir: &Path) -> Option<String> {
    let marker = deps_dir.join("vcg_deps.version");
    let raw = std::fs::read_to_string(marker).ok()?;
    Some(raw.trim().to_string())
}

/// Inspect a `_deps` directory.
pub fn state(deps_dir: &Path, manifest: &Manifest) -> State {
    let Some(found) = installed_version(deps_dir) else {
        return State::Missing;
    };
    if found != manifest.version {
        return State::VersionMismatch { found };
    }

    // The marker is written last by the installer, so its presence should
    // imply the rest — but a hand-edited or half-deleted folder must still be
    // reported honestly rather than trusted.
    let missing: Vec<String> = manifest
        .required_entries()
        .iter()
        .filter(|rel| !exists_any(deps_dir, rel))
        .map(|rel| (*rel).to_string())
        .collect();

    if missing.is_empty() {
        State::Ready
    } else {
        State::Incomplete { missing }
    }
}

/// Whether a required entry exists, allowing for a file or a directory.
fn exists_any(root: &Path, rel: &str) -> bool {
    let p: PathBuf = rel.split('/').fold(root.to_path_buf(), |acc, part| acc.join(part));
    p.exists()
}
