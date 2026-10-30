//! The guarantee behind the three modes.
//!
//! "Just deinterlace" is a promise to the user: hand this output to DaVinci
//! Resolve or Topaz and nothing aesthetic has been decided for you. A promise
//! like that is worth only as much as the test that enforces it, so the central
//! case here feeds the worst report the probes can produce — filthy tape, heavy
//! halos, strong color cast, illegal levels — and asserts that not one
//! optional filter gets set.

use vcg_core::analysis::classify::{
    BleedLevel, BleedingAnalysis, ColorCast, ColorData, Levels, NoiseAnalysis, NoiseTrigger,
    NoiseVerdict, SaturationIssue,
};
use vcg_core::analysis::audio::{AudioAnalysis, AudioVerdict, ChannelLevel};
use vcg_core::analysis::edges::{Detection, EdgeCrop};
use vcg_core::analysis::halo::{HaloAnalysis, HaloLevel};
use vcg_core::analysis::idet::{Confidence, Pattern, Telecine};
use vcg_core::analysis::probe::{
    Classification, DetectedCaptureMethod, DetectedFieldOrder, ParFormat, VideoInfo,
};
use vcg_core::analysis::Recommendation;
use vcg_core::config::{
    CaptureMethod, ColorCorrection, Config, CropPreset, DehaloMode, FieldOrder, LevelsAdjustment,
    Mode, NoiseLevel, SourceClass, VideoFormat,
};
use vcg_tools::apply::{apply_report, options_for};
use vcg_tools::probe::Report;

/// A report from a thoroughly damaged tape: every probe reporting its worst.
fn awful_tape() -> Report {
    Report {
        info: Some(VideoInfo {
            width: 720,
            height: 480,
            fps: 29.97,
            duration: 1800.0,
        }),
        classification: Some(Classification {
            source_class: SourceClass::Sd,
            codec: "rawvideo".into(),
            width: 720,
            height: 480,
            fps: 29.97,
            field_order: DetectedFieldOrder::Unknown,
            pix_fmt: "yuyv422".into(),
            ..Default::default()
        }),
        par: Some(ParFormat {
            format: Some(VideoFormat::Ntsc),
            capture_method: Some(DetectedCaptureMethod::Sd),
            sar: "10:11".into(),
        }),
        levels: Some(Levels {
            min_y: Some(2),
            max_y: Some(252),
            needs_adjustment: true,
        }),
        color: Some(ColorData {
            u_avg: 150.0,
            v_avg: 110.0,
            sat_avg: 95.0,
            u_offset: 22.0,
            v_offset: -18.0,
            u_std: 1.0,
            v_std: 1.0,
            color_cast: Some(ColorCast::Blue),
            sat_issue: Some(SaturationIssue::High),
            u_correction: -22.0,
            v_correction: 18.0,
        }),
        noise: Some(NoiseAnalysis {
            noise_level: NoiseVerdict::Heavy,
            recommendation: Recommendation::Heavy,
            trigger: NoiseTrigger::Tout,
            avg_diff: 42.0,
            std_diff: 3.0,
            avg_variance: 0.21,
            luma_level: NoiseVerdict::Heavy,
            avg_chroma_diff: 6.0,
            chroma_ratio: 6.0 / 42.0,
            chroma_level: NoiseVerdict::Clean,
            samples_analyzed: 400,
            analyzed: true,
        }),
        bleeding: Some(BleedingAnalysis {
            bleed_level: BleedLevel::Significant,
            recommendation: true,
            avg_h_spread: 140.0,
            avg_v_spread: 130.0,
            avg_saturation: 95.0,
            bleeding_score: 256.0,
            samples_analyzed: 10,
        }),
        halo: Some(HaloAnalysis {
            analyzed: true,
            halo_ratio: 0.42,
            edges_analyzed: 5000,
            halo_level: HaloLevel::Strong,
            recommendation: Recommendation::Strong,
        }),
        telecine: Some(Telecine {
            pattern: None,
            confidence: None,
            progressive_source: false,
            windows: 5,
            film_windows: 0,
            prog_ratio: 0.0,
            rep_ratio: 0.0,
        tff_frames: 0,
        bff_frames: 0,
        }),
        // Lopsided blanking with a head-switching band: the shape no preset
        // can express.
        edges: Some(Detection::Found(EdgeCrop {
            left: 18,
            right: 10,
            top: 0,
            bottom: 8,
            noise_rows: 8,
        })),
        // Sound on the left only: the fault worth catching.
        audio: Some(AudioAnalysis {
            channels: 2,
            levels: vec![
                ChannelLevel { rms_db: Some(-24.0), peak_db: Some(-8.0) },
                ChannelLevel { rms_db: None, peak_db: None },
            ],
            verdict: AudioVerdict::SilentSide { live: 0 },
        }),
        histogram: None,
        spawns: 45,
        warnings: Vec::new(),
    }
}

