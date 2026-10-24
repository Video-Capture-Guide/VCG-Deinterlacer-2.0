//! Invariants of the analysis probes.
//!
//! These pin the parsing and the hand-calibrated thresholds, and run against
//! the genuine ffmpeg output in `tests/fixtures/`, so a change in how the
//! text is parsed is caught here rather than in production.

use std::path::PathBuf;

use vcg_core::analysis::classify::{
    bleeding, color_data, levels, noise, BleedLevel, ColorCast, NoiseTrigger, NoiseVerdict,
    SaturationIssue,
};
use vcg_core::analysis::halo::{self, EdgeTally, HaloLevel, LumaPlane};
use vcg_core::analysis::idet::{sample_plan, verdict, Confidence, IdetCounts, Pattern};
use vcg_core::analysis::probe::{
    classify, par_format, video_info, DetectedCaptureMethod, DetectedFieldOrder,
};
use vcg_core::analysis::signalstats::SignalStatsRun;
use vcg_core::analysis::Recommendation;
use vcg_core::config::{SourceClass, VideoFormat};

fn fixture(name: &str) -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "tests", "fixtures", name]
        .iter()
        .collect();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn run(name: &str) -> SignalStatsRun {
    SignalStatsRun::parse(&fixture(name))
}

// ── Real captured output ────────────────────────────────────────────────────

#[test]
fn real_vhs_signalstats_parses() {
    let r = run("signalstats_vhs.txt");
    // The capture is a 720x480 NTSC VHS transfer; these are its actual values.
    assert_eq!(r.ymin.first(), Some(&13));
    assert_eq!(r.ymax.first(), Some(&228));
    assert_eq!(r.uavg.first(), Some(&122.25));
    assert_eq!(r.vavg.first(), Some(&130.483));
    assert_eq!(r.satavg.first(), Some(&9.26001));
    assert_eq!(r.tout.first(), Some(&0.00305845));
}

#[test]
fn real_vhs_luma_is_out_of_legal_range() {
    // YMIN 13 is below the legal floor of 16, so the levels page should offer
    // a correction for this file.
    let l = levels(&[run("signalstats_vhs.txt")]);
    // The aggregate minimum across every frame in the capture, not the first
    // frame's 13.
    assert_eq!(l.min_y, Some(12));
    assert!(l.needs_adjustment);
}

#[test]
fn real_vhs_is_very_low_saturation() {
    let c = color_data(&[run("signalstats_vhs.txt")]).expect("U and V were measured");
    assert_eq!(c.sat_issue, Some(SaturationIssue::VeryLow));
    // U sits 5.75 below neutral — real but under the 10-unit cast threshold.
    assert!(c.u_offset < 0.0 && c.u_offset.abs() < 10.0);
    assert_eq!(c.color_cast, None);
}

#[test]
fn real_vhs_reads_as_interlaced_video_not_film() {
    // 241 TFF frames, zero repeated fields — genuine interlaced video. This is
    // the case a telecine false positive would ruin, so it is worth pinning.
    let counts = IdetCounts::parse(&fixture("idet_vhs.txt")).expect("a summary is present");
    assert_eq!(counts.tff, 241);
    assert_eq!(counts.prog, 0);
    assert_eq!(counts.rep, 0);

    let t = verdict(&[counts], VideoFormat::Ntsc);
    assert!(!t.detected());
    assert!(!t.progressive_source);
    assert_eq!(t.windows, 1);
    assert_eq!(t.film_windows, 0);
}

#[test]
fn a_summary_glued_to_a_progress_line_is_still_read() {
    // ffmpeg writes the real summary straight after a `frame=…` progress line
    // with no newline, so line-prefix matching would miss it entirely.
    let raw = fixture("idet_vhs.txt");
    assert!(
        raw.lines()
            .any(|l| l.starts_with("frame=") && l.contains("Repeated Fields")),
        "fixture no longer exercises the glued-line case"
    );
    assert_eq!(
        IdetCounts::parse(&raw).map(|c| c.rep_total),
        Some(241),
        "the glued Repeated Fields line must still be parsed"
    );
}

