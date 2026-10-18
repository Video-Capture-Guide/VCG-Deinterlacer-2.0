//! Source analysis: the probes that inspect a file and recommend settings.
//!
//! Every probe in the Python is one function that spawns ffmpeg or ffprobe,
//! parses its output, and classifies the result against hand-calibrated
//! thresholds. Here the spawning is left to the caller and only the parsing and
//! classification live in the core, so each threshold can be exercised directly
//! against captured output — see `tests/fixtures/` for real ffmpeg text.

pub mod audio;
pub mod classify;
pub mod edges;
pub mod halo;
pub mod histogram;
pub mod idet;
pub mod probe;
pub mod scopes;
pub mod signalstats;

/// Filter strength a probe recommends.
///
/// The probes share this vocabulary but not its meaning: the noise probe's
/// `Light` verdict recommends `Moderate` denoising, and the halo probe's
/// `Moderate` verdict recommends `Light` dehalo. Keeping verdict and
/// recommendation as separate values makes that explicit rather than leaving
/// two same-named strings to be confused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recommendation {
    None,
    Light,
    Moderate,
    Strong,
    Heavy,
}