// ── The promise ─────────────────────────────────────────────────────────────

#[test]
fn just_deinterlace_applies_no_filter_however_bad_the_tape() {
    let mut config = Config::default();
    apply_report(&mut config, &awful_tape(), Mode::JustDeinterlace);

    // Every optional filter must still be at its off value.
    assert_eq!(config.noise_level, NoiseLevel::None, "noise was applied");
    assert_eq!(config.dehalo_mode, DehaloMode::None, "dehalo was applied");
    assert_eq!(
        config.color_correction,
        ColorCorrection::None,
        "color correction was applied"
    );
    assert_eq!(config.u_correction, 0.0);
    assert_eq!(config.v_correction, 0.0);
    assert_eq!(
        config.levels_adjustment,
        LevelsAdjustment::None,
        "levels were clamped — that discards picture data a grader may want"
    );
    assert_eq!(config.yc_delay, 0);
    assert!(!config.upscale_enabled);
    assert_eq!(config.grain_strength, 0.0);
    assert!(!config.dropout_removal);
    assert!(!config.chroma_shift);

    // And the summary predicate agrees.
    assert!(
        config.is_structural_only(),
        "is_structural_only disagrees with the field-by-field check"
    );
}

#[test]
fn just_deinterlace_still_does_the_structural_work() {
    // The promise is "nothing aesthetic", not "nothing at all". An output with
    // the wrong field order or non-square pixels is broken, not unenhanced.
    let mut config = Config::default();
    apply_report(&mut config, &awful_tape(), Mode::JustDeinterlace);

    assert_eq!(config.format, VideoFormat::Ntsc);
    assert_eq!(config.detected_sar, "10:11");
    assert_eq!(config.capture_method, CaptureMethod::Sd);
    assert_eq!(config.source.source_class, SourceClass::Sd);
    assert_eq!(config.source.pix_fmt, "yuyv422");
    // Interlaced video, no film pattern: deinterlace rather than IVTC.
    assert!(!config.ivtc_mode);
    assert_ne!(config.field_order, FieldOrder::Progressive);
    // Crop and PAR correction are on, because blanking and non-square pixels
    // are capture-chain faults rather than taste. The crop is the *measured*
    // one: the preset's symmetric 8 columns would leave blanking on the left
    // of this report and eat picture on the right.
    assert_eq!(config.effective_crop_preset(), CropPreset::Manual);
    assert_eq!(
        (config.crop_left, config.crop_right, config.crop_top, config.crop_bottom),
        (20, 12, 0, 10),
        "one bottom number, with the safety margin on every edge that found something"
    );
    // The whole of this report's bottom finding is switching noise, and it is
    // inside crop_bottom rather than alongside it.
    assert_eq!(config.head_switching_rows, 10);
    assert!(config.par_correction);
    assert!(config.dither_enabled);
}

// ── The measured crop ───────────────────────────────────────────────────────

#[test]
fn the_measured_crop_is_applied_in_every_mode() {
    // Blanking is not an enhancement, so no mode may decline it. A lopsided
    // capture is the case no preset can express at all.
    for mode in [
        Mode::JustDeinterlace,
        Mode::EnhanceAutomatically,
        Mode::EnhanceManually,
    ] {
        let mut config = Config::default();
        apply_report(&mut config, &awful_tape(), mode);
        assert_eq!(
            (
                config.crop_left,
                config.crop_right,
                config.crop_top,
                config.crop_bottom
            ),
            (20, 12, 0, 10),
            "{mode:?} did not apply the measured crop"
        );
    }
}

#[test]
fn a_measured_crop_of_nothing_beats_the_preset() {
    // A frame that is already all picture must not be cropped at all. The
    // BT.601 preset would remove 8 real columns from each side.
    let mut report = awful_tape();
    report.edges = Some(Detection::Found(EdgeCrop::default()));
    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::JustDeinterlace);

    assert_eq!(config.effective_crop_preset(), CropPreset::Manual);
    assert_eq!(
        (config.crop_left, config.crop_right, config.crop_top, config.crop_bottom),
        (0, 0, 0, 0),
        "a frame with no blanking was cropped anyway"
    );
}

