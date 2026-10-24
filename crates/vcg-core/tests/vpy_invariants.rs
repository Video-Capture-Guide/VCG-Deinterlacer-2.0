//! Structural invariants of the generated VapourSynth script.
//!
//! These pin the orderings the pipeline actually depends on — the ones that
//! otherwise reveal themselves only as a crash on a DVD rip, or a dark and
//! oversaturated render.

use vcg_core::config::{
    CaptureMethod, ColorMatrix, Config, CropPreset, DitherMethod, FieldOrder,
    LevelsAdjustment, OutputFormat,
    SourceClass, SourceClassification, VideoFormat,
};
use vcg_core::vpy::{generate_vpy, DepsPaths};

/// A deps root that does not exist, so the plugin-loading block is omitted and
/// the assertions are about the pipeline rather than the environment.
fn deps() -> DepsPaths {
    DepsPaths::new(r"Z:\no\such\deps")
}

fn script(config: &Config) -> String {
    generate_vpy(config, &deps())
}

/// Index of the first line containing `needle`.
fn pos(script: &str, needle: &str) -> usize {
    script
        .lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not found in:\n{script}"))
}

fn hd(source_class: SourceClass, par_needed: bool) -> SourceClassification {
    SourceClassification {
        source_class,
        par_needed,
        ..Default::default()
    }
}

// ── Stage ordering ──────────────────────────────────────────────────────────

#[test]
fn deferred_vertical_crop_lands_after_deinterlacing() {
    // The whole reason the crop is split: SeparateFields needs each field to
    // have whole chroma rows, so 4:2:0 luma height must stay mod 4. The
    // overscan crop (480-6=474) violates that and crashed on DVD rips.
    for preset in [CropPreset::Overscan, CropPreset::Manual] {
        let s = script(&Config {
            crop_preset: Some(preset),
            crop_top: 2,
            crop_bottom: 4,
            ..Config::default()
        });
        let qtgmc = pos(&s, "haf.QTGMC(");
        let vertical = pos(&s, "clip = core.std.Crop(clip, top=");
        assert!(
            vertical > qtgmc,
            "{preset:?}: vertical crop at {vertical} must follow QTGMC at {qtgmc}"
        );
    }
}

#[test]
fn horizontal_crop_precedes_deinterlacing() {
    // Horizontal crop stays early so the whole pipeline works on the narrower
    // frame.
    let s = script(&Config {
        crop_preset: Some(CropPreset::Overscan),
        ..Config::default()
    });
    assert!(pos(&s, "core.std.Crop(clip, left=8, right=8)") < pos(&s, "haf.QTGMC("));
}

#[test]
fn trim_runs_on_source_frames_before_any_filtering() {
    // Trimming after deinterlacing would waste work on cut material and
    // desync the separately-trimmed audio.
    let s = script(&Config {
        trim_ranges: vec![(100, 500)],
        ..Config::default()
    });
    let trim = pos(&s, "_vcg_trim(_vcg_src,");
    assert!(trim < pos(&s, "core.fmtc.bitdepth(clip, bits=16)"));
    assert!(trim < pos(&s, "haf.QTGMC("));
}

#[test]
fn pixel_format_normalisation_precedes_everything() {
    // VFM requires standard 8-bit YUV, so a 10-bit source must be converted
    // before any other filter sees it.
    let s = script(&Config {
        source: SourceClassification {
            needs_pixfmt_conversion: true,
            pix_fmt: "yuv420p10le".into(),
            ..Default::default()
        },
        trim_ranges: vec![(0, 10)],
        ..Config::default()
    });
    let norm = pos(&s, "format=vs.YUV420P8");
    assert!(norm < pos(&s, "_vcg_trim(_vcg_src,"));
    assert!(norm < pos(&s, "core.fmtc.bitdepth(clip, bits=16)"));
}

#[test]
fn matrix_conversion_follows_the_upscale_that_triggers_it() {
    // The conversion exists *because* the upscale made the frame HD-sized.
    let s = script(&Config {
        upscale_enabled: true,
        upscale_resolution: Some("1440x1080".into()),
        ..Config::default()
    });
    assert!(pos(&s, "core.nnedi3.nnedi3") < pos(&s, "mats=\"601\", matd=\"709\""));
}

#[test]
fn grain_is_added_after_upscaling_but_before_dithering() {
    // After the upscale so grain stays pixel-fine; before dither while the
    // pipeline is still 16-bit.
    let s = script(&Config {
        upscale_enabled: true,
        grain_strength: 2.0,
        ..Config::default()
    });
    let grain = pos(&s, "core.grain.Add");
    assert!(pos(&s, "core.nnedi3.nnedi3") < grain);
    // Match the dither stage's own comment: `fmtc.bitdepth` also appears near
    // the top for the 16-bit lift, which would match first.
    assert!(grain < pos(&s, "# Dither 16-bit"));
}

#[test]
fn the_script_ends_by_setting_output() {
    assert!(script(&Config::default()).ends_with("clip.set_output()"));
}

// ── Deinterlacing ───────────────────────────────────────────────────────────

#[test]
fn qtgmc_last_argument_carries_no_trailing_comma() {
    // A trailing comma before `)` is legal Python, but the Python builds the
    // list with commas and strips the last one; emitting a stray one here
    // would be a silent text divergence.
    let s = script(&Config::default());
    let lines: Vec<&str> = s.lines().collect();
    // Scan from the QTGMC call — the source-loader block also closes with `)`.
    let open = pos(&s, "haf.QTGMC(");
    let close = open
        + lines[open..]
            .iter()
            .position(|l| *l == ")")
            .expect("QTGMC call is closed");
    assert_eq!(lines[close - 1].trim(), "NoiseProcess=0");
}

