//! What was done to a video, written where it will still be found later.
//!
//! A restored tape outlives the machine that restored it, and usually the
//! memory of how. The one question anybody asks of a file like this a year on
//! is "what did I do to it" — and after that, "can I do the same again to the
//! next tape from the same deck". Both are answerable only if the settings
//! traveled with the file.
//!
//! Three forms, deliberately:
//!
//! * **A summary in the file's own metadata.** Survives copying, emailing and
//!   re-muxing, needs no companion file, and is visible in MediaInfo or
//!   ffprobe without this program. Short, because a metadata field is not a
//!   document.
//! * **A JSON sidecar.** The complete configuration, machine-readable, and
//!   the thing to reach for when restoring a second tape the same way.
//! * **A log.** What the tools actually said, for when something looks wrong
//!   and the question is why.
//!
//! All three are generated from the same `Config` the render used, so none of
//! them can describe a setting that was not applied.

use crate::config::{
    CaptureMethod, ColorCorrection, Config, DehaloMode, FieldOrder, LevelsAdjustment, Mode,
    NoiseLevel, OutputFormat, VideoFormat, Watermark, QtgmcImpl,};

/// A one-line summary of everything that was applied.
///
/// Reads as a sentence rather than a dump: it is going into a comment field
/// that players and taggers will show inline, often truncated, so the
/// important part goes first. Omits anything that was left alone — a list of
/// twelve "none"s tells a reader nothing and buries the two settings that
/// were actually used.
pub fn summary(config: &Config, version: &str) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(match config.field_order {
        FieldOrder::Progressive => "progressive source, not deinterlaced".to_string(),
        _ if config.ivtc_mode => "inverse telecine (pulldown removed)".to_string(),
        // The implementation is named, not just the settings. Two exports of
        // the same file with the same settings can differ by 16% in encoded
        // size depending on which QTGMC produced them — measured on a Hi8
        // capture — and with only "QTGMC TFF double-rate" written down there
        // is nothing in the log, the file metadata or the settings JSON that
        // says which one ran. Comparing two renders was then guesswork.
        order => format!(
            "QTGMC ({}) {} {}",
            match config.qtgmc_impl {
                QtgmcImpl::Legacy => "havsfunc",
                QtgmcImpl::Modern => "vs-jetpack",
            },
            match order {
                FieldOrder::Tff => "TFF",
                FieldOrder::Bff => "BFF",
                FieldOrder::Progressive => unreachable!(),
            },
            if config.double_rate {
                "double-rate"
            } else {
                "single-rate"
            }
        ),
    });

    if let Some(crop) = crop_summary(config) {
        parts.push(crop);
    }
    if config.levels_adjustment != LevelsAdjustment::None {
        parts.push(match config.levels_adjustment {
            LevelsAdjustment::Clamp => "levels clamped to 16-235".to_string(),
            LevelsAdjustment::Stretch => format!(
                "levels stretched from {}-{}",
                config.levels_black, config.levels_white
            ),
            LevelsAdjustment::None => unreachable!(),
        });
    }
    if config.color_correction != ColorCorrection::None {
        parts.push(format!(
            "color corrected U{:+.1} V{:+.1}",
            config.u_correction, config.v_correction
        ));
    }
    if config.noise_level != NoiseLevel::None {
        parts.push(format!("denoise {}", lower(&format!("{:?}", config.noise_level))));
    }
    if config.dehalo_mode != DehaloMode::None {
        parts.push(format!("dehalo {}", lower(&format!("{:?}", config.dehalo_mode))));
    }
    if config.yc_delay != 0 {
        parts.push(format!("chroma shifted {:+}px", config.yc_delay));
    }
    if config.grain_strength > 0.0 {
        parts.push(format!("grain {:.1}", config.grain_strength));
    }
    if config.upscale_enabled {
        parts.push("upscaled".to_string());
    }
    if config.fix_one_sided_audio {
        parts.push("one-sided audio repaired".to_string());
    }
    if !matches!(config.watermark, Watermark::None) {
        parts.push("watermarked".to_string());
    }
    if !config.trim_ranges.is_empty() {
        parts.push(format!("trimmed to {} range(s)", config.trim_ranges.len()));
    }

    format!("VCG Deinterlacer {version}: {}", parts.join("; "))
}

