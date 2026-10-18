//! Color-matrix resolution and FFmpeg color tagging.
//!
//! Ported from `_resolve_cs_tag`, `_resolve_trc_tag`, `_cs_tag_args`,
//! `_setparams_filter_str`, `_merge_video_filter`, `_hd_upscale_height`,
//! `_needs_601_to_709` and `_effective_output_matrix`.

use crate::config::{ColorMatrix, Config, VideoFormat};

/// An FFmpeg colorspace tag. Named rather than stringly-typed because the
/// encoder-option spelling and the filter-table spelling differ for PAL —
/// a distinction the Python code documents in a comment and gets right only
/// by having two separate helper functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsTag {
    Bt709,
    /// PAL BT.601.
    Bt470bg,
    /// NTSC BT.601.
    Smpte170m,
}

impl CsTag {
    /// Spelling for `-colorspace` / `-color_primaries`.
    pub fn name(self) -> &'static str {
        match self {
            CsTag::Bt709 => "bt709",
            CsTag::Bt470bg => "bt470bg",
            CsTag::Smpte170m => "smpte170m",
        }
    }

    /// Spelling for `-color_trc`.
    ///
    /// `bt470bg` is only valid for `-colorspace`/`-color_primaries`; the
    /// matching transfer characteristic (5, gamma 2.8) is named `gamma28`.
    /// Passing `bt470bg` to `-color_trc` makes FFmpeg fail with "Undefined
    /// constant" and abort the encode.
    pub fn trc_name(self) -> &'static str {
        match self {
            CsTag::Bt470bg => "gamma28",
            other => other.name(),
        }
    }

    /// Spelling inside the `setparams` filter, where PAL's transfer is
    /// spelled `bt470bg`, not `gamma28`.
    pub fn filter_name(self) -> &'static str {
        self.name()
    }

    /// FFmpeg color-tagging args for the output stream.
    pub fn tag_args(self) -> Vec<String> {
        vec![
            "-colorspace".into(),
            self.name().into(),
            "-color_primaries".into(),
            self.name().into(),
            "-color_trc".into(),
            self.trc_name().into(),
        ]
    }

    /// A `setparams` filter stamping frame-level color properties.
    ///
    /// The y4m pipe from vspipe drops the `_Primaries`/`_Transfer` frame props
    /// set in the .vpy, and with FFmpeg 8 the frame-level values (unknown)
    /// override the `-color_primaries`/`-color_trc` encoder options, leaving
    /// the output untagged. `setparams` re-stamps them so they survive to the
    /// encoder.
    pub fn setparams_filter(self) -> String {
        let n = self.filter_name();
        format!("setparams=range=tv:colorspace={n}:color_primaries={n}:color_trc={n}")
    }
}

/// The FFmpeg colorspace tag for a given matrix + video format.
pub fn resolve_cs_tag(matrix: ColorMatrix, format: VideoFormat) -> CsTag {
    match (matrix, format) {
        (ColorMatrix::Bt709, _) => CsTag::Bt709,
        (ColorMatrix::Bt601, VideoFormat::Pal) => CsTag::Bt470bg,
        (ColorMatrix::Bt601, VideoFormat::Ntsc) => CsTag::Smpte170m,
    }
}

/// Output frame height produced by the nnedi3 upscale, or `None` if no upscale.
///
/// Mirrors the upscale sizing in the .vpy generator.
pub fn hd_upscale_height(config: &Config) -> Option<u32> {
    if !config.upscale_enabled {
        return None;
    }
    let res = config.upscale_resolution();
    // A malformed "WxH" degrades to the per-format default rather than failing,
    // matching the Python except-clause.
    let fallback = match config.format {
        VideoFormat::Ntsc => 720,
        VideoFormat::Pal => 768,
    };
    Some(
        res.split('x')
            .nth(1)
            .and_then(|h| h.parse::<u32>().ok())
            .unwrap_or(fallback),
    )
}

/// True when an SD BT.601 source is upscaled to HD (final height >= 720).
///
/// HD-class platforms (YouTube) interpret HD-sized frames as BT.709, so an
/// upscaled SD (601) capture must have its *pixels* converted — not just its
/// color tag changed — or it displays dark and oversaturated. Only this case
/// needs the conversion: a real HD source is already 709, an SD-sized output
/// stays 601, and a source the user explicitly tagged 709 is treated as 709.
pub fn needs_601_to_709(config: &Config) -> bool {
    if config.color_matrix == ColorMatrix::Bt709 {
        return false;
    }
    if config.is_hd_source() {
        return false;
    }
    hd_upscale_height(config).is_some_and(|h| h >= 720)
}

/// The color matrix the output pixels are actually in after the pipeline runs.
///
/// Same as the user-selected matrix, except an SD source upscaled to HD is
/// converted 601 -> 709 during filtering, so its frame-prop tags and FFmpeg
/// color flags must read bt709 to match the converted pixels.
pub fn effective_output_matrix(config: &Config) -> ColorMatrix {
    if needs_601_to_709(config) {
        ColorMatrix::Bt709
    } else {
        config.color_matrix
    }
}

/// The colorspace tag to stamp on the output.
pub fn output_cs_tag(config: &Config) -> CsTag {
    resolve_cs_tag(effective_output_matrix(config), config.format)
}