#[test]
fn a_seek_past_end_of_file_yields_a_zeroed_but_present_summary() {
    // idet still prints its summaries with every count at zero. That is a real
    // (empty) result, not a failure, and the window is then dropped for having
    // too few frames rather than being treated as an error.
    let counts = IdetCounts::parse(&fixture("idet_past_end.txt")).expect("summary present");
    assert_eq!(counts.total(), 0);

    let t = verdict(&[counts], VideoFormat::Ntsc);
    assert_eq!(t.windows, 0);
    assert!(!t.detected());
}

#[test]
fn output_with_no_summary_is_a_failure_not_a_zero() {
    assert_eq!(IdetCounts::parse("ffmpeg version 8\nnothing here\n"), None);
}

#[test]
fn real_vhs_ffprobe_classifies_as_sd() {
    let c = classify(&fixture("ffprobe_streams_vhs.json"));
    assert_eq!(c.source_class, SourceClass::Sd);
    assert_eq!(c.width, 720);
    assert_eq!(c.height, 480);
    assert!(!c.needs_pixfmt_conversion);

    let p = par_format(&fixture("ffprobe_streams_vhs.json"));
    assert_eq!(p.format, Some(VideoFormat::Ntsc));

    let i = video_info(&fixture("ffprobe_full_vhs.json")).expect("a video stream");
    assert_eq!((i.width, i.height), (720, 480));
    assert!((i.fps - 29.97).abs() < 0.01);
    assert!(i.duration > 25.0 && i.duration < 26.0);
}

// ── Parsing robustness ──────────────────────────────────────────────────────

#[test]
fn malformed_values_are_dropped_not_fatal() {
    let r = SignalStatsRun::parse(
        "lavfi.signalstats.YMIN=not-a-number\n\
         lavfi.signalstats.YMAX=228\n\
         lavfi.signalstats.UAVG=oops\n",
    );
    assert!(r.ymin.is_empty());
    assert_eq!(r.ymax, vec![228]);
    assert!(r.uavg.is_empty());
}

#[test]
fn ymin_and_ymax_require_integers() {
    // The Python reads these with int(), so a fractional value is dropped
    // rather than truncated.
    let r = SignalStatsRun::parse("lavfi.signalstats.YMIN=13.5\n");
    assert!(r.ymin.is_empty());
}

#[test]
fn empty_input_produces_empty_measurements() {
    let r = SignalStatsRun::parse("");
    assert_eq!(r, SignalStatsRun::default());
    // And the classifiers must cope with having nothing.
    assert_eq!(color_data(&[]), None);
    assert!(!noise(&[]).analyzed);
    let l = levels(&[]);
    assert_eq!((l.min_y, l.max_y), (None, None));
}

#[test]
fn an_unmeasurable_file_reports_no_adjustment_needed() {
    // min_y stays at its 255 sentinel and max_y at 0, so neither bound looks
    // out of range. Reporting "needs adjustment" from sentinels would offer a
    // correction for a file nothing could be read from.
    let l = levels(&[SignalStatsRun::default()]);
    assert_eq!((l.min_y, l.max_y), (None, None));
    assert!(!l.needs_adjustment);
}

#[test]
fn ffprobe_garbage_degrades_to_defaults() {
    for json in ["", "{not json", "{}", r#"{"streams": []}"#] {
        assert_eq!(video_info(json), None, "json {json:?}");
        assert_eq!(par_format(json).format, None, "json {json:?}");
        // classify returns its SD default rather than failing.
        assert_eq!(classify(json).source_class, SourceClass::Sd, "json {json:?}");
    }
}

