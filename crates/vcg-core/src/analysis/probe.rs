//! Reading ffprobe JSON.
//!
//! Ported from `get_video_info`, `detect_par_format` and `classify_source`.
//! Each takes the JSON text rather than a file path, so the decisions are pure.
//!
//! All three Python functions wrap their whole body in a bare `except: pass`
//! and return a default, so malformed or missing JSON must degrade rather than
//! fail. That is reproduced by returning the defaults on a parse error.

use serde_json::Value;

use crate::config::{CaptureMethod, SourceClass, VideoFormat};

/// Basic stream facts. `None` when no video stream could be read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
}

/// Parse `r_frame_rate` ("30000/1001"), defaulting to 30.0.
///
/// A zero denominator or non-numeric half degrades to 30.0 rather than
/// raising, matching the Python's ZeroDivisionError/ValueError guard.
fn parse_frame_rate(raw: &str) -> f64 {
    let Some((num, den)) = raw.split_once('/') else {
        return 30.0;
    };
    match (num.parse::<f64>(), den.parse::<f64>()) {
        (Ok(n), Ok(d)) if d != 0.0 => n / d,
        _ => 30.0,
    }
}

/// Read the first video stream out of `-show_format -show_streams` JSON.
pub fn video_info(json: &str) -> Option<VideoInfo> {
    let v: Value = serde_json::from_str(json).ok()?;
    let streams = v.get("streams")?.as_array()?;
    let stream = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(Value::as_str) == Some("video"))?;

    Some(VideoInfo {
        width: stream.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
        height: stream.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
        fps: parse_frame_rate(
            stream
                .get("r_frame_rate")
                .and_then(Value::as_str)
                .unwrap_or("30/1"),
        ),
        // Duration lives on `format`, not the stream, and is a JSON string.
        duration: v
            .get("format")
            .and_then(|f| f.get("duration"))
            .and_then(Value::as_str)
            .and_then(|d| d.parse().ok())
            .unwrap_or(0.0),
    })
}

/// What `detect_par_format` works out from SAR, codec and resolution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParFormat {
    pub format: Option<VideoFormat>,
    pub capture_method: Option<DetectedCaptureMethod>,
    /// The raw SAR string from ffprobe, e.g. "10:11".
    pub sar: String,
}

/// Detection can also conclude "DVD", which `CaptureMethod` has no variant for
/// — the wizard maps it onto SD with different crop defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedCaptureMethod {
    Sd,
    Dv,
    Dvd,
}

impl DetectedCaptureMethod {
    /// How the pipeline treats it. DVD is an MPEG-2 SD source.
    pub fn as_capture_method(self) -> CaptureMethod {
        match self {
            DetectedCaptureMethod::Dv => CaptureMethod::Dv,
            DetectedCaptureMethod::Sd | DetectedCaptureMethod::Dvd => CaptureMethod::Sd,
        }
    }
}

/// Detect video standard and capture method from `-show_streams -select_streams v:0`.
///
/// SAR mapping:
///   10:11 -> NTSC SD analog capture      59:54 -> PAL SD analog capture
///    8:9  -> NTSC DV25 *or* NTSC DVD 4:3 16:15 -> PAL DV25 *or* PAL DVD 4:3
///   32:27 -> NTSC DVD 16:9 anamorphic    64:45 -> PAL DVD 16:9 anamorphic
///
/// Codec takes priority over SAR for the capture method, because NTSC DVD
/// (mpeg2video, SAR 8:9) and NTSC DV25 (dvvideo, SAR 8:9) share a pixel aspect
/// ratio and only the codec separates them.
pub fn par_format(json: &str) -> ParFormat {
    let mut out = ParFormat::default();

    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return out;
    };
    let Some(s) = v.get("streams").and_then(Value::as_array).and_then(|a| a.first()) else {
        return out;
    };

    let height = s.get("height").and_then(Value::as_u64).unwrap_or(0);
    let sar = s
        .get("sample_aspect_ratio")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let codec = s
        .get("codec_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();

    out.sar = sar.clone();

    // Format from SAR first (most precise), falling back to height.
    out.format = match sar.as_str() {
        "10:11" | "8:9" | "32:27" => Some(VideoFormat::Ntsc),
        "59:54" | "16:15" | "64:45" => Some(VideoFormat::Pal),
        _ => match height {
            480 => Some(VideoFormat::Ntsc),
            576 => Some(VideoFormat::Pal),
            _ => None,
        },
    };

    out.capture_method = if codec == "dvvideo" {
        // Unambiguous.
        Some(DetectedCaptureMethod::Dv)
    } else if codec == "mpeg2video" && (height == 480 || height == 576) {
        Some(DetectedCaptureMethod::Dvd)
    } else {
        match sar.as_str() {
            // Shared by DV and DVD; with no codec tag, assume DV.
            "8:9" | "16:15" => Some(DetectedCaptureMethod::Dv),
            "10:11" | "59:54" => Some(DetectedCaptureMethod::Sd),
            // Square pixels — typical of SD capture cards.
            "1:1" | "0:1" | "" => Some(DetectedCaptureMethod::Sd),
            _ => None,
        }
    };

    out
}

/// Field dominance as the container declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedFieldOrder {
    Tff,
    Bff,
    Progressive,
    Unknown,
}