#[test]
fn bottom_blanking_and_the_band_are_stored_apart() {
    // A tape with both: 14 rows found at the bottom, 6 of them snow. Storing
    // the total would make the band impossible to decline on its own.
    let mut report = awful_tape();
    report.edges = Some(Detection::Found(EdgeCrop {
        left: 0,
        right: 0,
        top: 0,
        bottom: 14,
        noise_rows: 6,
    }));
    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::JustDeinterlace);

    // 14 found at the bottom plus the margin, of which 6+2 are the band.
    assert_eq!(config.crop_bottom, 16, "one bottom number");
    assert_eq!(config.head_switching_rows, 8, "how much of it is the band");
    assert!(
        config.head_switching_rows <= config.crop_bottom,
        "the band has to fit inside the crop it describes"
    );
}

#[test]
fn no_band_detected_means_nothing_to_decline() {
    let mut report = awful_tape();
    report.edges = Some(Detection::Found(EdgeCrop {
        left: 8,
        right: 8,
        top: 0,
        bottom: 4,
        noise_rows: 0,
    }));
    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::JustDeinterlace);

    assert_eq!(config.crop_bottom, 6, "4 found plus the margin");
    assert_eq!(config.head_switching_rows, 0, "no band was found");
}

#[test]
fn an_inconclusive_measurement_leaves_the_preset_alone() {
    // Too dark to judge, or the decode failed. Falling back to the preset is
    // the safe answer; cropping to a guess is not.
    let mut report = awful_tape();
    report.edges = Some(Detection::Inconclusive("too dark"));
    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::JustDeinterlace);

    assert_eq!(config.effective_crop_preset(), CropPreset::Bt601);
}

#[test]
fn the_measured_crop_survives_switching_down_to_just_deinterlace() {
    // `clear_enhancements` must not treat it as taste.
    let mut config = Config::default();
    apply_report(&mut config, &awful_tape(), Mode::EnhanceManually);
    apply_report(&mut config, &awful_tape(), Mode::JustDeinterlace);
    assert_eq!(
        (config.crop_left, config.crop_right),
        (20, 12),
        "the measured crop was cleared as if it were an enhancement"
    );
    assert_eq!(
        config.crop_bottom, 10,
        "the bottom crop was cleared as if it were an enhancement"
    );
    assert!(config.is_structural_only());
}

#[test]
fn switching_down_to_just_deinterlace_clears_what_was_set() {
    // The user explores in manual mode, sets filters, then changes their mind.
    // A stale filter surviving the switch would silently break the promise.
    let mut config = Config {
        noise_level: NoiseLevel::Heavy,
        dehalo_mode: DehaloMode::Strong,
        levels_adjustment: LevelsAdjustment::Clamp,
        color_correction: ColorCorrection::AutoFixBoost,
        u_correction: -9.0,
        yc_delay: 4,
        grain_strength: 2.5,
        upscale_enabled: true,
        upscale_resolution: Some("1440x1080".into()),
        dropout_removal: true,
        chroma_shift: true,
        ..Config::default()
    };
    assert!(!config.is_structural_only(), "the premise of this test");

    apply_report(&mut config, &awful_tape(), Mode::JustDeinterlace);
    assert!(config.is_structural_only(), "a filter survived the mode switch");
    assert_eq!(config.upscale_resolution, None);
}

// ── Automatic mode ──────────────────────────────────────────────────────────

#[test]
fn automatic_mode_applies_what_was_measured() {
    let mut config = Config::default();
    apply_report(&mut config, &awful_tape(), Mode::EnhanceAutomatically);

    assert_eq!(config.noise_level, NoiseLevel::Heavy);
    assert_eq!(config.dehalo_mode, DehaloMode::Strong);
    assert_eq!(config.color_correction, ColorCorrection::AutoFix);
    assert_eq!(config.u_correction, -22.0);
    assert_eq!(config.v_correction, 18.0);
    assert!(!config.is_structural_only());
}