#[test]
fn a_zero_denominator_frame_rate_falls_back_to_30() {
    let c = classify(r#"{"streams":[{"codec_name":"rawvideo","r_frame_rate":"0/0"}]}"#);
    assert_eq!(c.fps, 30.0);
}

// ── Classification rules ────────────────────────────────────────────────────

#[test]
fn source_class_follows_codec_and_resolution() {
    let cls = |codec: &str, w: u32, h: u32| {
        classify(&format!(
            r#"{{"streams":[{{"codec_name":"{codec}","width":{w},"height":{h}}}]}}"#
        ))
        .source_class
    };
    assert_eq!(cls("h264", 1920, 1080), SourceClass::Avchd);
    assert_eq!(cls("h264", 1440, 1080), SourceClass::Avchd);
    assert_eq!(cls("mpeg2video", 1440, 1080), SourceClass::Hdv);
    // 1920-wide MPEG-2 is not HDV; HDV is specifically 1440.
    assert_eq!(cls("mpeg2video", 1920, 1080), SourceClass::Sd);
    assert_eq!(cls("h264", 1280, 720), SourceClass::Sd);
    assert_eq!(cls("rawvideo", 720, 480), SourceClass::Sd);
}

#[test]
fn only_1440_wide_hd_needs_par_correction() {
    let par = |codec: &str, w: u32| {
        classify(&format!(
            r#"{{"streams":[{{"codec_name":"{codec}","width":{w},"height":1080}}]}}"#
        ))
        .par_needed
    };
    assert!(par("h264", 1440));
    assert!(!par("h264", 1920));
    assert!(par("mpeg2video", 1440));
}

#[test]
fn a_progressive_container_flag_is_authoritative() {
    // A phone or DSLR 1080p clip must not be defaulted to interlaced.
    let c = classify(
        r#"{"streams":[{"codec_name":"h264","width":1920,"height":1080,
             "field_order":"progressive"}]}"#,
    );
    assert!(c.progressive);
    assert_eq!(c.field_order, DetectedFieldOrder::Progressive);
    assert!(c.display_name.contains("Progressive"));
}

#[test]
fn hd_sources_with_an_unknown_field_order_default_to_tff() {
    let c = classify(r#"{"streams":[{"codec_name":"h264","width":1920,"height":1080}]}"#);
    assert_eq!(c.field_order, DetectedFieldOrder::Tff);
    // But an SD source is left unknown.
    let sd = classify(r#"{"streams":[{"codec_name":"rawvideo","width":720,"height":480}]}"#);
    assert_eq!(sd.field_order, DetectedFieldOrder::Unknown);
}

#[test]
fn ten_bit_yuv_is_flagged_for_normalisation_but_standard_formats_are_not() {
    // VFM (inverse telecine) only accepts 8-bit YUV; a 10-bit source passes
    // the loader whitelist and then crashes it with a cryptic format error.
    let needs = |pf: &str| {
        classify(&format!(
            r#"{{"streams":[{{"codec_name":"rawvideo","width":720,"height":480,"pix_fmt":"{pf}"}}]}}"#
        ))
        .needs_pixfmt_conversion
    };
    for safe in ["yuv420p", "yuvj420p", "yuv422p", "yuv444p", "yuv411p", "gray"] {
        assert!(!needs(safe), "{safe} must not be converted");
    }
    for unsafe_fmt in ["yuv420p10le", "yuv422p10le", "yuv420p12le", "gray10le"] {
        assert!(needs(unsafe_fmt), "{unsafe_fmt} must be converted");
    }
    // RGB and packed formats are handled elsewhere with an explicit matrix.
    for other in ["rgb24", "bgr24", "nv12", "yuyv422", ""] {
        assert!(!needs(other), "{other} must not be converted here");
    }
}

#[test]
fn codec_beats_sar_for_capture_method() {
    // NTSC DVD (mpeg2video, SAR 8:9) and NTSC DV25 (dvvideo, SAR 8:9) share a
    // pixel aspect ratio, so only the codec separates them.
    let method = |codec: &str, sar: &str, h: u32| {
        par_format(&format!(
            r#"{{"streams":[{{"codec_name":"{codec}","width":720,"height":{h},
                 "sample_aspect_ratio":"{sar}"}}]}}"#
        ))
        .capture_method
    };
    assert_eq!(method("dvvideo", "8:9", 480), Some(DetectedCaptureMethod::Dv));
    assert_eq!(method("mpeg2video", "8:9", 480), Some(DetectedCaptureMethod::Dvd));
    // With no codec tag, 8:9 falls back to the DV assumption.
    assert_eq!(method("rawvideo", "8:9", 480), Some(DetectedCaptureMethod::Dv));
    assert_eq!(method("rawvideo", "10:11", 480), Some(DetectedCaptureMethod::Sd));
    // Square pixels are typical of SD capture cards.
    assert_eq!(method("rawvideo", "1:1", 480), Some(DetectedCaptureMethod::Sd));
    assert_eq!(method("rawvideo", "", 480), Some(DetectedCaptureMethod::Sd));
}

