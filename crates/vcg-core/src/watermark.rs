//! Watermark burn-in via FFmpeg filters.
//!
//! Ported from `build_watermark_args`, `_drawtext_fontfile_arg` and
//! `_merge_video_filter`.

use std::path::{Path, PathBuf};

use crate::colorspace::CsTag;
use crate::config::{Config, Watermark, WatermarkPosition};
use crate::ffmpeg::estimate_output_width;

/// A video filter option: the flag and the filter graph it carries.
///
/// FFmpeg allows only one `-vf`/`-filter_complex` per stream, so this is a
/// single value that later stages append to, never a list to push onto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFilter {
    /// Either `-vf` or `-filter_complex`.
    pub flag: &'static str,
    pub graph: String,
}

impl VideoFilter {
    /// Append the colorspace `setparams` filter to this chain, so it runs
    /// after the watermark and immediately before the encoder.
    pub fn with_cs_tag(mut self, cs: CsTag) -> Self {
        self.graph = format!("{},{}", self.graph, cs.setparams_filter());
        self
    }

    pub fn to_args(&self) -> Vec<String> {
        vec![self.flag.to_string(), self.graph.clone()]
    }
}

/// Merge the colorspace `setparams` filter into an optional watermark filter.
///
/// With no watermark this is a bare `-vf setparams=...`; with one, `setparams`
/// is appended to the existing chain.
pub fn merge_video_filter(wm: Option<VideoFilter>, cs: CsTag) -> VideoFilter {
    match wm {
        Some(f) => f.with_cs_tag(cs),
        None => VideoFilter {
            flag: "-vf",
            graph: cs.setparams_filter(),
        },
    }
}

impl WatermarkPosition {
    /// Position expression for `drawtext`, where `w`/`h` are the *input*
    /// dimensions and the text box is `tw`/`th`.
    fn drawtext_expr(self) -> &'static str {
        match self {
            WatermarkPosition::BottomRight => "x=w-tw-20:y=h-th-20",
            WatermarkPosition::BottomLeft => "x=20:y=h-th-20",
            WatermarkPosition::TopRight => "x=w-tw-20:y=20",
            WatermarkPosition::TopLeft => "x=20:y=20",
            WatermarkPosition::Center => "x=(w-tw)/2:y=(h-th)/2",
        }
    }

    /// Position expression for `overlay`, where `W`/`H` are the main input and
    /// `w`/`h` the overlaid logo — the opposite convention to `drawtext`.
    fn overlay_expr(self) -> &'static str {
        match self {
            WatermarkPosition::BottomRight => "W-w-20:H-h-20",
            WatermarkPosition::BottomLeft => "20:H-h-20",
            WatermarkPosition::TopRight => "W-w-20:20",
            WatermarkPosition::TopLeft => "20:20",
            WatermarkPosition::Center => "(W-w)/2:(H-h)/2",
        }
    }
}

/// Escape text for the `drawtext` filter's `text=` option.
///
/// Backslash, colon and quote are filter-parser specials. `%` needs *two*
/// backslashes: one survives the option parser, the second escapes the `%` for
/// drawtext's text expander — a bare `%` is read as a broken `%{...}` sequence
/// and kills the whole label.
pub fn escape_drawtext(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\\\%")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

/// Sans-serif system fonts to try, in preference order.
const FONT_CANDIDATES: [&str; 4] = ["segoeui.ttf", "arial.ttf", "calibri.ttf", "tahoma.ttf"];

/// Locate a usable font file for `drawtext`.
///
/// The bundled FFmpeg's fontconfig cannot find a default font on Windows, so
/// `drawtext` silently renders nothing unless given an explicit file.
pub fn find_system_font() -> Option<PathBuf> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
    let fonts = Path::new(&windir).join("Fonts");
    FONT_CANDIDATES
        .iter()
        .map(|f| fonts.join(f))
        .find(|p| p.exists())
}

/// Render a font path as a `fontfile='...':` prefix, escaping the colon for
/// the filter parser.
pub fn fontfile_arg(font: Option<&Path>) -> String {
    match font {
        Some(p) => {
            let escaped = p.to_string_lossy().replace('\\', "/").replace(':', "\\:");
            format!("fontfile='{escaped}':")
        }
        None => String::new(),
    }
}

/// Extra inputs and the filter for the configured watermark.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WatermarkArgs {
    /// Extra `-i <path>` args (a logo image, if any).
    pub extra_inputs: Vec<String>,
    pub filter: Option<VideoFilter>,
}

/// Build the watermark inputs and filter.
///
/// `font` is injected rather than looked up so this stays a pure function —
/// the Python version probes the filesystem from inside the arg builder, which
/// makes its output untestable and machine-dependent.
pub fn watermark_args(config: &Config, font: Option<&Path>) -> WatermarkArgs {
    let alpha = config.wm_opacity;

    match &config.watermark {
        Watermark::None => WatermarkArgs::default(),

        Watermark::Text { text, fontsize } => {
            let text = text.trim();
            // An empty label is a no-op, not an empty drawtext.
            if text.is_empty() {
                return WatermarkArgs::default();
            }
            let graph = format!(
                "drawtext={font}text='{text}':fontcolor=white@{alpha:.2}\
                 :fontsize={fontsize}:box=1:boxcolor=black@0.3\
                 :boxborderw=4:{pos}",
                font = fontfile_arg(font),
                text = escape_drawtext(text),
                pos = config.wm_position.drawtext_expr(),
            );
            WatermarkArgs {
                extra_inputs: Vec::new(),
                filter: Some(VideoFilter { flag: "-vf", graph }),
            }
        }

        Watermark::Logo {
            logo_path,
            logo_size,
        } => {
            if logo_path.is_empty() || !Path::new(logo_path).exists() {
                return WatermarkArgs::default();
            }
            // Size the logo as a percentage of the estimated output width,
            // never below 16px.
            let logo_px =
                ((estimate_output_width(config) as f64 * logo_size).round() as u32).max(16);
            let graph = format!(
                "[1:v]scale={logo_px}:-1,format=rgba,\
                 colorchannelmixer=aa={alpha:.2}[wm];\
                 [0:v][wm]overlay={pos}",
                pos = config.wm_position.overlay_expr(),
            );
            WatermarkArgs {
                extra_inputs: vec!["-i".into(), logo_path.clone()],
                filter: Some(VideoFilter {
                    flag: "-filter_complex",
                    graph,
                }),
            }
        }
    }
}