/// Levels are the exception: automatic mode measures them and says so, but
/// does not clamp.
///
/// Clamping is the only correction here that destroys data — the
/// super-blacks and super-whites on a tape capture are usually real picture
/// the deck recorded, and once clipped they cannot come back. The old
/// software recommended leaving them alone for exactly this reason; this
/// port used to clamp them automatically, which is worse than the thing it
/// was ported from.
#[test]
fn automatic_mode_never_clamps_levels_by_itself() {
    let report = awful_tape();
    assert!(
        report.levels.as_ref().is_some_and(|l| l.needs_adjustment),
        "the premise: this tape is outside the legal range"
    );

    for mode in Mode::ALL {
        let mut config = Config::default();
        apply_report(&mut config, &report, mode);
        assert_eq!(
            config.levels_adjustment,
            LevelsAdjustment::None,
            "{mode:?} clamped the levels without being asked"
        );
    }
}

#[test]
fn automatic_mode_adds_nothing_to_a_clean_tape() {
    // A clean source must come out of automatic mode looking like it came out
    // of just-deinterlace mode. "Automatic" means "as measured", not "always
    // do something".
    let mut report = awful_tape();
    report.noise = Some(NoiseAnalysis {
        noise_level: NoiseVerdict::Clean,
        recommendation: Recommendation::None,
        trigger: NoiseTrigger::Tout,
        avg_diff: 2.0,
        std_diff: 0.5,
        avg_variance: 0.001,
        luma_level: NoiseVerdict::Clean,
        avg_chroma_diff: 0.3,
        chroma_ratio: 0.15,
        chroma_level: NoiseVerdict::Clean,
        samples_analyzed: 400,
        analyzed: true,
    });
    report.halo = Some(HaloAnalysis {
        analyzed: true,
        halo_ratio: 0.04,
        edges_analyzed: 5000,
        halo_level: HaloLevel::Clean,
        recommendation: Recommendation::None,
    });
    report.color = Some(ColorData {
        color_cast: None,
        sat_issue: None,
        u_offset: 0.4,
        v_offset: -0.3,
        u_correction: -0.4,
        v_correction: 0.3,
        ..awful_tape().color.unwrap()
    });
    report.levels = Some(Levels {
        min_y: Some(18),
        max_y: Some(230),
        needs_adjustment: false,
    });

    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::EnhanceAutomatically);
    assert!(
        config.is_structural_only(),
        "a clean tape should pick up no filters"
    );
}

#[test]
fn an_unreliable_measurement_is_not_acted_on() {
    // `analyzed: false` means too few frames were measured to trust. Acting on
    // it would apply heavy denoising on the strength of two samples.
    let mut report = awful_tape();
    report.noise = Some(NoiseAnalysis {
        analyzed: false,
        samples_analyzed: 2,
        ..awful_tape().noise.unwrap()
    });
    report.halo = Some(HaloAnalysis {
        analyzed: false,
        ..awful_tape().halo.unwrap()
    });

    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::EnhanceAutomatically);
    assert_eq!(config.noise_level, NoiseLevel::None);
    assert_eq!(config.dehalo_mode, DehaloMode::None);
}

#[test]
fn color_bleeding_is_never_applied_automatically() {
    // The correction is a chroma shift in whole pixels and the right amount is
    // a visual judgement, so a high score must not set it unattended.
    let mut config = Config::default();
    apply_report(&mut config, &awful_tape(), Mode::EnhanceAutomatically);
    assert_eq!(config.yc_delay, 0);
    assert!(!config.chroma_shift);
}

// ── Manual mode ─────────────────────────────────────────────────────────────

#[test]
fn manual_mode_applies_nothing_by_itself() {
    // Manual means the pages start from the recommendation, not that the
    // recommendation is already in force.
    let mut config = Config::default();
    apply_report(&mut config, &awful_tape(), Mode::EnhanceManually);

    assert_eq!(config.noise_level, NoiseLevel::None);
    assert_eq!(config.dehalo_mode, DehaloMode::None);
    assert_eq!(config.levels_adjustment, LevelsAdjustment::None);
    // But the structural work is still done.
    assert_eq!(config.format, VideoFormat::Ntsc);
    assert_eq!(config.detected_sar, "10:11");
}

#[test]
fn manual_mode_does_not_clear_what_the_user_set() {
    // Unlike just-deinterlace, manual must leave the user's own choices alone
    // when the report is re-applied after a re-analysis.
    let mut config = Config {
        noise_level: NoiseLevel::Moderate,
        grain_strength: 1.5,
        ..Config::default()
    };
    apply_report(&mut config, &awful_tape(), Mode::EnhanceManually);
    assert_eq!(config.noise_level, NoiseLevel::Moderate);
    assert_eq!(config.grain_strength, 1.5);
}

