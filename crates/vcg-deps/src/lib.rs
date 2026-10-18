//! Fetching and installing the portable dependency payload.
//!
//! Separate from `vcg-tools` so that its HTTP, TLS and zip dependencies stay
//! out of the binaries that only need to *run* the tools — the probe CLI is
//! 0.38 MB and should remain so.

pub mod install;
pub mod manifest;
pub mod state;
pub mod update;
