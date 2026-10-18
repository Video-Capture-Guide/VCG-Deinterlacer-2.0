//! FFmpeg output arguments, frame rate and frame sizing.
//!
//! Ported from `get_ffmpeg_output_args`, `_output_frame_rate` and
//! `_estimate_output_width`.

use crate::colorspace::output_cs_tag;
use crate::config::{Config, CropPreset, FieldOrder, OutputFormat, VideoFormat};

/// A chosen encoder: the FFmpeg args and the container extension that must
/// accompany them. Returning these together makes it impossible to pair
/// ProRes args with an `.mkv`, which the Python version's parallel
/// `(args, ext)` tuples allowed by convention only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputArgs {
    pub args: Vec<String>,
    pub extension: &'static str,
}

/// Build the encoder args and container extension for the configured format.
pub fn output_args(config: &Config) -> OutputArgs {
    let cs = output_cs_tag(config).tag_args();

    let (codec, extension): (&[&str], &str) = match config.output_format {
        // vspipe outputs 10-bit (dithered by fmtc); -pix_fmt keeps the
        // encoder explicit.
        OutputFormat::Prores => (
            &["-c:v", "prores_ks", "-profile:v", "3", "-pix_fmt", "yuv422p10le"],
            ".mov",
        ),
        OutputFormat::H264 => (
            &[
                "-c:v", "libx264", "-crf", "18", "-preset", "slow",
                "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k",
            ],
            ".mp4",
        ),
        OutputFormat::Ffv1 => (
            &["-c:v", "ffv1", "-level", "3", "-coder", "1", "-slicecrc", "1"],
            ".mkv",
        ),
        OutputFormat::Huffyuv => (&["-c:v", "huffyuv", "-pix_fmt", "yuv422p"], ".avi"),
        OutputFormat::Utvideo => (&["-c:v", "utvideo", "-pix_fmt", "yuv422p"], ".avi"),
        OutputFormat::Lagarith => (&["-c:v", "lagarith", "-pix_fmt", "yuv420p"], ".avi"),
    };

    let mut args: Vec<String> = codec.iter().map(|s| (*s).to_string()).collect();
    args.extend(cs);
    OutputArgs { args, extension }
}

/// The output frame-rate string for `ffmpeg -r`.
///
/// Mirrors the frame-rate decisions made when generating the .vpy:
///   * progressive / single-rate QTGMC → source rate (29.97 or 25)
///   * double-rate "bob" QTGMC         → doubled rate (59.94 or 50)
///   * inverse telecine (film)         → native film rate (23.976 or 25)
///
/// Used to stamp the muxed output so audio and video stay in sync and playback
/// runs at the correct speed.
pub fn output_frame_rate(config: &Config) -> &'static str {
    let pal = config.format.is_pal();

    // Inverse telecine restores the native progressive film rate.
    if config.ivtc_mode {
        return if pal { "25" } else { "24000/1001" };
    }
    // No frame-rate change — carry the source rate through.
    if config.field_order == FieldOrder::Progressive {
        return if pal { "25" } else { "30000/1001" };
    }
    // Double-rate bob: one output frame per interlaced field.
    if config.double_rate {
        return if pal { "50" } else { "60000/1001" };
    }
    // Single-rate QTGMC: one output frame per interlaced frame.
    if pal { "25" } else { "30000/1001" }
}

/// Best-effort output frame width, mirroring the .vpy sizing.
///
/// Used to size a logo watermark as a percentage of frame width — the encode
/// input arrives over a pipe, so it cannot be probed. A small error here is
/// only cosmetic (a slightly larger or smaller logo).
pub fn estimate_output_width(config: &Config) -> u32 {
    if config.upscale_enabled {
        let fallback = match config.format {
            VideoFormat::Ntsc => 960,
            VideoFormat::Pal => 1024,
        };
        return config
            .upscale_resolution()
            .split('x')
            .next()
            .and_then(|w| w.parse::<u32>().ok())
            .unwrap_or(fallback);
    }

    if config.is_hd_source() {
        return 1920;
    }

    if config.crop_preset() == CropPreset::Overscan {
        return 704;
    }

    if config.par_correction {
        return match config.format {
            // Anamorphic SAR widens the square-pixel output.
            VideoFormat::Ntsc if config.detected_sar == "32:27" => 854,
            VideoFormat::Ntsc => 640,
            VideoFormat::Pal if config.detected_sar == "64:45" => 1024,
            VideoFormat::Pal => 768,
        };
    }

    if config.crop_preset() == CropPreset::Bt601 { 704 } else { 720 }
}