// ── Scan type is structural in every mode ───────────────────────────────────

#[test]
fn film_sources_get_inverse_telecine_in_every_mode() {
    let mut report = awful_tape();
    report.telecine = Some(Telecine {
        pattern: Some(Pattern::ThreeTwo),
        confidence: Some(Confidence::High),
        progressive_source: false,
        windows: 5,
        film_windows: 5,
        prog_ratio: 0.40,
        rep_ratio: 0.40,
        tff_frames: 0,
        bff_frames: 0,
    });

    for mode in Mode::ALL {
        let mut config = Config::default();
        apply_report(&mut config, &report, mode);
        assert!(config.ivtc_mode, "{mode:?} did not set inverse telecine");
    }
}

#[test]
fn progressive_sources_skip_deinterlacing_in_every_mode() {
    let mut report = awful_tape();
    report.telecine = Some(Telecine {
        pattern: None,
        confidence: None,
        progressive_source: true,
        windows: 5,
        film_windows: 0,
        prog_ratio: 0.99,
        rep_ratio: 0.0,
        tff_frames: 0,
        bff_frames: 0,
    });

    for mode in Mode::ALL {
        let mut config = Config::default();
        apply_report(&mut config, &report, mode);
        assert_eq!(config.field_order, FieldOrder::Progressive, "{mode:?}");
        assert!(!config.ivtc_mode, "{mode:?}");
    }
}

/// A film report, for the override tests below.
fn film_report() -> Report {
    let mut report = awful_tape();
    report.telecine = Some(Telecine {
        pattern: Some(Pattern::ThreeTwo),
        confidence: Some(Confidence::High),
        progressive_source: false,
        windows: 5,
        film_windows: 5,
        prog_ratio: 0.40,
        rep_ratio: 0.40,
        tff_frames: 0,
        bff_frames: 0,
    });
    report
}

/// A report with no pulldown found, for the override tests below.
fn video_report() -> Report {
    let mut report = awful_tape();
    report.telecine = Some(Telecine {
        pattern: None,
        confidence: None,
        progressive_source: false,
        windows: 5,
        film_windows: 0,
        prog_ratio: 0.08,
        rep_ratio: 0.01,
        tff_frames: 1200,
        bff_frames: 0,
    });
    report
}

#[test]
fn a_hand_made_film_choice_survives_re_analysis() {
    // The Check page's film-versus-video choice writes `ivtc_mode` directly,
    // and `apply_report` runs again every time the user changes mode. Without
    // the override flag the user's answer was overwritten by the detection a
    // second after they gave it — the setting moved on its own, which is the
    // one thing a manual override must never do.
    //
    // Both directions matter: a film transfer the scan missed, and a video
    // source the scan flagged by mistake.
    let mut config = Config { ivtc_mode: true, ivtc_user_override: true, ..Config::default() };
    for mode in Mode::ALL {
        apply_report(&mut config, &video_report(), mode);
        assert!(config.ivtc_mode, "{mode:?} discarded a manual film choice");
    }

    let mut config = Config { ivtc_mode: false, ivtc_user_override: true, ..Config::default() };
    for mode in Mode::ALL {
        apply_report(&mut config, &film_report(), mode);
        assert!(!config.ivtc_mode, "{mode:?} discarded a manual video choice");
    }
}

#[test]
fn without_an_override_the_scan_still_decides() {
    // The guard must not become a way for a stale value to persist: with the
    // flag clear, the measurement is in charge in both directions.
    for mode in Mode::ALL {
        let mut config = Config { ivtc_mode: true, ..Config::default() };
        apply_report(&mut config, &video_report(), mode);
        assert!(!config.ivtc_mode, "{mode:?}");

        let mut config = Config::default();
        apply_report(&mut config, &film_report(), mode);
        assert!(config.ivtc_mode, "{mode:?}");
    }
}