#[test]
fn progressive_sources_skip_deinterlacing_entirely() {
    let s = script(&Config {
        field_order: FieldOrder::Progressive,
        ..Config::default()
    });
    assert!(s.contains("# Progressive source — deinterlacing skipped"));
    assert!(!s.contains("haf.QTGMC("));
    assert!(!s.contains("core.vivtc.VFM"));
}

#[test]
fn pal_ivtc_does_not_decimate() {
    // PAL 2:2 has no duplicate frames; VDecimate (cycle=5) would discard every
    // 5th real frame and produce 20fps judder.
    let pal = script(&Config {
        format: VideoFormat::Pal,
        ivtc_mode: true,
        ..Config::default()
    });
    assert!(pal.contains("core.vivtc.VFM"));
    assert!(!pal.contains("VDecimate"));

    let ntsc = script(&Config {
        ivtc_mode: true,
        ..Config::default()
    });
    assert!(ntsc.contains("core.vivtc.VDecimate(clip)"));
}

#[test]
fn ivtc_field_match_runs_on_an_8_bit_copy() {
    // VFM only accepts 8-bit YUV, but the pipeline is at 16-bit; clip2 pulls
    // the output frames from the 16-bit clip so no precision is lost.
    let s = script(&Config {
        ivtc_mode: true,
        ..Config::default()
    });
    assert!(s.contains("_vfm8 = core.fmtc.bitdepth(clip, bits=8)"));
    assert!(s.contains("clip2=clip)"));
}

#[test]
fn field_order_maps_to_tff_and_vfm_order() {
    let tff = script(&Config::default());
    assert!(tff.contains("TFF=True,"));

    let bff = script(&Config {
        field_order: FieldOrder::Bff,
        ..Config::default()
    });
    assert!(bff.contains("TFF=False,"));

    // VFM spells the same thing as order=1/0.
    let ivtc_tff = script(&Config {
        ivtc_mode: true,
        ..Config::default()
    });
    assert!(ivtc_tff.contains("order=1,"));
    let ivtc_bff = script(&Config {
        ivtc_mode: true,
        field_order: FieldOrder::Bff,
        ..Config::default()
    });
    assert!(ivtc_bff.contains("order=0,"));
}

// ── HD sources ──────────────────────────────────────────────────────────────

#[test]
fn hd_sources_skip_crop_and_par_correction() {
    // An HD source is already square-pixel 709 with no blanking to remove, so
    // the user's crop choice is overridden and PAR correction must not run.
    for sc in [SourceClass::Avchd, SourceClass::Hdv] {
        let s = script(&Config {
            source: hd(sc, false),
            // Both of these would otherwise emit something.
            crop_preset: Some(CropPreset::Overscan),
            par_correction: true,
            ..Config::default()
        });
        assert!(!s.contains("core.std.Crop(clip, left="), "{sc:?} cropped");
        assert!(!s.contains("# PAR correction → "), "{sc:?} PAR-corrected");
        assert!(!s.contains("_SARNum="), "{sc:?} had SAR set");
    }
}

#[test]
fn only_anamorphic_hd_gets_rescaled_to_1920() {
    // HDV and some Sony/Panasonic AVCHD record 1440x1080 non-square; Canon and
    // most AVCHD record 1920x1080 square and must be left alone.
    let anamorphic = script(&Config {
        source: hd(SourceClass::Hdv, true),
        ..Config::default()
    });
    assert!(anamorphic.contains("width=1920, height=1080"));

    let square = script(&Config {
        source: hd(SourceClass::Hdv, false),
        ..Config::default()
    });
    assert!(!square.contains("width=1920, height=1080"));
}

#[test]
fn avchd_and_hdv_try_ffms2_before_lsmas() {
    // Files moved out of their BDMV directory have corrupt duration metadata,
    // so lsmas reports millions of frames and fails past the real end; ffms2
    // scans the file physically.
    for sc in [SourceClass::Avchd, SourceClass::Hdv] {
        let s = script(&Config {
            source: hd(sc, false),
            ..Config::default()
        });
        assert!(pos(&s, "ffms2.Source") < pos(&s, "lsmas.LWLibavSource"), "{sc:?}");
    }

    // SD keeps lsmas first for performance on normal files.
    let sd = script(&Config::default());
    assert!(pos(&sd, "lsmas.LWLibavSource") < pos(&sd, "ffms2.Source"));
}

// ── Sizing and tagging ──────────────────────────────────────────────────────

#[test]
fn overscan_sets_sar_instead_of_resizing() {
    // The cropped dimensions are preserved exactly; SAR makes the player show
    // them at 4:3.
    let ntsc = script(&Config {
        crop_preset: Some(CropPreset::Overscan),
        ..Config::default()
    });
    assert!(ntsc.contains("_SARNum=79, _SARDen=88"));
    assert!(!ntsc.contains("# PAR correction → "));

    let pal = script(&Config {
        format: VideoFormat::Pal,
        crop_preset: Some(CropPreset::Overscan),
        ..Config::default()
    });
    assert!(pal.contains("_SARNum=95, _SARDen=88"));
}

#[test]
fn par_correction_can_be_turned_off() {
    let s = script(&Config {
        par_correction: false,
        ..Config::default()
    });
    assert!(!s.contains("# PAR correction → "));
    assert!(!s.contains("_SARNum="));
}

