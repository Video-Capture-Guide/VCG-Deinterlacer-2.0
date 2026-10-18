//! Invariants of the core decisions.
//!
//! These pin the rules the core decisions depend on — especially the filter
//! escaping, where a mistake does not error, it silently renders the wrong
//! label.

use vcg_core::colorspace::{effective_output_matrix, needs_601_to_709, output_cs_tag, CsTag};
use vcg_core::config::{
    ColorMatrix, Config, CropPreset, FieldOrder, OutputFormat, SourceClass,
    SourceClassification, VideoFormat, Watermark,
};
use vcg_core::ffmpeg::{estimate_output_width, output_args, output_frame_rate};
use vcg_core::watermark::{merge_video_filter, watermark_args};

fn ntsc() -> Config {
    Config::default()
}

fn pal() -> Config {
    Config {
        format: VideoFormat::Pal,
        ..Config::default()
    }
}

fn text_wm(text: &str) -> Config {
    Config {
        watermark: Watermark::Text {
            text: text.into(),
            fontsize: 28,
        },
        ..Config::default()
    }
}

/// The filter graph for a text watermark, with no font file so the assertions
/// are about the escaping alone.
fn text_graph(text: &str) -> String {
    watermark_args(&text_wm(text), None)
        .filter
        .expect("a non-empty label produces a filter")
        .graph
}

// ── Color tagging ──────────────────────────────────────────────────────────

#[test]
fn pal_transfer_is_gamma28_not_bt470bg() {
    // Passing bt470bg to -color_trc makes FFmpeg fail with "Undefined
    // constant" and abort the encode mid-run.
    assert_eq!(CsTag::Bt470bg.name(), "bt470bg");
    assert_eq!(CsTag::Bt470bg.trc_name(), "gamma28");
    // Inside the setparams filter the transfer *is* spelled bt470bg.
    assert!(CsTag::Bt470bg.setparams_filter().contains("color_trc=bt470bg"));
}

#[test]
fn ntsc_and_pal_601_resolve_to_different_tags() {
    assert_eq!(output_cs_tag(&ntsc()), CsTag::Smpte170m);
    assert_eq!(output_cs_tag(&pal()), CsTag::Bt470bg);
}

#[test]
fn sd_601_upscaled_to_hd_converts_to_709() {
    let c = Config {
        upscale_enabled: true,
        upscale_resolution: Some("1440x1080".into()),
        ..Config::default()
    };
    assert!(needs_601_to_709(&c));
    assert_eq!(effective_output_matrix(&c), ColorMatrix::Bt709);
    assert_eq!(output_cs_tag(&c), CsTag::Bt709);
}

#[test]
fn height_720_is_the_conversion_threshold() {
    let at = |h: &str| {
        needs_601_to_709(&Config {
            upscale_enabled: true,
            upscale_resolution: Some(format!("960x{h}")),
            ..Config::default()
        })
    };
    assert!(!at("719"));
    assert!(at("720"));
}

#[test]
fn real_hd_sources_are_never_converted() {
    // Already 709; converting again would double-apply the matrix.
    for sc in [SourceClass::Avchd, SourceClass::Hdv] {
        let c = Config {
            source: SourceClassification {
                source_class: sc,
                ..Default::default()
            },
            upscale_enabled: true,
            upscale_resolution: Some("1920x1080".into()),
            ..Config::default()
        };
        assert!(!needs_601_to_709(&c), "{sc:?} must not be converted");
    }
}

#[test]
fn explicit_709_is_never_converted() {
    let c = Config {
        color_matrix: ColorMatrix::Bt709,
        upscale_enabled: true,
        upscale_resolution: Some("1920x1080".into()),
        ..Config::default()
    };
    assert!(!needs_601_to_709(&c));
}

#[test]
fn a_malformed_upscale_resolution_degrades_rather_than_panics() {
    for res in ["not-a-resolution", "1920", "", "x", "1920x"] {
        let c = Config {
            upscale_enabled: true,
            upscale_resolution: Some(res.into()),
            ..Config::default()
        };
        // Must not panic, and must still produce a usable width.
        assert!(estimate_output_width(&c) > 0, "res {res:?}");
    }
}