/// What `classify_source` determines about a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Classification {
    pub source_class: SourceClass,
    pub display_name: String,
    pub codec: String,
    /// The stream's FourCC, as the container spells it — `HFYU`, `LAGS`,
    /// `YUY2`, `dvsd`. Kept beside `codec`, not folded into it, because the
    /// two answer different questions: `codec` is the decoder ffmpeg chose,
    /// while this is what the capture program actually wrote in the file.
    /// For a lossless capture codec they agree, but an uncompressed AVI
    /// decodes as plain `rawvideo` whatever its pixel layout, and only the
    /// FourCC distinguishes a YUY2 capture from a UYVY or RGB one.
    pub codec_tag: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub field_order: DetectedFieldOrder,
    /// The container flags the stream as progressive.
    pub progressive: bool,
    /// 1440x1080 with non-square pixels, needing a 1920x1080 scale.
    pub par_needed: bool,
    pub pix_fmt: String,
    /// 10-bit or otherwise non-standard YUV that must be normalized to 8-bit
    /// right after the source load.
    pub needs_pixfmt_conversion: bool,
}

impl Default for Classification {
    fn default() -> Self {
        Self {
            source_class: SourceClass::Sd,
            display_name: "SD Interlaced".to_string(),
            codec: String::new(),
            codec_tag: String::new(),
            width: 0,
            height: 0,
            fps: 0.0,
            field_order: DetectedFieldOrder::Unknown,
            progressive: false,
            par_needed: false,
            pix_fmt: String::new(),
            needs_pixfmt_conversion: false,
        }
    }
}

/// Pixel formats VFM (inverse telecine) accepts.
///
/// RGB and packed formats are excluded because the .vpy already converts those
/// with an explicit matrix, and yuv411p (DV) is excluded because the existing
/// YUV411 -> YUV422P8 path handles it with better chroma.
const VFM_SAFE: [&str; 10] = [
    "yuv420p", "yuvj420p", "yuv422p", "yuvj422p", "yuv440p", "yuvj440p", "yuv444p", "yuvj444p",
    "yuv411p", "yuvj411p",
];

/// Classify a file as SD, AVCHD or HDV from its codec and resolution.
///
/// Rules:
///   h264       + height 1080             -> avchd
///   mpeg2video + height 1080 + width 1440 -> hdv
///   anything else                         -> sd
pub fn classify(json: &str) -> Classification {
    let mut out = Classification::default();

    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return out;
    };
    let Some(s) = v.get("streams").and_then(Value::as_array).and_then(|a| a.first()) else {
        return out;
    };

    out.codec = s
        .get("codec_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    // ffprobe writes "[0][0][0][0]" when a container carries no FourCC at
    // all (MP4, Matroska); that is not a tag, so it is dropped rather than
    // shown as one.
    out.codec_tag = s
        .get("codec_tag_string")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if out.codec_tag.starts_with("[0]") {
        out.codec_tag.clear();
    }
    out.width = s.get("width").and_then(Value::as_u64).unwrap_or(0) as u32;
    out.height = s.get("height").and_then(Value::as_u64).unwrap_or(0) as u32;
    out.fps = parse_frame_rate(
        s.get("r_frame_rate")
            .and_then(Value::as_str)
            .unwrap_or("30/1"),
    );
    out.pix_fmt = s
        .get("pix_fmt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();

    // 10-bit sources (a DVD rip re-encoded as yuv420p10le, say) pass the
    // loader's format whitelist and then crash VFM with a cryptic format
    // error, so flag anything YUV or gray that is not standard 8-bit.
    let p = out.pix_fmt.as_str();
    if (p.starts_with("yuv") || p.starts_with("gray")) && !VFM_SAFE.contains(&p) && p != "gray" {
        out.needs_pixfmt_conversion = true;
    }

    out.field_order = match s
        .get("field_order")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "tt" | "tff" => DetectedFieldOrder::Tff,
        "bb" | "bff" => DetectedFieldOrder::Bff,
        // The container flag is authoritative when it says progressive — a
        // phone or DSLR 1080p clip is not an interlaced camcorder source and
        // must not be labeled or defaulted as one.
        "progressive" => {
            out.progressive = true;
            DetectedFieldOrder::Progressive
        }
        _ => DetectedFieldOrder::Unknown,
    };

    let prog = out.progressive;

    if out.codec == "h264" && out.height == 1080 {
        out.source_class = SourceClass::Avchd;
        if out.field_order == DetectedFieldOrder::Unknown {
            out.field_order = DetectedFieldOrder::Tff;
        }
        // Some Sony/Panasonic AVCHD camcorders record 1440x1080 with
        // non-square pixels (4:3 PAR) that must be scaled to 1920x1080, same
        // as HDV. Canon and most others record native 1920x1080 square pixels.
        if out.width == 1440 {
            out.display_name = if prog {
                "HD Progressive (1440×1080p anamorphic)"
            } else {
                "AVCHD / MTS (1440×1080i anamorphic)"
            }
            .to_string();
            out.par_needed = true;
        } else {
            out.display_name = if prog {
                "HD Progressive (1920×1080p)"
            } else {
                "AVCHD / MTS (1920×1080i)"
            }
            .to_string();
            out.par_needed = false;
        }
    } else if out.codec == "mpeg2video" && out.height == 1080 && out.width == 1440 {
        out.source_class = SourceClass::Hdv;
        out.display_name = if prog { "HDV (1080p)" } else { "HDV (1080i)" }.to_string();
        if out.field_order == DetectedFieldOrder::Unknown {
            out.field_order = DetectedFieldOrder::Tff;
        }
        out.par_needed = true;
    } else {
        out.source_class = SourceClass::Sd;
        out.display_name = if prog { "SD Progressive" } else { "SD Interlaced" }.to_string();
    }

    out
}