#[test]
fn dv_with_no_crop_still_gets_par_correction() {
    // The "no crop path" branch exists precisely for this.
    let s = script(&Config {
        capture_method: CaptureMethod::Dv,
        ..Config::default()
    });
    assert_eq!(s.matches("# PAR correction → ").count(), 1);
    assert!(s.contains("(no crop path)"));
    assert!(!s.contains("core.std.Crop(clip, left="));
}

#[test]
fn colorspace_tag_follows_the_converted_pixels() {
    // An SD 601 source upscaled to HD has its pixels converted, so the tag
    // must read 709 (_Matrix=1) to match.
    let upscaled = script(&Config {
        upscale_enabled: true,
        upscale_resolution: Some("1440x1080".into()),
        ..Config::default()
    });
    assert!(upscaled.contains("_Matrix=1, _Primaries=1, _Transfer=1"));

    // NTSC 601 stays SMPTE170M (6), PAL 601 stays BT.470BG (5).
    assert!(script(&Config::default()).contains("_Matrix=6"));
    assert!(script(&Config {
        format: VideoFormat::Pal,
        ..Config::default()
    })
    .contains("_Matrix=5"));
    assert!(script(&Config {
        color_matrix: ColorMatrix::Bt709,
        ..Config::default()
    })
    .contains("_Matrix=1"));
}

// ── Dither ──────────────────────────────────────────────────────────────────

#[test]
fn prores_dithers_to_10_bit_and_everything_else_to_8() {
    let prores = script(&Config::default());
    assert!(prores.contains("bits=10, dmode=3"));

    for fmt in [
        OutputFormat::H264,
        OutputFormat::Ffv1,
        OutputFormat::Huffyuv,
        OutputFormat::Utvideo,
        OutputFormat::Lagarith,
    ] {
        let s = script(&Config {
            output_format: fmt,
            ..Config::default()
        });
        assert!(s.contains("bits=8, dmode=3"), "{fmt:?}");
    }
}

#[test]
fn dither_method_selects_the_fmtc_dmode() {
    assert!(script(&Config::default()).contains("dmode=3"));
    assert!(script(&Config {
        dither_method: DitherMethod::Ordered,
        ..Config::default()
    })
    .contains("dmode=0"));
}

#[test]
fn disabling_dither_drops_the_fmtc_call() {
    let s = script(&Config {
        dither_enabled: false,
        ..Config::default()
    });
    assert!(!s.contains("fmtc.bitdepth(clip, bits=10"));
    assert!(s.contains("format=vs.YUV422P10"));
}

// ── Formatting ──────────────────────────────────────────────────────────────

#[test]
fn grain_comment_uses_python_g_formatting() {
    // Python's `{:g}` drops trailing zeros: 2.0 prints as "2", not "2.0".
    let at = |g: f64| {
        let s = script(&Config {
            grain_strength: g,
            ..Config::default()
        });
        s.lines()
            .find(|l| l.starts_with("# Film grain overlay"))
            .expect("grain comment")
            .to_string()
    };
    assert!(at(2.0).ends_with("var 2)"));
    assert!(at(2.5).ends_with("var 2.5)"));
    assert!(at(10.0).ends_with("var 10)"));
    assert!(at(0.05).ends_with("var 0.05)"));
}

#[test]
fn zero_grain_emits_nothing() {
    assert!(!script(&Config::default()).contains("core.grain.Add"));
}

#[test]
fn zero_yc_delay_emits_nothing() {
    assert!(!script(&Config::default()).contains("Y/C delay"));
    let shifted = script(&Config {
        yc_delay: -4,
        ..Config::default()
    });
    // The comment carries an explicit sign.
    assert!(shifted.contains("shift: -4 px"));
    assert!(shifted.contains("src_left=-4"));
    let plus = script(&Config {
        yc_delay: 4,
        ..Config::default()
    });
    assert!(plus.contains("shift: +4 px"));
}

#[test]
fn an_embedded_path_has_its_backslashes_doubled() {
    let s = script(&Config {
        input_path: r"D:\captures\tape 01.avi".into(),
        ..Config::default()
    });
    assert!(s.contains(r"r"), "paths are emitted as raw strings");
    assert!(s.contains(r"D:\\captures\\tape 01.avi"));
}

// ── The bottom crop ─────────────────────────────────────────────────────────

/// The head-switching band used to be its own setting with its own toggle,
/// added to the manual bottom crop at render time. That let the bottom be
/// cropped twice — once by each — with neither control showing the total. It
/// is now one number, and the only thing the band field does is say how much
/// of that number it accounts for.
#[test]
fn the_bottom_crop_is_a_single_number() {
    let config = Config {
        crop_preset: Some(CropPreset::Manual),
        crop_top: 0,
        crop_bottom: 12,
        // Informational only: already inside crop_bottom.
        head_switching_rows: 12,
        ..Config::default()
    };
    let s = script(&config);
    assert!(s.contains("_vcg_t, _vcg_b = 0, 12"), "{s}");
    assert!(
        !s.contains("_vcg_hs"),
        "the band must not be cropped a second time:
{s}"
    );
}

/// Whatever it is made of, the bottom crop lands after deinterlacing: taking
/// lines off an interlaced frame would discard half of each one.
#[test]
fn the_bottom_crop_still_waits_for_deinterlacing() {
    let config = Config {
        crop_preset: Some(CropPreset::Manual),
        crop_bottom: 12,
        ..Config::default()
    };
    let s = script(&config);
    assert!(pos(&s, "_vcg_t, _vcg_b = 0, 12") > pos(&s, "haf.QTGMC("));
}