#[test]
fn an_override_does_not_disturb_the_progressive_field_order() {
    // Field order is read off the picture and is structural; forcing inverse
    // telecine by hand says nothing about it, so the progressive verdict must
    // still land.
    let mut report = awful_tape();
    report.telecine = Some(Telecine {
        pattern: None,
        confidence: None,
        progressive_source: true,
        windows: 5,
        film_windows: 0,
        prog_ratio: 0.99,
        rep_ratio: 0.0,
        tff_frames: 0,
        bff_frames: 0,
    });
    let mut config = Config { ivtc_mode: true, ivtc_user_override: true, ..Config::default() };
    apply_report(&mut config, &report, Mode::EnhanceAutomatically);
    assert_eq!(config.field_order, FieldOrder::Progressive);
    assert!(config.ivtc_mode, "the manual choice still stands");
}

#[test]
fn a_measured_scan_type_outranks_the_container() {
    // Containers frequently declare TFF on film-sourced material. The picture
    // is the better evidence.
    let mut report = awful_tape();
    report.classification = Some(Classification {
        field_order: DetectedFieldOrder::Tff,
        ..awful_tape().classification.unwrap()
    });
    report.telecine = Some(Telecine {
        pattern: None,
        confidence: None,
        progressive_source: true,
        windows: 5,
        film_windows: 0,
        prog_ratio: 0.99,
        rep_ratio: 0.0,
        tff_frames: 0,
        bff_frames: 0,
    });

    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::JustDeinterlace);
    assert_eq!(config.field_order, FieldOrder::Progressive);
}

// ── Probe selection ─────────────────────────────────────────────────────────

#[test]
fn just_deinterlace_skips_the_probes_nothing_will_act_on() {
    let opts = options_for(Mode::JustDeinterlace);
    // Scan type is structural, so it still runs.
    assert!(opts.telecine);
    // These are most of the analysis time and have no effect in this mode.
    assert!(!opts.noise);
    assert!(!opts.halo);
    assert!(!opts.bleeding);
    assert!(!opts.levels_color);
    // Edge detection is the exception: cropping is on in this mode, so the
    // measurement it depends on has to run.
    assert!(opts.edges);
}

#[test]
fn the_enhancement_modes_run_everything() {
    for mode in [Mode::EnhanceAutomatically, Mode::EnhanceManually] {
        let opts = options_for(mode);
        assert!(opts.telecine, "{mode:?}");
        assert!(opts.noise, "{mode:?}");
        assert!(opts.halo, "{mode:?}");
        assert!(opts.bleeding, "{mode:?}");
        assert!(opts.levels_color, "{mode:?}");
        assert!(opts.edges, "{mode:?}");
    }
}

#[test]
fn mode_predicates_are_mutually_consistent() {
    // Exactly one mode applies recommendations, and exactly one shows the
    // filter pages; the probe-skipping mode is the one that does neither.
    assert_eq!(
        Mode::ALL
            .iter()
            .filter(|m| m.applies_recommendations())
            .count(),
        1
    );
    assert_eq!(
        Mode::ALL.iter().filter(|m| m.shows_filter_pages()).count(),
        1
    );
    for mode in Mode::ALL {
        if !mode.runs_enhancement_probes() {
            assert!(!mode.applies_recommendations(), "{mode:?}");
            assert!(!mode.shows_filter_pages(), "{mode:?}");
        }
    }
}

// ── DV comes from two different places ──────────────────────────────────────

/// DV is not one kind of source. A DV camera recorded its own picture and has
/// no head-switching band; a DV digitiser fed from a VHS deck carries the
/// deck's band exactly as any other capture would. Nothing may key off "this
/// is DV" — the band is a measurement, and the two cases differ only in what
/// that measurement finds.
#[test]
fn dv_follows_the_measurement_not_the_capture_method() {
    let camera = EdgeCrop {
        left: 0,
        right: 0,
        top: 0,
        bottom: 0,
        noise_rows: 0,
    };
    let digitised_tape = EdgeCrop {
        left: 10,
        right: 8,
        top: 0,
        bottom: 8,
        noise_rows: 8,
    };

    for (name, edges, expect_band) in [
        ("DV camera original", camera, false),
        ("DV digitiser fed from VHS", digitised_tape, true),
    ] {
        let mut report = awful_tape();
        report.par = Some(ParFormat {
            format: Some(VideoFormat::Pal),
            capture_method: Some(DetectedCaptureMethod::Dv),
            sar: "16:15".into(),
        });
        report.edges = Some(Detection::Found(edges));

        let mut config = Config::default();
        apply_report(&mut config, &report, Mode::JustDeinterlace);

        assert_eq!(config.capture_method, CaptureMethod::Dv, "{name}");
        assert_eq!(
            config.head_switching_rows > 0,
            expect_band,
            "{name}: the band did not follow the measurement"
        );
        assert_eq!(
            config.head_switching_rows,
            if expect_band { 10 } else { 0 },
            "{name}"
        );
    }
}