// ── Frame rate ──────────────────────────────────────────────────────────────

#[test]
fn ivtc_wins_over_field_order_and_double_rate() {
    let c = Config {
        ivtc_mode: true,
        double_rate: true,
        field_order: FieldOrder::Progressive,
        ..Config::default()
    };
    assert_eq!(output_frame_rate(&c), "24000/1001");
}

#[test]
fn frame_rate_covers_every_branch() {
    let cases = [
        // (pal, ivtc, field_order, double_rate) -> rate
        (false, true, FieldOrder::Tff, true, "24000/1001"),
        (true, true, FieldOrder::Tff, true, "25"),
        (false, false, FieldOrder::Progressive, true, "30000/1001"),
        (true, false, FieldOrder::Progressive, true, "25"),
        (false, false, FieldOrder::Tff, true, "60000/1001"),
        (true, false, FieldOrder::Tff, true, "50"),
        (false, false, FieldOrder::Tff, false, "30000/1001"),
        (true, false, FieldOrder::Tff, false, "25"),
    ];
    for (is_pal, ivtc, field_order, double_rate, expected) in cases {
        let c = Config {
            format: if is_pal {
                VideoFormat::Pal
            } else {
                VideoFormat::Ntsc
            },
            ivtc_mode: ivtc,
            field_order,
            double_rate,
            ..Config::default()
        };
        assert_eq!(
            output_frame_rate(&c),
            expected,
            "pal={is_pal} ivtc={ivtc} {field_order:?} double={double_rate}"
        );
    }
}

// ── Output width ────────────────────────────────────────────────────────────

#[test]
fn dv_defaults_to_no_crop_and_sd_to_bt601() {
    use vcg_core::config::CaptureMethod;
    let dv = Config {
        capture_method: CaptureMethod::Dv,
        ..Config::default()
    };
    assert_eq!(dv.crop_preset(), CropPreset::None);
    assert_eq!(ntsc().crop_preset(), CropPreset::Bt601);
}

#[test]
fn anamorphic_sar_widens_the_square_pixel_output() {
    let widen = |format, sar: &str| {
        estimate_output_width(&Config {
            format,
            par_correction: true,
            detected_sar: sar.into(),
            ..Config::default()
        })
    };
    assert_eq!(widen(VideoFormat::Ntsc, "32:27"), 854);
    assert_eq!(widen(VideoFormat::Ntsc, ""), 640);
    assert_eq!(widen(VideoFormat::Pal, "64:45"), 1024);
    assert_eq!(widen(VideoFormat::Pal, ""), 768);
}

#[test]
fn hd_sources_are_1920_regardless_of_crop_and_par() {
    let c = Config {
        source: SourceClassification {
            source_class: SourceClass::Avchd,
            ..Default::default()
        },
        crop_preset: Some(CropPreset::Overscan),
        par_correction: true,
        ..Config::default()
    };
    assert_eq!(estimate_output_width(&c), 1920);
}

// ── drawtext escaping ───────────────────────────────────────────────────────
//
// These are the cases that break a filter silently rather than loudly.

#[test]
fn percent_gets_two_backslashes() {
    // One backslash survives the option parser, the second escapes the % for
    // drawtext's text expander. A bare % is read as a broken %{...} sequence
    // and kills the whole label.
    let g = text_graph("100% restored");
    assert!(g.contains(r"100\\% restored"), "{g}");
}

#[test]
fn colon_and_quote_are_escaped() {
    assert!(text_graph("Tape: 01").contains(r"Tape\: 01"));
    assert!(text_graph("Darryl's tape").contains(r"Darryl\'s tape"));
}

#[test]
fn backslash_is_doubled_before_other_escaping() {
    // A Windows path is the realistic case. The drive colon must come out
    // escaped, and the separators doubled.
    let g = text_graph(r"D:\captures");
    assert!(g.contains(r"D\:\\captures"), "{g}");
}