// ── Keeping the shape ───────────────────────────────────────────────────────

/// How far a region's shape is from what a 640x480 (or 768x576) output claims.
/// Above 1.0 is too tall, below is too wide.
fn shape_error(kept_w: f64, kept_h: f64, target_h: f64, reference: f64) -> f64 {
    kept_w / (reference * kept_h / target_h)
}

/// The arithmetic, checked against the real numbers rather than trusted.
///
/// This mirrors the expression the script emits. If one changes without the
/// other, the comments stop meaning anything — so the shared helper is
/// compared against it here too.
#[test]
fn the_trim_is_what_the_shape_actually_needs() {
    // A real capture: 720 wide cropped to 692, 480 tall cropped to 470.
    let (kept_w, kept_h, target_h) = (692.0_f64, 470.0_f64, 480.0_f64);

    let needed = 704.0 * kept_h / target_h;
    assert!((needed - 689.33).abs() < 0.1, "needs {needed:.2} columns");

    let ax = ((kept_w - needed) / 4.0).round() * 4.0;
    assert_eq!(ax, 4.0, "four columns, two a side — not the sixteen I first had");
    assert_eq!(
        Config::aspect_trim(kept_w as u32, kept_h as u32, target_h as u32, 704),
        ax as u32,
        "the helper the Crop page displays must agree with the script"
    );

    // Trimming must leave the shape closer to honest than leaving it alone.
    let before = shape_error(kept_w, kept_h, target_h, 704.0);
    let after = shape_error(kept_w - ax, kept_h, target_h, 704.0);
    assert!(after.abs_sub_one() < before.abs_sub_one(), "{before} -> {after}");
    assert!(after.abs_sub_one() < 0.005, "left {:.2}% off", (after - 1.0) * 100.0);
}

trait AbsSubOne {
    fn abs_sub_one(self) -> f64;
}

impl AbsSubOne for f64 {
    fn abs_sub_one(self) -> f64 {
        (self - 1.0).abs()
    }
}

/// The failure this replaced: trimming in proportion to the rows removed
/// assumed the cropped width was 4:3 by definition. It is not — the raster
/// decides that — and on the measured sample the "fix" took the shape error
/// from +0.39% to -1.93%.
#[test]
fn the_old_proportional_trim_would_have_made_it_worse() {
    let (kept_w, kept_h, target_h) = (692.0_f64, 470.0_f64, 480.0_f64);
    let untouched = shape_error(kept_w, kept_h, target_h, 704.0);
    let old_way = shape_error(kept_w - 16.0, kept_h, target_h, 704.0);
    let new_way = shape_error(
        kept_w - Config::aspect_trim(kept_w as u32, kept_h as u32, target_h as u32, 704) as f64,
        kept_h,
        target_h,
        704.0,
    );
    assert!(old_way.abs_sub_one() > untouched.abs_sub_one(), "the bug");
    assert!(new_way.abs_sub_one() < untouched.abs_sub_one(), "the fix");
}

#[test]
fn the_preset_region_is_already_the_right_shape() {
    // 704x480 is the reference, so there is nothing to trim.
    assert_eq!(Config::aspect_trim(704, 480, 480, 704), 0);
    assert_eq!(Config::aspect_trim(704, 576, 576, 704), 0);
    // And DV, whose whole 720 is active.
    assert_eq!(Config::aspect_trim(720, 480, 480, 720), 0);
}

/// A region that is already too narrow cannot be fixed by trimming — widening
/// it would mean putting blanking back.
#[test]
fn an_over_trimmed_region_is_left_alone() {
    assert_eq!(Config::aspect_trim(692, 480, 480, 704), 0);
}

#[test]
fn pal_uses_its_own_target_height() {
    // PAL_DV: 720 cropped to 702, 576 cropped to 568 by an 8-row band.
    let trim = Config::aspect_trim(702, 568, 576, 704);
    assert_eq!(trim, 8);
    let after = shape_error(702.0 - trim as f64, 568.0, 576.0, 704.0);
    assert!(after.abs_sub_one() < 0.005, "{:.2}% off", (after - 1.0) * 100.0);
}

#[test]
fn the_compensation_follows_the_vertical_crop_and_precedes_the_resize() {
    let config = Config {
        crop_preset: Some(CropPreset::Manual),
        crop_left: 14,
        crop_right: 14,
        crop_bottom: 10,
        ..Config::default()
    };
    let s = script(&config);
    assert!(s.contains("_vcg_need = 704 * clip.height / 480"), "{s}");
    assert!(
        pos(&s, "_vcg_need") > pos(&s, "_vcg_t, _vcg_b = 0, 10"),
        "it reads clip.height, so the rows must already be gone"
    );
    assert!(
        pos(&s, "_vcg_need") < pos(&s, "PAR correction"),
        "and it must precede the resize it is correcting for"
    );
}

/// Without a PAR resize nothing is mapped onto a fixed target, so nothing is
/// distorted and nothing should be trimmed.
#[test]
fn no_par_correction_means_no_compensation() {
    let config = Config {
        crop_preset: Some(CropPreset::Manual),
        crop_bottom: 10,
        par_correction: false,
        ..Config::default()
    };
    assert!(!script(&config).contains("_vcg_need"));
}

