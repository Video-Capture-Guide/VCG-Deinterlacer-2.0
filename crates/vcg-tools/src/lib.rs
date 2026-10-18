//! The side of the port that touches the operating system: finding the
//! bundled tools and running them.
//!
//! `vcg-core` is deliberately pure so it can be diffed against the Python.
//! Everything that spawns a process or reads the filesystem lives here, which
//! keeps that boundary enforceable rather than aspirational.

pub mod apply;
pub mod comparison;
pub mod edges;
pub mod encode;
pub mod paths;
pub mod preview;
pub mod probe;
pub mod run;

/// The font file a text watermark should be drawn with.
///
/// Looked up once here rather than inside the parity-verified argument
/// builder, which takes it as a parameter precisely so it stays a pure
/// function.
pub fn watermark_font() -> Option<std::path::PathBuf> {
    watermark::find_system_font()
}

/// Re-exported so callers do not need to reach into vcg-core for it.
pub use vcg_core::watermark;
