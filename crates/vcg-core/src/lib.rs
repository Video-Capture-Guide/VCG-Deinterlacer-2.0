//! Core logic for the VCG Deinterlacer: configuration, color handling, and
//! the FFmpeg/VapourSynth command construction.
//!
//! Deliberately free of any GUI or process-spawning concerns so that every
//! decision in here is a pure function that can be diffed against the Python
//! implementation it was ported from.

pub mod analysis;
pub mod colorspace;
pub mod config;
pub mod ffmpeg;
pub mod provenance;
pub mod vpy;
pub mod watermark;

pub use config::Config;

/// The application version, as it appears in filenames, metadata and logs.
///
/// Here rather than in the GUI because the encoder stamps it into every file
/// it writes, and a version number that lives in the interface layer cannot
/// be reached from the layer that actually does the writing.
pub const VERSION: &str = "2.0.5";

/// The date this version was built, ISO first.
///
/// Updated by hand with `VERSION`, not generated at compile time: a date that
/// changes on every rebuild makes two binaries of the same version claim to
/// be different things, and the log a user sends back is then harder to place
/// rather than easier.
pub const BUILD_DATE: &str = "2026-10-29";

/// `VERSION (BUILD_DATE)`, the form that goes in the generated script's
/// header and on the About page.
///
/// Spelled out rather than built from the two above, because `concat!` cannot
/// take a `const`. `version_string_agrees_with_its_parts` fails the build if
/// the three ever drift apart.
pub const VERSION_STRING: &str = "2.0.5 (2026-10-29)";

/// Where releases are published, for the update check and the About page.
pub const REPO_URL: &str = "https://github.com/Video-Capture-Guide/VCG-Deinterlacer-2.0";

pub const AUTHOR_HANDLE: &str = "@VideoCaptureGuide";

/// The author's site, linked from the front page and the About panel.
pub const AUTHOR_URL: &str = "https://videocaptureguide.com/";
pub const AUTHOR_NAME: &str = "VideoCaptureGuide";

#[cfg(test)]
mod version_tests {
    use super::*;

    /// `VERSION_STRING` repeats the other two by hand, so something has to
    /// notice when one of the three is bumped and the others are not.
    #[test]
    fn version_string_agrees_with_its_parts() {
        assert_eq!(VERSION_STRING, format!("{VERSION} ({BUILD_DATE})"));
    }

    /// A build date that is not ISO sorts wrong everywhere it is listed.
    #[test]
    fn the_build_date_is_iso() {
        let parts: Vec<&str> = BUILD_DATE.split('-').collect();
        assert_eq!(parts.len(), 3, "{BUILD_DATE}");
        assert_eq!(parts[0].len(), 4, "four-digit year: {BUILD_DATE}");
        assert!(
            parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())),
            "{BUILD_DATE}"
        );
    }
}