/// The overscan preset sets SAR for its own exact geometry instead of
/// resizing, so it is already self-consistent.
#[test]
fn the_overscan_preset_compensates_for_itself() {
    let config = Config {
        crop_preset: Some(CropPreset::Overscan),
        ..Config::default()
    };
    let s = script(&config);
    assert!(s.contains("_SARNum=79"), "the premise of this test");
    assert!(!s.contains("_vcg_need"));
}

#[test]
fn an_hd_source_is_never_compensated() {
    let config = Config {
        crop_preset: Some(CropPreset::Manual),
        crop_bottom: 10,
        source: hd(SourceClass::Avchd, false),
        ..Config::default()
    };
    assert!(!script(&config).contains("_vcg_need"));
}

// ── Trim ────────────────────────────────────────────────────────────────────

/// A kept range that covers the whole file must not emit a trim at all.
///
/// The UI collapses a full-length selection to an empty range for this
/// reason; this pins the other half, that an empty range is the thing the
/// generator treats as "keep everything".
#[test]
fn no_ranges_means_no_trim() {
    let config = Config::default();
    assert!(config.trim_ranges.is_empty(), "the premise");
    assert!(!script(&config).contains("_vcg_trim"));
}

/// Trimming happens on source frames, before anything else, so cut material
/// is never deinterlaced — which is also why trimming makes an export faster.
#[test]
fn a_kept_range_is_cut_before_everything_else() {
    let config = Config {
        trim_ranges: vec![(120, 900)],
        ..Config::default()
    };
    let s = script(&config);
    assert!(s.contains("_vcg_trim(_vcg_src, 120, 900)"), "{s}");
    assert!(pos(&s, "_vcg_trim") < pos(&s, "haf.QTGMC("));
    // And before the crop, so the crop works on fewer frames too.
    assert!(pos(&s, "_vcg_trim") < pos(&s, "core.std.Crop"));
}

/// The frame numbers are clamped inside the script, because a tape capture's
/// declared duration is routinely a little wrong and a frame past the end
/// would stop the render rather than produce a slightly short file.
#[test]
fn the_script_clamps_the_frame_numbers_itself() {
    let config = Config {
        trim_ranges: vec![(0, 999_999)],
        ..Config::default()
    };
    let s = script(&config);
    assert!(s.contains("b = min(b, c.num_frames - 1)"), "{s}");
    assert!(s.contains("a = max(0, min(a, b))"), "{s}");
}

// ── Levels ──────────────────────────────────────────────────────────────────

/// The two options have to be two things.
///
/// The Python emitted the same call for both — `gamma=1.0` is `std.Levels`'
/// default, so clamp and stretch differed only by a comment — and the page
/// offering them showed two identical histograms, which is how it was caught.
#[test]
fn clamp_and_stretch_are_not_the_same_filter() {
    let clamp = script(&Config {
        levels_adjustment: LevelsAdjustment::Clamp,
        ..Config::default()
    });
    let stretch = script(&Config {
        levels_adjustment: LevelsAdjustment::Stretch,
        levels_black: 40,
        levels_white: 200,
        ..Config::default()
    });
    let line = |s: &str| {
        s.lines()
            .find(|l| l.contains("core.std.Levels"))
            .unwrap_or("<none>")
            .to_string()
    };
    assert_ne!(line(&clamp), line(&stretch), "two options, two filters");
}

/// Clamping must leave legal pictures alone.
///
/// The Python mapped the whole 0-255 range onto 16-235, which is a full-to-
/// limited range conversion, not a clamp. Measured against VapourSynth, a
/// picture already inside the range came out with its black at 29.7 and its
/// white at 216 — it darkened footage that needed nothing done to it. The
/// input window must be the legal window, so the mapping is the identity
/// inside it and a cut outside.
#[test]
fn clamping_maps_the_legal_window_onto_itself() {
    let s = script(&Config {
        levels_adjustment: LevelsAdjustment::Clamp,
        ..Config::default()
    });
    assert!(
        s.contains("core.std.Levels(clip, min_in=4096, max_in=60160, min_out=4096, max_out=60160"),
        "{s}"
    );
    assert!(
        !s.contains("min_in=0, max_in=65535"),
        "the whole code range is not the clamp's input window:\n{s}"
    );
}

/// Stretching has to stretch *this picture*, which means its measured black
/// and white points are the input window. Against a fixed 0-255 input there
/// is nothing being stretched.
#[test]
fn stretching_expands_the_measured_range_to_the_legal_one() {
    let s = script(&Config {
        levels_adjustment: LevelsAdjustment::Stretch,
        levels_black: 40,
        levels_white: 200,
        ..Config::default()
    });
    // 40x256 = 10240, 200x256 = 51200.
    assert!(
        s.contains("core.std.Levels(clip, min_in=10240, max_in=51200, min_out=4096, max_out=60160"),
        "{s}"
    );
}

/// With nothing measured the defaults are the legal limits, so a stretch
/// expands 16-235 onto 16-235 and changes nothing. A setting that alters
/// brightness must do nothing rather than guess.
#[test]
fn stretching_without_a_measurement_changes_nothing() {
    let s = script(&Config {
        levels_adjustment: LevelsAdjustment::Stretch,
        ..Config::default()
    });
    assert!(
        s.contains("min_in=4096, max_in=60160, min_out=4096, max_out=60160"),
        "{s}"
    );
}

/// Measured nonsense — a white point at or below the black point — must not
/// produce a filter at all. Dividing by that range would invert the picture.
#[test]
fn an_impossible_measurement_emits_no_filter() {
    let s = script(&Config {
        levels_adjustment: LevelsAdjustment::Stretch,
        levels_black: 200,
        levels_white: 40,
        ..Config::default()
    });
    assert!(!s.contains("core.std.Levels"), "{s}");
}

