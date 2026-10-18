//! The pinned description of the dependency payload.
//!
//! Compiled into the binary rather than fetched, so a given build can only
//! ever install the exact archive it was released against. The Python derives
//! its URL from a version constant and verifies nothing about what arrives —
//! only that the file is larger than 1 MB.

/// Everything needed to fetch and trust one release of the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Manifest {
    /// The contents of `vcg_deps.version` inside the archive.
    pub version: &'static str,
    pub url: &'static str,
    /// Lowercase hex SHA-256 of the archive.
    pub sha256: &'static str,
    /// Exact archive size in bytes.
    pub size: u64,
    /// The directory the archive unpacks into, which is renamed to `_deps`.
    pub archive_root: &'static str,
}

/// The payload this build installs.
///
/// Published 2026-10-17. The digest is of the archive that was uploaded, and
/// was checked by installing from it before the release went out.
///
/// v11 over v10 adds what the two optional paths needed, so that neither is
/// a separate install any more:
///
/// * **vs-jetpack** and its dependencies — the `vs*` namespace packages,
///   `jetpytools` and `numpy` — plus the `resize2` and `zsmooth` plugins it
///   calls. Modern QTGMC used to tell the user to `pip install vsjetpack`
///   into whichever Python runs VapourSynth, which almost nobody completed.
/// * **`bm3dcpu`** (AVX2) and **`bm3dcuda`**, which are the same BM3D the
///   bundled plugin implements, at 4.0x and 7.9x its speed. The bundled
///   original stays as the fallback for pre-AVX2 machines.
///
/// `__pycache__`, `.lwi` indexes and `.pyc` files are stripped — 14 MB of
/// rebuildable cache that would otherwise ship and be regenerated anyway.
pub const CURRENT: Manifest = Manifest {
    version: "11",
    url: "https://github.com/Video-Capture-Guide/vcg-deinterlacer-deps\
          /releases/download/v11/vcg-deps-v11.zip",
    sha256: "ed77541e7bb0d4932e6210c666d890d1e408aa300b6bf157dfa57b7993aabf16",
    size: 165_246_426,
    archive_root: "vcg-deps-v11",
};

impl Manifest {
    /// Size in MiB, for progress reporting.
    pub fn size_mib(&self) -> f64 {
        self.size as f64 / 1_048_576.0
    }

    /// Files that must exist after a successful install.
    ///
    /// Checked before the new payload replaces the old one, so a truncated or
    /// wrong-shaped archive cannot take out a working installation.
    pub fn required_entries(&self) -> &'static [&'static str] {
        &[
            "vcg_deps.version",
            "ffmpeg/ffmpeg.exe",
            "ffmpeg/ffprobe.exe",
            "vs/vspipe.exe",
            "vs/plugins64",
        ]
    }
}