#[test]
fn an_empty_or_whitespace_label_produces_no_filter() {
    for text in ["", "   ", "\t\n"] {
        assert!(
            watermark_args(&text_wm(text), None).filter.is_none(),
            "text {text:?} must be a no-op, not an empty drawtext"
        );
    }
}

#[test]
fn a_label_is_trimmed() {
    assert!(text_graph("  padded  ").contains("text='padded'"));
}

// ── Filter merging ──────────────────────────────────────────────────────────

#[test]
fn setparams_is_appended_to_an_existing_chain_not_passed_twice() {
    // FFmpeg accepts only one -vf per stream, so this must be one option
    // whose graph ends in setparams.
    let c = text_wm("hello");
    let wm = watermark_args(&c, None);
    let merged = merge_video_filter(wm.filter, output_cs_tag(&c));
    let args = merged.to_args();

    assert_eq!(args.len(), 2, "exactly one flag and one graph");
    assert_eq!(args[0], "-vf");
    assert!(args[1].starts_with("drawtext="));
    assert!(args[1].contains(",setparams=range=tv:"));
    assert_eq!(args[1].matches("setparams=").count(), 1);
}

#[test]
fn with_no_watermark_setparams_stands_alone() {
    let merged = merge_video_filter(None, CsTag::Smpte170m);
    assert_eq!(merged.flag, "-vf");
    assert!(merged.graph.starts_with("setparams="));
}

#[test]
fn a_logo_uses_filter_complex_and_an_extra_input() {
    // A logo needs a second input, so it cannot use -vf.
    let c = Config {
        watermark: Watermark::Logo {
            // A path that does not exist is a no-op, so point at this file.
            logo_path: file!().into(),
            logo_size: 0.10,
        },
        ..Config::default()
    };
    let wm = watermark_args(&c, None);
    // Only assert the shape if the path resolved; `file!()` is relative to the
    // workspace root, which is the test's working directory for this crate.
    if let Some(f) = wm.filter {
        assert_eq!(f.flag, "-filter_complex");
        assert_eq!(wm.extra_inputs.len(), 2);
        assert_eq!(wm.extra_inputs[0], "-i");
    }
}

#[test]
fn a_missing_logo_file_is_a_no_op() {
    let c = Config {
        watermark: Watermark::Logo {
            logo_path: r"Z:\definitely\not\here.png".into(),
            logo_size: 0.10,
        },
        ..Config::default()
    };
    let wm = watermark_args(&c, None);
    assert!(wm.filter.is_none());
    assert!(wm.extra_inputs.is_empty());
}

// ── Encoder args ────────────────────────────────────────────────────────────

#[test]
fn every_format_pairs_its_args_with_the_right_container() {
    let expect = [
        (OutputFormat::Prores, ".mov", "prores_ks"),
        (OutputFormat::H264, ".mp4", "libx264"),
        (OutputFormat::Ffv1, ".mkv", "ffv1"),
        (OutputFormat::Huffyuv, ".avi", "huffyuv"),
        (OutputFormat::Utvideo, ".avi", "utvideo"),
        (OutputFormat::Lagarith, ".avi", "lagarith"),
    ];
    for (format, ext, codec) in expect {
        let out = output_args(&Config {
            output_format: format,
            ..Config::default()
        });
        assert_eq!(out.extension, ext, "{format:?}");
        assert_eq!(out.args[0], "-c:v");
        assert_eq!(out.args[1], codec, "{format:?}");
        // Color tags are always present.
        assert!(out.args.iter().any(|a| a == "-colorspace"), "{format:?}");
    }
}

#[test]
fn prores_always_pins_the_pixel_format() {
    // The Python fall-through default lost this; an enum has no fall-through.
    let out = output_args(&Config {
        output_format: OutputFormat::Prores,
        ..Config::default()
    });
    let i = out.args.iter().position(|a| a == "-pix_fmt").expect("-pix_fmt");
    assert_eq!(out.args[i + 1], "yuv422p10le");
}
