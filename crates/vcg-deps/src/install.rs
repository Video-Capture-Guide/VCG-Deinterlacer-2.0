//! Downloading, verifying and installing the dependency payload.
//!
//! Two things the Python's `FirstRunSetupWindow` gets wrong, both fixed here:
//!
//! 1. **Nothing is verified.** It downloads via PowerShell (falling back to
//!    `curl.exe`) and the only check is `if size_mb < 1.0` — "did at least a
//!    megabyte arrive". A truncated, corrupted or substituted 135 MB archive
//!    passes. Here the exact size and SHA-256 from the compiled-in manifest
//!    must both match, and the hash is computed while streaming so the archive
//!    is never held in memory.
//!
//! 2. **It destroys the working install before it has a replacement.**
//!    `_extract` calls `shutil.rmtree(DEPS_DIR)` *first*, then extracts. A
//!    failure at any point after that — full disk, corrupt archive, closed lid
//!    — leaves the user with no `_deps` at all, having started with a working
//!    one. Here everything happens in a staging directory and the existing
//!    install is only moved aside once a complete, verified payload is ready to
//!    take its place. If the swap itself fails, the original is put back.
//!
//! The download also no longer spawns a shell. An unsigned binary that starts
//! PowerShell to fetch an archive is a close match for what heuristic
//! antivirus engines look for.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::manifest::Manifest;

/// Progress during an install.
#[derive(Debug, Clone, Copy)]
pub enum Phase {
    /// Bytes received so far, and the total expected.
    Downloading { received: u64, total: u64 },
    Verifying,
    /// Entries extracted so far, and the total.
    Extracting { done: usize, total: usize },
    /// Swapping the new payload into place.
    Installing,
}