#[test]
fn format_comes_from_sar_first_then_height() {
    let fmt = |sar: &str, h: u32| {
        par_format(&format!(
            r#"{{"streams":[{{"codec_name":"rawvideo","width":720,"height":{h},
                 "sample_aspect_ratio":"{sar}"}}]}}"#
        ))
        .format
    };
    assert_eq!(fmt("10:11", 1080), Some(VideoFormat::Ntsc));
    assert_eq!(fmt("59:54", 1080), Some(VideoFormat::Pal));
    // Unrecognized SAR falls back to height.
    assert_eq!(fmt("7:5", 480), Some(VideoFormat::Ntsc));
    assert_eq!(fmt("7:5", 576), Some(VideoFormat::Pal));
    assert_eq!(fmt("7:5", 720), None);
}

// ── Thresholds ──────────────────────────────────────────────────────────────

fn stats(pairs: &[(&str, f64)]) -> SignalStatsRun {
    let text: String = pairs
        .iter()
        .map(|(k, v)| format!("lavfi.signalstats.{k}={v}\n"))
        .collect();
    SignalStatsRun::parse(&text)
}

#[test]
fn noise_leads_on_tout_with_ydif_as_a_secondary_trigger() {
    let at = |tout: f64, ydif: f64| noise(&[stats(&[("TOUT", tout), ("YDIF", ydif)])]);

    assert_eq!(at(0.0, 0.0).noise_level, NoiseVerdict::Clean);
    assert_eq!(at(0.013, 0.0).noise_level, NoiseVerdict::Light);
    assert_eq!(at(0.031, 0.0).noise_level, NoiseVerdict::Moderate);
    assert_eq!(at(0.081, 0.0).noise_level, NoiseVerdict::Heavy);

    // A quiet tape with heavy motion is classified via YDIF, and the trigger
    // label says so, so the UI shows the right number.
    let by_motion = at(0.001, 31.0);
    assert_eq!(by_motion.noise_level, NoiseVerdict::Heavy);
    assert_eq!(by_motion.trigger, NoiseTrigger::Ydif);
    assert_eq!(at(0.081, 0.0).trigger, NoiseTrigger::Tout);
}

#[test]
fn light_noise_still_recommends_moderate_denoising() {
    // Verdict and recommendation are deliberately not the same vocabulary.
    let n = noise(&[stats(&[("TOUT", 0.013), ("YDIF", 0.0)])]);
    assert_eq!(n.noise_level, NoiseVerdict::Light);
    assert_eq!(n.recommendation, Recommendation::Moderate);
}

#[test]
fn noise_needs_three_samples_to_count_as_analyzed() {
    let with = |n: usize| {
        let text: String = (0..n)
            .map(|_| "lavfi.signalstats.YDIF=5\n".to_string())
            .collect();
        noise(&[SignalStatsRun::parse(&text)]).analyzed
    };
    assert!(!with(2));
    assert!(with(3));
}

#[test]
fn an_inconsistent_color_cast_is_not_flagged() {
    // A large average offset built from wildly varying frames is scene
    // variation, not a cast. Requiring stddev < 1.5x the offset is what stops
    // one bright scene driving a correction.
    // mean 146.67 -> offset +18.67, stddev 34.0. The offset clears the
    // 10-unit bar but 34.0 is not under 1.5 x 18.67, so it must be rejected.
    let scattered = color_data(&[
        stats(&[("UAVG", 100.0), ("VAVG", 128.0)]),
        stats(&[("UAVG", 160.0), ("VAVG", 128.0)]),
        stats(&[("UAVG", 180.0), ("VAVG", 128.0)]),
    ])
    .unwrap();
    assert!(scattered.u_offset.abs() > 10.0, "offset is large");
    assert_eq!(scattered.color_cast, None, "but inconsistent, so not flagged");

    let steady = color_data(&[
        stats(&[("UAVG", 143.0), ("VAVG", 128.0)]),
        stats(&[("UAVG", 144.0), ("VAVG", 128.0)]),
        stats(&[("UAVG", 142.0), ("VAVG", 128.0)]),
    ])
    .unwrap();
    assert_eq!(steady.color_cast, Some(ColorCast::Blue));
}