/// The user can still overrule, which is the point: a measurement is
/// evidence, not a verdict. With one number there is one thing to change.
#[test]
fn the_bottom_crop_can_be_overruled() {
    let mut report = awful_tape();
    report.edges = Some(Detection::Found(EdgeCrop {
        left: 0,
        right: 0,
        top: 0,
        bottom: 6,
        noise_rows: 6,
    }));
    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::JustDeinterlace);
    assert_eq!(config.crop_bottom, 8, "6 found plus the margin");

    // Declined entirely.
    config.crop_bottom = 0;
    assert_eq!(config.crop_bottom, 0);

    // Or set by hand on a file where nothing was found.
    let mut clean = Config::default();
    assert_eq!(clean.crop_bottom, 0);
    clean.crop_bottom = 8;
    assert_eq!(clean.crop_bottom, 8);
}

// ── Field order, when the container does not declare one ────────────────────

/// Reported from testing on a MiniDV file: "Results are bad. Stepping. Looks
/// low res."
///
/// DV NTSC is bottom-field-first by specification. The container usually says
/// nothing, and `Unknown` used to fall through to the config default of TFF.
/// QTGMC fed the wrong dominance interpolates each field against its
/// neighbor in the wrong temporal order, so motion jerks back and forth and
/// edges step — the output looks less deinterlaced than the source.
#[test]
fn an_undeclared_dv_source_is_bottom_field_first() {
    let mut report = awful_tape();
    if let Some(p) = report.par.as_mut() {
        p.capture_method = Some(DetectedCaptureMethod::Dv);
    }
    // Static footage: idet cannot tell, so the format has to decide.
    report.telecine = Some(Telecine {
        pattern: None,
        confidence: None,
        progressive_source: false,
        windows: 2,
        film_windows: 0,
        prog_ratio: 0.0,
        rep_ratio: 0.0,
        tff_frames: 3,
        bff_frames: 2,
    });

    for mode in Mode::ALL {
        let mut config = Config::default();
        apply_report(&mut config, &report, mode);
        assert_eq!(
            config.field_order,
            FieldOrder::Bff,
            "{mode:?} deinterlaced DV as top-field-first"
        );
    }
}

/// An analog capture card is the other way round, and must stay that way.
#[test]
fn an_undeclared_analog_capture_is_top_field_first() {
    let report = awful_tape();
    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::EnhanceAutomatically);
    assert_eq!(config.field_order, FieldOrder::Tff);
}

/// What the picture shows outranks what the format implies: a DV file that
/// measurably reads top-field-first is top-field-first. The measurement is a
/// direct observation; the format is a convention, and conventions are broken
/// by converters and editing software every day.
#[test]
fn a_measured_dominance_outranks_the_format_default() {
    let mut report = awful_tape();
    if let Some(p) = report.par.as_mut() {
        p.capture_method = Some(DetectedCaptureMethod::Dv);
    }
    report.telecine = Some(Telecine {
        pattern: None,
        confidence: None,
        progressive_source: false,
        windows: 5,
        film_windows: 0,
        prog_ratio: 0.0,
        rep_ratio: 0.0,
        tff_frames: 400,
        bff_frames: 5,
    });

    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::EnhanceAutomatically);
    assert_eq!(config.field_order, FieldOrder::Tff);
}

/// A declared field order is never second-guessed. The container knowing is
/// the one case where there is nothing to infer.
#[test]
fn a_declared_field_order_wins_over_both() {
    let mut report = awful_tape();
    if let Some(c) = report.classification.as_mut() {
        c.field_order = DetectedFieldOrder::Bff;
    }
    if let Some(p) = report.par.as_mut() {
        p.capture_method = Some(DetectedCaptureMethod::Sd);
    }
    report.telecine = Some(Telecine {
        pattern: None,
        confidence: None,
        progressive_source: false,
        windows: 5,
        film_windows: 0,
        prog_ratio: 0.0,
        rep_ratio: 0.0,
        tff_frames: 400,
        bff_frames: 5,
    });

    let mut config = Config::default();
    apply_report(&mut config, &report, Mode::EnhanceAutomatically);
    assert_eq!(config.field_order, FieldOrder::Bff, "the container said BFF");
}