/// Callback for install progress.
pub type Progress<'a> = &'a mut dyn FnMut(Phase);

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The server did not return 200, or the request failed.
    Http(String),
    /// The archive was not the size the manifest says.
    SizeMismatch { expected: u64, got: u64 },
    /// The archive did not hash to the manifest's digest.
    DigestMismatch { expected: String, got: String },
    /// The archive did not contain the expected layout.
    BadArchive(String),
    /// An entry tried to escape the extraction directory.
    UnsafeEntry(String),
    Zip(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Http(m) => write!(f, "download failed: {m}"),
            Error::SizeMismatch { expected, got } => write!(
                f,
                "archive is {got} bytes, expected {expected} — the download is \
                 incomplete or the release was changed"
            ),
            Error::DigestMismatch { expected, got } => write!(
                f,
                "archive SHA-256 is {got}, expected {expected} — the download is \
                 corrupt or not the expected file; nothing was installed"
            ),
            Error::BadArchive(m) => write!(f, "archive is not what was expected: {m}"),
            Error::UnsafeEntry(m) => {
                write!(f, "archive contains an entry that escapes the target: {m}")
            }
            Error::Zip(m) => write!(f, "could not read the archive: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

type Result<T> = std::result::Result<T, Error>;

/// Install the payload into `deps_dir`, replacing whatever is there.
///
/// `deps_dir` itself is never written to until the final swap, so an
/// interrupted install leaves the existing payload untouched.
pub fn install(
    deps_dir: &Path,
    manifest: &Manifest,
    progress: Option<Progress<'_>>,
) -> Result<()> {
    // Normalize the optional callback once, so everything below takes a plain
    // `&mut dyn FnMut` and needs no per-call reborrowing.
    let mut discard = |_: Phase| {};
    let report: &mut dyn FnMut(Phase) = match progress {
        Some(p) => p,
        None => &mut discard,
    };

    let parent = deps_dir.parent().unwrap_or(Path::new("."));
    let staging = parent.join(".vcg-deps-staging");

    // A staging directory left by a previous failed run is stale by
    // definition — the download is not resumable.
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;

    let result = (|| {
        let archive = staging.join("payload.zip");
        download_and_verify(&archive, manifest, report)?;
        unpack_and_promote(&staging, &archive, deps_dir, manifest, report)
    })();

    // Always clean up, success or failure, so a failed attempt does not leave
    // 135 MB behind.
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Install from an archive already on disk.
///
/// The archive is still verified against the manifest's size and digest, so a
/// hand-supplied file gets the same guarantees as a download. Exposed mainly so
/// the extract-verify-promote chain can be tested without the network.
pub fn install_from_archive(
    archive: &Path,
    deps_dir: &Path,
    manifest: &Manifest,
    progress: Option<Progress<'_>>,
) -> Result<()> {
    let mut discard = |_: Phase| {};
    let report: &mut dyn FnMut(Phase) = match progress {
        Some(p) => p,
        None => &mut discard,
    };

    verify_archive_file(archive, manifest, report)?;

    let parent = deps_dir.parent().unwrap_or(Path::new("."));
    let staging = parent.join(".vcg-deps-staging");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;

    let result = unpack_and_promote(&staging, archive, deps_dir, manifest, report);
    let _ = fs::remove_dir_all(&staging);
    result
}

fn unpack_and_promote(
    staging: &Path,
    archive: &Path,
    deps_dir: &Path,
    manifest: &Manifest,
    report: &mut dyn FnMut(Phase),
) -> Result<()> {
    let unpacked = staging.join("unpacked");
    fs::create_dir_all(&unpacked)?;
    extract(archive, &unpacked, report)?;

    // The archive unpacks into its own directory, which becomes `_deps`.
    let root = locate_root(&unpacked, manifest)?;
    verify_layout(&root, manifest)?;

    report(Phase::Installing);
    promote(&root, deps_dir)?;
    Ok(())
}

/// Stream the archive to disk, hashing as it arrives, then check both the size
/// and the digest before anything is extracted.
fn download_and_verify(
    dest: &Path,
    manifest: &Manifest,
    report: &mut dyn FnMut(Phase),
) -> Result<()> {
    let response = ureq::get(manifest.url)
        .call()
        .map_err(|e| Error::Http(e.to_string()))?;

    let status = response.status();
    if status != 200 {
        return Err(Error::Http(format!("server returned HTTP {status}")));
    }

    // `into_reader` is the unlimited, streaming path — the convenience methods
    // cap at 10 MB. The manifest's exact size is a tighter bound anyway, and
    // the reader errors if the connection closes before Content-Length is met.
    let mut body = response.into_body().into_reader();
    let mut file = File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut received: u64 = 0;

    loop {
        let n = body.read(&mut buf).map_err(|e| Error::Http(e.to_string()))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        // Refuse to keep writing a response that is already too big, rather
        // than filling the disk and failing at the end.
        if received > manifest.size {
            return Err(Error::SizeMismatch {
                expected: manifest.size,
                got: received,
            });
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        report(Phase::Downloading {
            received,
            total: manifest.size,
        });
    }
    file.flush()?;
    drop(file);

    if received != manifest.size {
        return Err(Error::SizeMismatch {
            expected: manifest.size,
            got: received,
        });
    }

    report(Phase::Verifying);
    let got = hex(&hasher.finalize());
    if got != manifest.sha256 {
        return Err(Error::DigestMismatch {
            expected: manifest.sha256.to_string(),
            got,
        });
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Check an archive already on disk against the manifest.
///
/// Streamed rather than read whole, for the same reason the download is: a
/// 135 MB buffer is not needed to compute a digest.
fn verify_archive_file(
    archive: &Path,
    manifest: &Manifest,
    report: &mut dyn FnMut(Phase),
) -> Result<()> {
    let size = fs::metadata(archive)?.len();
    if size != manifest.size {
        return Err(Error::SizeMismatch {
            expected: manifest.size,
            got: size,
        });
    }

    report(Phase::Verifying);
    let mut file = File::open(archive)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let got = hex(&hasher.finalize());
    if got != manifest.sha256 {
        return Err(Error::DigestMismatch {
            expected: manifest.sha256.to_string(),
            got,
        });
    }
    Ok(())
}

/// Extract every entry, refusing any that escapes `target`.
fn extract(archive: &Path, target: &Path, report: &mut dyn FnMut(Phase)) -> Result<()> {
    let file = File::open(archive)?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| Error::Zip(e.to_string()))?;
    let total = zip.len();

    for i in 0..total {
        let mut entry = zip.by_index(i).map_err(|e| Error::Zip(e.to_string()))?;

        // `enclosed_name` is the zip-slip guard: it returns None for an
        // absolute path or any `..` that would climb out of the target. An
        // archive is untrusted input even when we published it.
        let Some(rel) = entry.enclosed_name() else {
            return Err(Error::UnsafeEntry(entry.name().to_string()));
        };
        let out = target.join(rel);

        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(dir) = out.parent() {
                fs::create_dir_all(dir)?;
            }
            let mut dest = File::create(&out)?;
            io::copy(&mut entry, &mut dest)?;
        }

        report(Phase::Extracting { done: i + 1, total });
    }
    Ok(())
}

/// Find the directory inside the extraction that becomes `_deps`.
///
/// Normally `vcg-deps-v<N>` per the manifest. An archive packed without that
/// wrapper is accepted too, as long as the version marker is at its top level.
fn locate_root(unpacked: &Path, manifest: &Manifest) -> Result<PathBuf> {
    let named = unpacked.join(manifest.archive_root);
    if named.is_dir() {
        return Ok(named);
    }
    if unpacked.join("vcg_deps.version").is_file() {
        return Ok(unpacked.to_path_buf());
    }

    // Exactly one directory at the top level is unambiguous enough to use.
    let dirs: Vec<PathBuf> = fs::read_dir(unpacked)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    if dirs.len() == 1 {
        return Ok(dirs.into_iter().next().expect("length checked"));
    }

    Err(Error::BadArchive(format!(
        "expected a '{}' directory; found {} top-level directories",
        manifest.archive_root,
        dirs.len()
    )))
}

/// Check the payload is complete *before* it replaces a working install.
fn verify_layout(root: &Path, manifest: &Manifest) -> Result<()> {
    let missing: Vec<&str> = manifest
        .required_entries()
        .iter()
        .copied()
        .filter(|rel| {
            let p = rel.split('/').fold(root.to_path_buf(), |acc, part| acc.join(part));
            !p.exists()
        })
        .collect();

    if !missing.is_empty() {
        return Err(Error::BadArchive(format!("missing {}", missing.join(", "))));
    }

    let version = fs::read_to_string(root.join("vcg_deps.version"))
        .map_err(|e| Error::BadArchive(format!("version marker unreadable: {e}")))?;
    if version.trim() != manifest.version {
        return Err(Error::BadArchive(format!(
            "archive declares version {:?}, expected {:?}",
            version.trim(),
            manifest.version
        )));
    }
    Ok(())
}

/// Move the verified payload into place, keeping the old one until the new one
/// has landed.
///
/// The Python deletes first and extracts second, so any failure in between
/// loses a working installation. Here the worst case is that the original is
/// restored.
fn promote(new_root: &Path, deps_dir: &Path) -> Result<()> {
    let backup = deps_dir.with_file_name(format!(
        "{}.previous",
        deps_dir.file_name().unwrap_or_default().to_string_lossy()
    ));
    let _ = fs::remove_dir_all(&backup);

    let had_existing = deps_dir.exists();
    if had_existing {
        fs::rename(deps_dir, &backup)?;
    }

    match fs::rename(new_root, deps_dir) {
        Ok(()) => {
            // Only now is the old payload expendable.
            let _ = fs::remove_dir_all(&backup);
            Ok(())
        }
        Err(e) => {
            // Put the working install back before reporting the failure.
            if had_existing {
                let _ = fs::rename(&backup, deps_dir);
            }
            Err(Error::Io(e))
        }
    }
}