#[test]
fn cast_direction_and_the_u_v_tie_break() {
    let cast = |u: f64, v: f64| {
        color_data(&[stats(&[("UAVG", 128.0 + u), ("VAVG", 128.0 + v)])])
            .unwrap()
            .color_cast
    };
    assert_eq!(cast(25.0, 0.0), Some(ColorCast::Blue));
    assert_eq!(cast(-25.0, 0.0), Some(ColorCast::Yellow));
    assert_eq!(cast(0.0, 25.0), Some(ColorCast::RedMagenta));
    assert_eq!(cast(0.0, -25.0), Some(ColorCast::GreenCyan));
    // Equal magnitudes: U wins, because the comparison is `>=`.
    assert_eq!(cast(15.0, 15.0), Some(ColorCast::Blue));
    assert_eq!(cast(-15.0, -15.0), Some(ColorCast::Yellow));
    // Just under the threshold.
    assert_eq!(cast(9.9, 0.0), None);
}

#[test]
fn the_cast_correction_cancels_the_offset() {
    let c = color_data(&[stats(&[("UAVG", 140.0), ("VAVG", 120.0)])]).unwrap();
    assert_eq!(c.u_correction, -c.u_offset);
    assert_eq!(c.v_correction, -c.v_offset);
    assert_eq!(c.u_avg + c.u_correction, 128.0);
}

#[test]
fn light_bleeding_is_reported_but_not_recommended() {
    let b = bleeding(&[stats(&[
        ("UHIGH", 160.0),
        ("ULOW", 100.0),
        ("VHIGH", 160.0),
        ("VLOW", 100.0),
        ("SATAVG", 30.0),
    ])]);
    assert_eq!(b.bleed_level, BleedLevel::Moderate);
    assert!(b.recommendation);

    let faint = bleeding(&[stats(&[
        ("UHIGH", 135.0),
        ("ULOW", 120.0),
        ("VHIGH", 135.0),
        ("VLOW", 120.0),
        ("SATAVG", 10.0),
    ])]);
    assert_eq!(faint.bleed_level, BleedLevel::None);
    assert!(!faint.recommendation);
}

#[test]
fn bleeding_seeds_chroma_at_neutral_when_nothing_was_measured() {
    // Missing UHIGH/ULOW must give a spread of zero, not a wild value.
    let b = bleeding(&[SignalStatsRun::default()]);
    assert_eq!(b.avg_h_spread, 0.0);
    assert_eq!(b.avg_v_spread, 0.0);
    assert_eq!(b.bleed_level, BleedLevel::None);
}

// ── Telecine ────────────────────────────────────────────────────────────────

fn window(tff: u64, prog: u64, rep: u64, rep_total: u64) -> IdetCounts {
    IdetCounts { tff, bff: 0, prog, rep, rep_total }
}

/// Textbook 3:2: 40% progressive, 40% carrying a repeated field.
fn film_window() -> IdetCounts {
    window(144, 96, 96, 240)
}

fn video_window() -> IdetCounts {
    window(240, 0, 0, 240)
}

#[test]
fn pal_needs_a_far_higher_progressive_share_than_ntsc() {
    // idet cannot tell a still frame from a progressive one, so ordinary
    // interlaced PAL video easily shows 30-50% "progressive". A shared
    // threshold would call most PAL 50i content film.
    let borderline = [window(144, 96, 0, 240); 3];
    assert!(verdict(&borderline, VideoFormat::Ntsc).detected());
    assert!(!verdict(&borderline, VideoFormat::Pal).detected());
}

#[test]
fn repeated_fields_alone_detect_film_in_busy_footage() {
    // Busy film footage can have idet call every frame interlaced, so the
    // progressive share reads as video while the pulldown signature is still
    // mechanically present.
    let busy = [window(240, 0, 96, 240); 3];
    let t = verdict(&busy, VideoFormat::Ntsc);
    assert!(t.detected());
    assert_eq!(t.pattern, Some(Pattern::ThreeTwo));
    // A mechanical marker is high confidence on its own.
    assert_eq!(t.confidence, Some(Confidence::High));

    // PAL 2:2 repeats no fields, so the same signal must not apply there.
    assert!(!verdict(&busy, VideoFormat::Pal).detected());
}