fn crop_summary(config: &Config) -> Option<String> {
    use crate::config::CropPreset;
    match config.effective_crop_preset() {
        CropPreset::None => None,
        CropPreset::Manual => Some(format!(
            "cropped L{} R{} T{} B{}",
            config.crop_left, config.crop_right, config.crop_top, config.crop_bottom
        )),
        CropPreset::Bt601 => Some("cropped BT.601 (8 a side)".to_string()),
        CropPreset::Overscan => Some("cropped overscan-clean".to_string()),
    }
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// The settings as JSON, for re-use on another tape.
///
/// `Config` already derives `Serialize` for the parity harness, so this is
/// mostly a matter of adding the things a reader needs that are not settings:
/// what the file was, what was measured, and which build did it. Wrapped in
/// an object with a `format` marker so a future version can recognize and
/// migrate it rather than guessing.
pub fn sidecar_json(
    config: &Config,
    version: &str,
    source_name: &str,
    findings: Option<&str>,
) -> String {
    let settings = serde_json::to_value(config)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
    let doc = serde_json::json!({
        "format": SIDECAR_FORMAT,
        "format_version": 1,
        "produced_by": format!("VCG Deinterlacer {version}"),
        "source": source_name,
        "summary": summary(config, version),
        "analysis": findings,
        "settings": settings,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// The marker written into every sidecar, and the only thing that identifies
/// one. Checked on the way back in, because a settings file and a log file
/// sit beside each other with similar names and picking the wrong one should
/// say so rather than produce a configuration of defaults.
pub const SIDECAR_FORMAT: &str = "vcg-deinterlacer-settings";

/// What a settings file was read back as.
pub struct LoadedSettings {
    pub config: Config,
    /// The file it was originally made from, for the line that confirms the
    /// load. A preset is usually recognized by the tape it came off rather
    /// than by the name somebody gave the JSON.
    pub source: Option<String>,
    /// Which build wrote it.
    pub produced_by: Option<String>,
}

/// Read a settings file back into a configuration.
///
/// The inverse of `sidecar_json`, and deliberately strict about the wrapper:
/// a bare `Config` object, a log file, or another application's JSON are all
/// rejected by name rather than quietly deserializing into defaults. Missing
/// *fields* inside `settings` are a different matter — those fall back to
/// `Config::default()`, which is what makes a file written by an older build
/// still loadable.
///
/// `input_path` is not a setting and never comes back: the configuration
/// being loaded into already points at the file the user has open, and the
/// generated script reads its source from exactly that field.
pub fn load_sidecar(text: &str) -> Result<LoadedSettings, String> {
    let doc: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("that is not a JSON file ({e})"))?;

    let format = doc.get("format").and_then(|v| v.as_str());
    if format != Some(SIDECAR_FORMAT) {
        return Err("that JSON file was not written by this app".to_string());
    }
    let settings = doc
        .get("settings")
        .ok_or_else(|| "the file has no settings in it".to_string())?;

    let mut config: Config = serde_json::from_value(settings.clone())
        .map_err(|e| format!("the settings could not be read ({e})"))?;
    config.input_path = String::new();

    Ok(LoadedSettings {
        config,
        source: doc
            .get("source")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        produced_by: doc
            .get("produced_by")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

/// A human-readable log of one export.
///
/// Everything a support question needs in one paste: what went in, what came
/// out, what was decided about it and what the tools said while doing it. The
/// tool output goes last because it is the longest and the least often
/// wanted.
pub fn log_text(entry: LogEntry<'_>) -> String {
    let LogEntry {
        config,
        version,
        source_name,
        output_name,
        started,
        seconds,
        findings,
        tool_output,
        mode,
    } = entry;

    let mut s = String::new();
    s.push_str(&format!("VCG Deinterlacer {version}\n"));
    s.push_str(&format!("Exported {started}\n\n"));
    s.push_str(&format!("Source      {source_name}\n"));
    s.push_str(&format!("Output      {output_name}\n"));
    s.push_str(&format!("Took        {}\n", duration(seconds)));
    s.push_str(&format!("Mode        {}\n\n", mode_name(mode)));

    s.push_str("What was applied\n");
    s.push_str(&format!("  {}\n\n", summary(config, version)));

    s.push_str("Source\n");
    s.push_str(&format!(
        "  standard    {}\n",
        match config.format {
            VideoFormat::Ntsc => "NTSC",
            VideoFormat::Pal => "PAL",
        }
    ));
    s.push_str(&format!(
        "  capture     {}\n",
        match config.capture_method {
            CaptureMethod::Dv => "DV",
            CaptureMethod::Sd => "analog capture card",
        }
    ));
    s.push_str(&format!("  field order {:?}\n", config.field_order));
    s.push_str(&format!(
        "  output      {}\n\n",
        // The three the app offers, plus the lossless AVI codecs the engine
        // still supports from the Python. A log must describe whatever was
        // actually used, not only what the wizard can pick.
        match config.output_format {
            OutputFormat::Prores => "ProRes 422 HQ (.mov)",
            OutputFormat::Ffv1 => "FFV1 (.mkv)",
            OutputFormat::H264 => "H.264 (.mp4)",
            OutputFormat::Huffyuv => "Huffyuv (.avi)",
            OutputFormat::Utvideo => "Ut Video (.avi)",
            OutputFormat::Lagarith => "Lagarith (.avi)",
        }
    ));

    if let Some(f) = findings {
        s.push_str("What was measured\n");
        for line in f.lines() {
            s.push_str(&format!("  {line}\n"));
        }
        s.push('\n');
    }

    if !tool_output.trim().is_empty() {
        s.push_str("Tool output\n");
        for line in tool_output.lines() {
            s.push_str(&format!("  {line}\n"));
        }
    }
    s
}

/// Everything the log needs. A struct because eight positional parameters of
/// mostly-strings is a mistake waiting to happen.
pub struct LogEntry<'a> {
    pub config: &'a Config,
    pub version: &'a str,
    pub source_name: &'a str,
    pub output_name: &'a str,
    /// When the export finished, already formatted.
    pub started: &'a str,
    pub seconds: f64,
    pub findings: Option<&'a str>,
    pub tool_output: &'a str,
    /// Which mode the export ran in. Not on `Config`: the mode decides how a
    /// config is *arrived at*, not what is in it, and the two are kept apart
    /// deliberately so a mode cannot be inferred from settings that several
    /// modes could have produced.
    pub mode: Option<Mode>,
}

fn mode_name(mode: Option<Mode>) -> &'static str {
    match mode {
        Some(Mode::JustDeinterlace) => "Just deinterlace",
        Some(Mode::EnhanceAutomatically) => "Deinterlace + auto-fix",
        Some(Mode::EnhanceManually) => "Full control",
        None => "unknown",
    }
}

fn duration(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The summary says what was done and stays quiet about what was not. A
    /// list of twelve "none"s buries the two settings that mattered.
    #[test]
    fn the_summary_names_only_what_was_applied() {
        let config = Config {
            noise_level: NoiseLevel::Moderate,
            ..Config::default()
        };
        let s = summary(&config, "2.0.0");
        assert!(s.contains("denoise moderate"), "{s}");
        assert!(!s.contains("dehalo"), "untouched settings stay out: {s}");
        assert!(!s.contains("grain"), "{s}");
    }

    /// Field order is in the summary, because it is the setting most worth
    /// being able to check after the fact.
    #[test]
    fn the_summary_records_the_field_order() {
        let tff = summary(&Config::default(), "2.0.0");
        assert!(tff.contains("TFF"), "{tff}");

        let bff = summary(
            &Config {
                field_order: FieldOrder::Bff,
                ..Config::default()
            },
            "2.0.0",
        );
        assert!(bff.contains("BFF"), "{bff}");
    }

    /// …and so is which QTGMC did the work.
    ///
    /// Two exports of one file with identical settings came back 16% apart
    /// in encoded size, and nothing in the log, the file metadata or the
    /// settings JSON said which implementation produced either of them, so
    /// the pair could not be compared at all.
    #[test]
    fn the_summary_records_which_qtgmc_ran() {
        let legacy = summary(
            &Config { qtgmc_impl: QtgmcImpl::Legacy, ..Config::default() },
            "2.0.0",
        );
        assert!(legacy.contains("havsfunc"), "{legacy}");
        assert!(!legacy.contains("jetpack"), "{legacy}");

        let modern = summary(
            &Config { qtgmc_impl: QtgmcImpl::Modern, ..Config::default() },
            "2.0.0",
        );
        assert!(modern.contains("vs-jetpack"), "{modern}");
        assert!(!modern.contains("havsfunc"), "{modern}");
    }

    /// A progressive source names no deinterlacer, because none ran.
    #[test]
    fn a_progressive_source_names_no_implementation() {
        let s = summary(
            &Config {
                field_order: FieldOrder::Progressive,
                ..Config::default()
            },
            "2.0.0",
        );
        assert!(!s.contains("havsfunc"), "{s}");
        assert!(!s.contains("jetpack"), "{s}");
    }

    /// A progressive source was not deinterlaced, and must not claim to have
    /// been.
    #[test]
    fn a_progressive_source_says_so() {
        let s = summary(
            &Config {
                field_order: FieldOrder::Progressive,
                ..Config::default()
            },
            "2.0.0",
        );
        assert!(s.contains("not deinterlaced"), "{s}");
        assert!(!s.contains("QTGMC"), "{s}");
    }

    /// The sidecar has to round-trip: its whole purpose is restoring a second
    /// tape the same way, which means reading it back.
    #[test]
    fn the_sidecar_round_trips_through_serde() {
        let config = Config {
            noise_level: NoiseLevel::Heavy,
            yc_delay: -2,
            crop_left: 18,
            ..Config::default()
        };
        let json = sidecar_json(&config, "2.0.0", "tape.avi", Some("nothing of note"));
        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(doc["format"], "vcg-deinterlacer-settings");
        assert_eq!(doc["source"], "tape.avi");

        let back: Config =
            serde_json::from_value(doc["settings"].clone()).expect("settings read back");
        assert_eq!(back.noise_level, NoiseLevel::Heavy);
        assert_eq!(back.yc_delay, -2);
        assert_eq!(back.crop_left, 18);
    }

    #[test]
    fn the_log_names_the_file_both_ways_and_the_settings() {
        let config = Config::default();
        let text = log_text(LogEntry {
            config: &config,
            version: "2.0.0",
            source_name: "tape.avi",
            output_name: "tape_VCG200.mov",
            started: "14 October 2026 21:40",
            seconds: 605.0,
            findings: Some("levels 16-233"),
            tool_output: "frame=1000",
            mode: Some(Mode::EnhanceManually),
        });
        assert!(text.contains("tape.avi"));
        assert!(text.contains("tape_VCG200.mov"));
        assert!(text.contains("10m 05s"));
        assert!(text.contains("levels 16-233"));
        assert!(text.contains("frame=1000"));
    }
}