// ── Frame rate ──────────────────────────────────────────────────────────────

/// The fps choice has to reach both legs of the export and agree with itself.
///
/// FPSDivisor is set in the VapourSynth script; `-r` is set on the ffmpeg
/// command that muxes what the script produces. If the two disagree the file
/// plays at the wrong speed and the sound drifts out of sync against it —
/// and neither half is obviously wrong on its own, which is why this is
/// pinned rather than left to reading.
#[test]
fn the_frame_rate_choice_reaches_the_script_and_the_muxer() {
    use vcg_core::ffmpeg::output_frame_rate;

    for (pal, double, divisor, rate) in [
        (false, true, "FPSDivisor=1", "60000/1001"),
        (false, false, "FPSDivisor=2", "30000/1001"),
        (true, true, "FPSDivisor=1", "50"),
        (true, false, "FPSDivisor=2", "25"),
    ] {
        let config = Config {
            format: if pal {
                VideoFormat::Pal
            } else {
                VideoFormat::Ntsc
            },
            double_rate: double,
            ..Config::default()
        };
        let s = script(&config);
        assert!(
            s.contains(divisor),
            "pal={pal} double={double}: {divisor} missing from the script"
        );
        assert_eq!(
            output_frame_rate(&config),
            rate,
            "pal={pal} double={double}: the muxer disagrees with the script"
        );
    }
}

/// Inverse telecine restores the film rate whatever the fps choice says —
/// there are no fields left to double once the pulldown is removed.
#[test]
fn inverse_telecine_outranks_the_frame_rate_choice() {
    use vcg_core::ffmpeg::output_frame_rate;

    let config = Config {
        ivtc_mode: true,
        double_rate: true,
        ..Config::default()
    };
    assert_eq!(output_frame_rate(&config), "24000/1001");
}