#[test]
fn the_dominance_counts_survive_the_verdict() {
    // These were summed into locals and never written back, so every verdict
    // carried 0 TFF / 0 BFF no matter what idet saw. `measured_field_order`
    // reads the two, finds a total of 0, and calls that "too few frames to
    // tell" — so field order silently fell back to the format default on
    // every file, which is the one thing this measurement exists to prevent.
    let t = verdict(&[video_window(), video_window()], VideoFormat::Ntsc);
    assert_eq!(t.tff_frames, 480);
    assert_eq!(t.bff_frames, 0);
    assert_eq!(t.measured_field_order(), Some(true));

    // And the other way round, so the test cannot pass on a hardcoded TFF.
    let bff = IdetCounts { tff: 0, bff: 240, prog: 0, rep: 0, rep_total: 240 };
    let t = verdict(&[bff, bff], VideoFormat::Ntsc);
    assert_eq!(t.bff_frames, 480);
    assert_eq!(t.measured_field_order(), Some(false));
}

/// A file that is clearly one dominance must be reported as such even when
/// the verdict returns early, which is the path every interlaced capture
/// takes: no film windows, so `verdict` bails before it reaches the end.
#[test]
fn an_early_return_still_reports_what_was_measured() {
    let t = verdict(&[video_window(); 5], VideoFormat::Ntsc);
    assert!(!t.detected(), "plain interlaced video is not telecined");
    assert_eq!(t.tff_frames, 1200, "but the counts still came back");
    assert_eq!(t.measured_field_order(), Some(true));
}

#[test]
fn a_minority_of_film_windows_is_rejected() {
    // One studio logo or one unusual scene must not swing the verdict.
    let mostly_video = [film_window(), video_window(), video_window(), video_window()];
    let t = verdict(&mostly_video, VideoFormat::Ntsc);
    assert!(!t.detected());
    assert_eq!(t.film_windows, 1);
    assert_eq!(t.windows, 4);
}

#[test]
fn exactly_half_the_windows_is_enough() {
    // The majority rule is `film * 2 >= windows`.
    let t = verdict(&[film_window(), video_window()], VideoFormat::Ntsc);
    assert!(t.detected());
}

#[test]
fn an_all_progressive_file_is_not_telecined() {
    // It needs neither IVTC nor QTGMC, which is a different recommendation
    // from "film transfer".
    let t = verdict(&[window(0, 240, 0, 240); 3], VideoFormat::Ntsc);
    assert!(t.progressive_source);
    assert!(!t.detected());
}

#[test]
fn windows_too_short_to_judge_are_skipped() {
    let t = verdict(&[window(10, 5, 0, 15), film_window()], VideoFormat::Ntsc);
    assert_eq!(t.windows, 1, "the 15-frame window is ignored");
}

#[test]
fn pattern_target_frame_rates() {
    assert!((Pattern::ThreeTwo.target_fps() - 23.976).abs() < 0.001);
    assert_eq!(Pattern::TwoTwo.target_fps(), 25.0);
}

// ── Sample planning ─────────────────────────────────────────────────────────

#[test]
fn a_short_file_gets_one_pass_from_the_top() {
    for duration in [0.0, -1.0, 10.0, 29.9] {
        let plan = sample_plan(duration, 29.97);
        assert_eq!(plan.len(), 1, "duration {duration}");
        assert_eq!(plan[0].start_seconds, 0.0);
        assert_eq!(plan[0].frames, 500);
    }
}

#[test]
fn longer_files_get_more_windows() {
    assert!(sample_plan(60.0, 29.97).len() <= 3);
    assert_eq!(sample_plan(300.0, 29.97).len(), 5);
    assert_eq!(sample_plan(3600.0, 29.97).len(), 7);
}

#[test]
fn the_plan_always_skips_the_opening_logo() {
    // A distributor logo or network bumper is rendered at the video frame rate
    // and would read as video, so no window may start at zero.
    let plan = sample_plan(3600.0, 29.97);
    assert!(plan[0].start_seconds >= 5.0, "{:?}", plan[0]);
}

#[test]
fn every_window_stays_inside_the_file() {
    for duration in [35.0, 60.0, 120.0, 300.0, 3600.0] {
        let plan = sample_plan(duration, 29.97);
        for w in &plan {
            let end = w.start_seconds + w.frames as f64 / 29.97;
            assert!(
                end <= duration || plan.len() == 1,
                "duration {duration}, window {w:?} ends at {end}"
            );
        }
    }
}

#[test]
fn a_nonsense_frame_rate_falls_back_to_25() {
    // Only the window length depends on fps, so the plans must match.
    for fps in [0.0, 1.0, -5.0] {
        assert_eq!(sample_plan(300.0, fps), sample_plan(300.0, 25.0), "fps {fps}");
    }
}

// ── Halo scan ───────────────────────────────────────────────────────────────

/// Vertical step edges, optionally carrying the two-sided sharpening signature.
///
/// The probe reads its near bands at offsets 4-7 either side of the edge and
/// its plateaus at 11-14, so the overshoot and undershoot must sit at 4-7 —
/// putting them adjacent to the edge lands them inside the gradient zone and
/// the scan sees nothing.
fn edge_plane(with_halo: bool, period: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = (320usize, 240usize);
    let edge = period / 2;
    let mut data = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let phase = x % period;
            let mut v: i32 = if phase < edge { 60 } else { 190 };
            if with_halo {
                // Bright-side overshoot, 4-7 past the edge.
                if (edge + 4..=edge + 7).contains(&phase) {
                    v = 230;
                }
                // Dark-side undershoot, 4-7 before it.
                if (edge - 7..=edge - 4).contains(&phase) {
                    v = 20;
                }
            }
            data[y * w + x] = v as u8;
        }
    }
    (data, w, h)
}

fn scan(data: &[u8], w: usize, h: usize) -> EdgeTally {
    let mut tally = EdgeTally::default();
    halo::scan_frame(&LumaPlane { data, width: w, height: h }, &mut tally);
    tally
}

#[test]
fn a_flat_frame_has_no_edges_to_judge() {
    for value in [0u8, 128, 255] {
        let data = vec![value; 320 * 240];
        let t = scan(&data, 320, 240);
        assert_eq!(t.total_edges, 0, "value {value}");
        let v = halo::verdict(t);
        assert!(!v.analyzed);
        assert_eq!(v.recommendation, Recommendation::None);
    }
}

#[test]
fn a_frame_too_small_to_scan_is_skipped_without_panicking() {
    // The probe reads up to 14 pixels either side, so a tiny frame would index
    // out of bounds.
    for (w, h) in [(32usize, 32usize), (63, 240), (320, 63), (1, 1)] {
        let data = vec![128u8; w * h];
        assert_eq!(scan(&data, w, h).total_edges, 0, "{w}x{h}");
    }
}

#[test]
fn clean_edges_score_lower_than_haloed_edges() {
    let (clean, w, h) = edge_plane(false, 40);
    let (haloed, _, _) = edge_plane(true, 40);

    let clean_v = halo::verdict(scan(&clean, w, h));
    let haloed_v = halo::verdict(scan(&haloed, w, h));

    assert!(clean_v.edges_analyzed > 0, "the test pattern has edges");
    assert!(
        haloed_v.halo_ratio > clean_v.halo_ratio,
        "clean {} vs haloed {}",
        clean_v.halo_ratio,
        haloed_v.halo_ratio
    );
    assert_eq!(clean_v.halo_level, HaloLevel::Clean);
    assert_eq!(clean_v.recommendation, Recommendation::None);
    // And the haloed pattern must actually be detected, or this test would
    // pass on two planes that both score zero.
    assert!(haloed_v.halo_ratio > 0.10, "ratio {}", haloed_v.halo_ratio);
    assert_ne!(haloed_v.halo_level, HaloLevel::Clean);
}

#[test]
fn busy_texture_does_not_vote() {
    // Random texture has strong gradients but no settled plateaus. If it voted,
    // every detailed scene would read as haloed.
    let mut state = 12345u64;
    let (w, h) = (320usize, 240usize);
    let data: Vec<u8> = (0..w * h)
        .map(|_| {
            // xorshift, so the pattern is deterministic across runs.
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state & 0xff) as u8
        })
        .collect();
    let v = halo::verdict(scan(&data, w, h));
    assert_eq!(v.halo_level, HaloLevel::Clean, "ratio was {}", v.halo_ratio);
}

#[test]
fn halo_thresholds_map_to_recommendations() {
    let at = |halo_edges: u64| {
        halo::verdict(EdgeTally { total_edges: 1000, halo_edges }).recommendation
    };
    assert_eq!(at(100), Recommendation::None); // 10.0% — not above 0.10
    assert_eq!(at(101), Recommendation::Light); // 10.1%
    assert_eq!(at(181), Recommendation::Light); // 18.1%
    assert_eq!(at(301), Recommendation::Strong); // 30.1%
}