/// A plugin that will not load must not fail silently.
///
/// Every LoadPlugin failure used to be swallowed. Most are genuinely benign —
/// a plugin the host autoloaded already, and the support libraries in the same
/// folder that are not plugins at all — but one is fatal and was being
/// swallowed with the rest: VapourSynth R80 removed API 3, and almost every
/// bundled plugin uses it. Pointed at an R80+ core the payload loses mvtools,
/// nnedi3 and seventeen others, says nothing, and dies later inside havsfunc
/// with "no attribute or namespace named mv" — which reads as a missing
/// install rather than as a core too new to load what is already on disk.
/// Reported exactly that way, after a pip command upgraded the core.
#[test]
fn an_api3_core_mismatch_is_raised_rather_than_swallowed() {
    // Needs a real plugin directory, or the whole block is omitted.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-plugins/plugins64");
    std::fs::create_dir_all(&dir).unwrap();
    let deps = DepsPaths::new(dir.parent().unwrap());

    let s = generate_vpy(&Config::default(), &deps);
    assert!(s.contains("_vcg_api3"), "{s}");
    assert!(s.contains("\"API 3\" in str(_vcg_e)"), "{s}");
    assert!(s.contains("R80 removed"), "{s}");
    assert!(s.contains("vapoursynth==79"), "{s}");
    // And the failure must be raised, not collected and ignored.
    assert!(s.contains("if _vcg_api3:"), "{s}");
    assert!(
        !s.contains("        except Exception:
            pass"),
        "the blanket swallow is back: {s}"
    );
}

// ── The two QTGMCs ──────────────────────────────────────────────────────────

/// The havsfunc path is the default. The vs-jetpack option was added by
/// branching around this code, which is exactly the shape of edit that
/// silently rewrites the branch it was meant to leave alone.
#[test]
fn legacy_qtgmc_is_the_default_and_still_emits_havsfunc() {
    use vcg_core::config::QtgmcImpl;

    let config = Config::default();
    assert_eq!(config.qtgmc_impl, QtgmcImpl::Legacy);

    let s = script(&config);
    assert!(s.contains("import havsfunc as haf"), "{s}");
    assert!(s.contains("clip = haf.QTGMC("), "{s}");
    assert!(s.contains("    FPSDivisor=1,"), "{s}");
    assert!(!s.contains("QTempGaussMC"), "{s}");
}

/// The modern path replaces the import as well as the call.
///
/// Leaving `import havsfunc` in place would work — it is bundled — but it
/// would also mean a script that names a library it never calls, and the
/// failure for anyone without havsfunc would land on the import rather than
/// on the setting that asked for the other one.
#[test]
fn modern_qtgmc_swaps_the_import_and_the_call() {
    use vcg_core::config::QtgmcImpl;

    let s = script(&Config {
        qtgmc_impl: QtgmcImpl::Modern,
        ..Config::default()
    });
    assert!(!s.contains("import havsfunc"), "{s}");
    assert!(!s.contains("haf.QTGMC"), "{s}");
    assert!(
        s.contains("from vsdeinterlace import QTempGaussMC as _vcg_qtgmc"),
        "{s}"
    );
    assert!(s.contains(".deinterlace(clip, tff=True)"), "{s}");
    // The two plugins the bundled payload does not carry, named in the
    // script so the failure is one sentence rather than a traceback.
    assert!(s.contains("\"resize2\", \"zsmooth\""), "{s}");
    // Scene-change thresholds are the rewrite's own now. The old pair was
    // looser than any default and risked carrying motion across cuts.
    assert!(!s.contains("thscd="), "{s}");
}

/// Both backends must spell out the same least-destructive profile.
///
/// The rewrite's defaults are not the Slower preset's: left alone it bobs
/// with 256 neurons at quality 2 and adds a Recalculate pass, none of which
/// this profile wants. And two of the mappings are swapped by number
/// between the libraries, so this pins the *meaning* of each one.
#[test]
fn modern_qtgmc_uses_the_least_destructive_profile() {
    use vcg_core::config::QtgmcImpl;

    let s = script(&Config {
        qtgmc_impl: QtgmcImpl::Modern,
        ..Config::default()
    });

    // Preset="Slower", spelled out: havsfunc uses NNSize=1, NNeurons=1,
    // EdiQual=1, and RefineMotion=False.
    assert!(
        s.contains(".basic(bobber=_vcg_nnedi3(nsize=1, nns=1, qual=1))"),
        "{s}"
    );
    assert!(s.contains(".analyze(refine=0)"), "{s}");
    assert!(s.contains("from vsaa import NNEDI3 as _vcg_nnedi3"), "{s}");

    // NoiseProcess=0. IDENTIFY with both restores at zero makes the denoise
    // stage return before it builds anything — and the rewrite's IDENTIFY is
    // 0 where havsfunc's "identify only" is 2, so this must never be mapped
    // by number.
    assert!(
        s.contains(".denoise(mode=_vcg_qtgmc.NoiseProcessMode.IDENTIFY)"),
        "{s}"
    );
    assert!(!s.contains("NoiseProcessMode.DENOISE"), "{s}");
    assert!(!s.contains("noise_restore"), "{s}");

    // Lossless=2 is "before resharpening" — PRESHARPEN, whose value is 1.
    // POSTSMOOTH is the identically numbered trap.
    assert!(
        s.contains(".lossless(mode=_vcg_qtgmc.LosslessMode.PRESHARPEN)"),
        "{s}"
    );
    assert!(!s.contains("LosslessMode.POSTSMOOTH"), "{s}");

    // TR2=1, SourceMatch=3, Sharpness=0.1, Sbb=0.
    assert!(s.contains(".final(tr=1)"), "{s}");
    assert!(
        s.contains(".source_match(mode=_vcg_qtgmc.SourceMatchMode.TWICE_REFINED)"),
        "{s}"
    );
    assert!(s.contains(".sharpen(strength=0.1)"), "{s}");
    assert!(s.contains(".back_blend(sigma=0)"), "{s}");
}

/// Neither backend may denoise, whichever one is selected.
///
/// Denoising is the Noise step's job, on progressive frames, where the user
/// can see it and switch it off. QTGMC doing some of it anyway meant it
/// happened whether or not anybody asked.
#[test]
fn neither_backend_denoises_inside_qtgmc() {
    use vcg_core::config::QtgmcImpl;

    for impl_ in [QtgmcImpl::Legacy, QtgmcImpl::Modern] {
        let s = script(&Config {
            qtgmc_impl: impl_,
            ..Config::default()
        });
        for banned in [
            "NoiseRestore",
            "GrainRestore",
            "NoisePreset",
            "NoiseTR",
            "NoiseDeint",
            "StabilizeNoise",
            "ThSCD1",
            "ThSCD2",
            "Rep0",
            "Rep1",
            "Rep2",
            "DCT=",
            "TR0",
            "TR1",
        ] {
            assert!(
                !s.contains(banned),
                "{impl_:?} still emits {banned}:
{s}"
            );
        }
    }
}

/// Single-rate must decimate, not motion-blur.
///
/// `QTempGaussMC` decides whether to blur by comparing the two shutter angles,
/// and at its default pair of (180, 180) an fps_divisor of 2 reads as a
/// request for a 50% flow blur. havsfunc's FPSDivisor only ever dropped
/// frames, so the source angle is declared as 360 to land on the branch that
/// blurs nothing. Without this the 29.97 fps option would quietly smear.
#[test]
fn modern_qtgmc_single_rate_does_not_turn_on_motion_blur() {
    use vcg_core::config::QtgmcImpl;

    let double = script(&Config {
        qtgmc_impl: QtgmcImpl::Modern,
        double_rate: true,
        ..Config::default()
    });
    assert!(double.contains(".motion_blur(fps_divisor=1)"), "{double}");

    let single = script(&Config {
        qtgmc_impl: QtgmcImpl::Modern,
        double_rate: false,
        ..Config::default()
    });
    assert!(
        single.contains(".motion_blur(shutter_angle=(360, 180), fps_divisor=2)"),
        "{single}"
    );
}

/// Field order reaches the rewrite under its own spelling.
///
/// Feeding QTGMC the wrong dominance is the single worst thing that can happen
/// to a deinterlace — see the field-order entry in HANDOVER.md — and the
/// modern call passes it as a different keyword in a different position, so it
/// is its own opportunity to get it backwards.
#[test]
fn modern_qtgmc_carries_bottom_field_first() {
    use vcg_core::config::QtgmcImpl;

    let s = script(&Config {
        qtgmc_impl: QtgmcImpl::Modern,
        field_order: FieldOrder::Bff,
        ..Config::default()
    });
    assert!(s.contains(".deinterlace(clip, tff=False)"), "{s}");
}

/// Every setting the script passes must have a description on the About
/// page, and vice versa.
///
/// The Python's About text was a hand-written block beside the settings dict
/// and the two drifted: by the end it documented `NoiseProcess=2` on a build
/// that had been shipping something else for months. The page reads the
/// table now, so the only way they can disagree is a missing description.
#[test]
fn the_profile_and_its_descriptions_line_up() {
    let profile = vcg_core::vpy::qtgmc_profile();
    assert_eq!(profile.len(), 8, "the least-destructive profile is 8 settings");
    for (key, value, why) in &profile {
        assert!(!why.is_empty(), "{key} has no description for the About page");
        assert!(!value.is_empty(), "{key} has no value");
        assert!(
            !value.contains('"'),
            "{key}'s value still carries its Python quotes: {value}"
        );
    }
    // The two that must never silently come back.
    let noise = profile.iter().find(|(k, ..)| *k == "NoiseProcess");
    assert_eq!(noise.map(|(_, v, _)| *v), Some("0"), "QTGMC must not denoise");
    let sharp = profile.iter().find(|(k, ..)| *k == "Sharpness");
    assert_eq!(sharp.map(|(_, v, _)| *v), Some("0.1"));
}

/// The preview must normalise the pixel format exactly as the export does.
///
/// It used to handle RGB only. DV is 4:1:1, `normalize_pixel_format` lets it
/// through because VFM accepts 411, and `mv.Super` then refuses the clip:
/// "input clip must be GRAY, 420, 422, 440, or 444". Every MiniDV capture
/// showed that traceback where the preview should have been, while the
/// export of the same file worked — the two paths disagreed about what a
/// clip is.
#[test]
fn the_preview_converts_the_same_formats_the_export_does() {
    let config = Config::default();
    let preview = vcg_core::vpy::generate_preview_vpy(&config, &deps());
    let export = script(&config);

    for branch in [
        "clip.format.subsampling_w == 2 and clip.format.subsampling_h == 0",
        "elif clip.format.color_family == vs.RGB:",
        "format=vs.YUV422P8",
    ] {
        assert!(
            export.contains(branch),
            "the export lost a conversion branch: {branch}"
        );
        assert!(
            preview.contains(branch),
            "the preview is missing {branch}, so a DV source reaches mv.Super as 4:1:1:
{preview}"
        );
    }
}

// ── Chroma denoising ────────────────────────────────────────────────────────

/// A config with denoising on at the given strength.
fn denoised(level: vcg_core::config::NoiseLevel) -> String {
    script(&Config { noise_level: level, ..Default::default() })
}

#[test]
fn chroma_is_denoised_spatially_while_luma_gets_a_temporal_radius() {
    let s = denoised(vcg_core::config::NoiseLevel::Moderate);

    // Luma: sigma 3.0 with a temporal radius.
    assert!(
        s.contains("sigma=[3.0, 0, 0], radius=1"),
        "luma pass missing its temporal radius:\n{s}"
    );
    // Chroma: half again the sigma, and radius=0 — temporal block matching on
    // soft noisy chroma is what smears colour.
    assert!(
        s.contains("sigma=[0, 4.5, 4.5], radius=0"),
        "chroma pass missing or not spatial-only:\n{s}"
    );
    // The two passes are recombined rather than one overwriting the other.
    assert!(
        s.contains("ShufflePlanes([_vcg_y, _vcg_c, _vcg_c]"),
        "the luma and chroma passes are never merged:\n{s}"
    );
}

#[test]
fn heavy_denoising_scales_both_planes() {
    let s = denoised(vcg_core::config::NoiseLevel::Heavy);
    assert!(s.contains("sigma=[6.0, 0, 0], radius=2"), "{s}");
    assert!(s.contains("sigma=[0, 9.0, 9.0], radius=0"), "{s}");
}

#[test]
fn the_knlmeanscl_fallback_never_uses_a_channels_mode_that_breaks_on_420() {
    // Measured against the bundled plugin: on a 4:2:0 clip channels="auto"
    // silently resolves to "Y" and never touches chroma, and channels="YUV"
    // is a hard crash. A separate "UV" pass is the only working form.
    let s = denoised(vcg_core::config::NoiseLevel::Moderate);
    assert!(!s.contains(r#"channels="auto""#), "auto is a no-op on 4:2:0:\n{s}");
    assert!(!s.contains(r#"channels="YUV""#), "YUV crashes on 4:2:0:\n{s}");
    assert!(s.contains(r#"channels="Y""#), "{s}");
    assert!(s.contains(r#"channels="UV""#), "{s}");
    // The chroma pass is spatial-only here too.
    assert!(s.contains(r#"d=0, a=2, s=4, h=1.8, channels="UV""#), "{s}");
}

#[test]
fn the_bundled_bm3d_fallback_stays_luma_only() {
    // The original VapourSynth-BM3D refuses a subsampled clip outright when
    // chroma is processed ("convert it to YUV444 or RGB first"), and it raises
    // vs.Error, which the surrounding `except AttributeError` would not catch.
    let s = denoised(vcg_core::config::NoiseLevel::Moderate);
    for call in ["core.bm3d.Basic(", "core.bm3d.Final("] {
        let line = s
            .lines()
            .find(|l| l.contains(call))
            .unwrap_or_else(|| panic!("{call} not found in:\n{s}"));
        assert!(
            line.contains("sigma=[3.0, 0, 0]"),
            "{call} must stay luma-only, got: {line}"
        );
    }
}

#[test]
fn denoising_off_emits_no_denoise_at_all() {
    let s = denoised(vcg_core::config::NoiseLevel::None);
    assert!(!s.contains("BM3Dv2"), "{s}");
    assert!(!s.contains("KNLMeansCL"), "{s}");
}