#[test]
fn too_few_edges_reports_clean_rather_than_guessing() {
    // Flat or dark content cannot be judged, so it must not be called haloed.
    let v = halo::verdict(EdgeTally { total_edges: 49, halo_edges: 49 });
    assert_eq!(v.halo_level, HaloLevel::Clean);
    assert_eq!(v.halo_ratio, 0.0);
    assert!(v.analyzed, "edges were seen, just not enough of them");
}

// ── Chroma noise ───────────────────────────────────────────────────────────
//
// The numbers below are the measured calibration set: a real VHS capture with
// known chroma noise added, and five real captures with none. See
// `classify::chroma_noise`.

#[test]
fn chroma_noise_is_detected_when_the_luma_looks_clean() {
    // TOUT cannot see chroma noise at all — measured, it does not move from
    // sigma 0 to sigma 40 — so this case used to be reported as clean.
    let at = |ydif: f64, udif: f64, vdif: f64| {
        noise(&[stats(&[
            ("TOUT", 0.001),
            ("YDIF", ydif),
            ("UDIF", udif),
            ("VDIF", vdif),
        ])])
    };

    // As captured: chroma 0.83, ratio 0.14.
    assert_eq!(at(6.01, 0.95, 0.71).noise_level, NoiseVerdict::Clean);

    // Added chroma sigma 5: chroma 3.01, ratio 0.50.
    let mild = at(6.01, 3.2, 2.82);
    assert_eq!(mild.noise_level, NoiseVerdict::Moderate);
    assert_eq!(mild.trigger, NoiseTrigger::Chroma);
    // The luma verdict is kept alongside, and still says clean.
    assert_eq!(mild.luma_level, NoiseVerdict::Clean);

    // Added chroma sigma 10: chroma 6.18, ratio 1.03.
    let strong = at(6.01, 6.4, 5.96);
    assert_eq!(strong.noise_level, NoiseVerdict::Heavy);
    assert_eq!(strong.trigger, NoiseTrigger::Chroma);
    assert_eq!(strong.recommendation, Recommendation::Heavy);
}

#[test]
fn a_busy_clean_clip_is_not_mistaken_for_chroma_noise() {
    // The measured trap: a restored PAL DV clip full of movement has a chroma
    // difference of 5.56 — higher than the genuinely noisy sigma-5 case —
    // purely from motion. Only the ratio tells them apart, which is why the
    // classifier needs both a ratio and an absolute floor.
    let busy = noise(&[stats(&[
        ("TOUT", 0.0189),
        ("YDIF", 31.57),
        ("UDIF", 5.91),
        ("VDIF", 5.20),
    ])]);
    assert_eq!(busy.chroma_level, NoiseVerdict::Clean);
    assert!(busy.avg_chroma_diff > 4.0, "the raw figure alone would have tripped");
    assert!(busy.chroma_ratio < 0.45);
}

#[test]
fn luma_noise_does_not_leak_into_the_chroma_verdict() {
    // Adding luma noise pushes the ratio *down* (measured: 0.031 at sigma 40),
    // so a grainy-luma tape can never false-positive as chroma noise.
    let grainy = noise(&[stats(&[
        ("TOUT", 0.0048),
        ("YDIF", 27.14),
        ("UDIF", 0.95),
        ("VDIF", 0.71),
    ])]);
    assert_eq!(grainy.chroma_level, NoiseVerdict::Clean);
    assert!(grainy.chroma_ratio < 0.1);
}

#[test]
fn a_still_frame_does_not_divide_by_zero_into_a_heavy_verdict() {
    // No luma movement at all would make the ratio infinite and call
    // everything heavy. The guard turns that into a clean verdict instead.
    let still = noise(&[stats(&[("TOUT", 0.0), ("YDIF", 0.0), ("UDIF", 9.0), ("VDIF", 9.0)])]);
    assert_eq!(still.chroma_ratio, 0.0);
    assert_eq!(still.chroma_level, NoiseVerdict::Clean);
}

#[test]
fn udif_and_vdif_are_parsed() {
    let r = stats(&[("UDIF", 1.25), ("VDIF", 0.75), ("YDIF", 6.0)]);
    assert_eq!(r.udif, vec![1.25]);
    assert_eq!(r.vdif, vec![0.75]);
    // UDIF must not be mistaken for YDIF by the substring match.
    assert_eq!(r.ydif, vec![6.0]);
}
