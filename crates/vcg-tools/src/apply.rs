//! Turning an analysis report into a configuration.
//!
//! This is the one place a measurement becomes a setting, which is what makes
//! the mode guarantee checkable: `JustDeinterlace` can be shown, by test, never
//! to set an optional filter no matter what the analysis found.
//!
//! The split is deliberate and not negotiable at this layer:
//!
//! * **Structural** findings are applied in every mode — scan type, field
//!   order, video standard, pixel aspect, pixel format. Without them the output
//!   is simply wrong, not merely unenhanced.
//! * **Recommendations** are applied only in `EnhanceAutomatically`. In
//!   `EnhanceManually` they become the starting value of each page instead, and
//!   in `JustDeinterlace` they are not applied at all.

use vcg_core::analysis::edges::{self as edges, Detection};
use vcg_core::analysis::probe::{DetectedCaptureMethod, DetectedFieldOrder};
use vcg_core::analysis::Recommendation;
use vcg_core::config::{
    CaptureMethod, ColorCorrection, Config, CropPreset, DehaloMode, FieldOrder,
    Mode,
    NoiseLevel, SourceClassification,
};

use crate::probe::{Options, Report};

/// Which probes a mode needs.
///
/// `JustDeinterlace` skips noise, halos and color bleeding: they are the bulk
/// of the analysis time and nothing in that mode acts on them. Levels and
/// color are skipped too, for the same reason.
pub fn options_for(mode: Mode) -> Options {
    if mode.runs_enhancement_probes() {
        Options::default()
    } else {
        Options {
            // Scan type decides deinterlace versus inverse telecine, so it is
            // structural and always runs.
            telecine: true,
            halo: false,
            noise: false,
            bleeding: false,
            levels_color: false,
            // Edge detection runs here too. Cropping is on in this mode, so
            // leaving blanking in the picture would be a defect in the one
            // thing the mode promises to get right.
            edges: true,
            // And the sound check, for the same reason: a file that plays
            // out of one speaker is broken, not unenhanced.
            audio: true,
        }
    }
}

/// Apply a report to a configuration under the rules of `mode`.
pub fn apply_report(config: &mut Config, report: &Report, mode: Mode) {
    apply_structural(config, report);

    if mode.applies_recommendations() {
        apply_recommendations(config, report);
    } else if mode == Mode::JustDeinterlace {
        // Belt and braces: even if a recommendation were applied earlier, or
        // the user switched down from another mode, nothing optional survives.
        config.clear_enhancements();
    }
}

/// Findings the output is wrong without.
fn apply_structural(config: &mut Config, report: &Report) {
    let mut declared_field_order = false;
    if let Some(c) = &report.classification {
        config.source = SourceClassification {
            source_class: c.source_class,
            needs_pixfmt_conversion: c.needs_pixfmt_conversion,
            pix_fmt: c.pix_fmt.clone(),
            par_needed: c.par_needed,
        };

        // The container's declaration is used when it makes one. Most analog
        // captures do not, which is what the scan-type probe is for.
        //
        // `Unknown` used to fall through to the config default, which is TFF.
        // That is right for an analog capture card and wrong for DV, and the
        // wrongness is severe: QTGMC fed the wrong dominance interpolates
        // each field against its neighbor in the wrong temporal order, so
        // motion jerks back and forth every frame and edges step. A MiniDV
        // file reported from testing came out looking less deinterlaced than
        // the source. It is now measured rather than assumed — see below.
        match c.field_order {
            DetectedFieldOrder::Tff => config.field_order = FieldOrder::Tff,
            DetectedFieldOrder::Bff => config.field_order = FieldOrder::Bff,
            DetectedFieldOrder::Progressive => config.field_order = FieldOrder::Progressive,
            DetectedFieldOrder::Unknown => {}
        }
        declared_field_order = c.field_order != DetectedFieldOrder::Unknown;
    }

    if let Some(p) = &report.par {
        if let Some(format) = p.format {
            config.format = format;
        }
        config.detected_sar = p.sar.clone();
        if let Some(method) = p.capture_method {
            config.capture_method = match method {
                DetectedCaptureMethod::Dv => CaptureMethod::Dv,
                // DVD is MPEG-2 standard definition; the pipeline treats it as
                // an SD source with SD crop defaults.
                DetectedCaptureMethod::Sd | DetectedCaptureMethod::Dvd => CaptureMethod::Sd,
            };
        }
    }

    // A measured crop outranks the preset, in every mode.
    //
    // The presets can only ever be symmetric, and a capture card's blanking is
    // routinely not: the measured sample here needs 14 columns a side where
    // BT.601 crops 8, so the preset leaves 6 columns of blanking in the
    // picture. A measurement of zero is also worth keeping — it means the
    // frame is already all picture and the preset would eat 8 real columns.
    //
    // This is structural rather than a recommendation because a black bar in
    // the output is wrong, not unenhanced. `Inconclusive` changes nothing,
    // which is why it is a variant rather than an error.
    if let Some(Detection::Found(c)) = &report.edges {
        // A safety margin on every edge that found something. The measurement
        // locates where the junk stops, which is the one place not to cut: the
        // side boundary is a soft ramp and the bottom one a ragged tear, and
        // both wander a line or two between frames. Cutting just past them is
        // the difference between a clean edge and a flickering one.
        let c = c.with_margin(edges::SAFETY_MARGIN);

        config.crop_preset = Some(CropPreset::Manual);
        config.crop_left = c.left;
        config.crop_right = c.right;
        config.crop_top = c.top;
        // One bottom number, blanking and band together. They were two
        // settings with two controls, which let the bottom be cropped twice.
        config.crop_bottom = c.bottom;
        // Informational, already inside crop_bottom: it lets the screen say
        // why the bottom crop is the size it is.
        config.head_switching_rows = c.noise_rows;
    }

    // The picture's own black and white points, for the stretch option.
    //
    // A measurement, not a recommendation, so it is applied in every mode and
    // survives `clear_enhancements`: it says what the picture *is*, and only
    // takes effect if the user asks for a stretch. Nothing here turns one on.
    //
    // Percentiles rather than the outright extremes. One stuck pixel at 16
    // and one at 235 would make a stretch a no-op on a picture that plainly
    // needs one, and the option's own note already warns that it can clip.
    if let Some(h) = &report.histogram
        && !h.is_empty()
    {
        let black = h.percentile(0.001);
        let white = h.percentile(0.999);
        if white > black {
            config.levels_black = black;
            config.levels_white = white;
        }
    }

    // One-sided sound. Structural, like the crop: a file that plays out of
    // one speaker is broken rather than unenhanced, and the repair discards
    // nothing a listener can hear. Dual mono — the same signal on both sides,
    // which is what a mono tape over a stereo cable gives — is explicitly not
    // this, and is left alone.
    if let Some(a) = &report.audio {
        config.fix_one_sided_audio = a.verdict.can_fix();
        config.audio_source_channel = a.verdict.source_channel();
    }

    // Field order, when the container did not say.
    //
    // Measured first: idet counts how many frames read as each dominance, and
    // on anything with motion in it that is a direct observation of the thing
    // being guessed at. It abstains when the counts are close, which a static
    // shot produces.
    //
    // Failing that, the format decides. DV NTSC is bottom-field-first by
    // specification — that is how every camcorder records it — while an
    // analog capture card is top-field-first. The old code had neither step
    // and silently took the config default of TFF for both.
    if !declared_field_order && config.field_order != FieldOrder::Progressive {
        let measured = report.telecine.as_ref().and_then(|t| t.measured_field_order());
        config.field_order = match measured {
            Some(true) => FieldOrder::Tff,
            Some(false) => FieldOrder::Bff,
            None => match config.capture_method {
                CaptureMethod::Dv => FieldOrder::Bff,
                CaptureMethod::Sd => FieldOrder::Tff,
            },
        };
    }

    // Measured scan type outranks the container, because the container is
    // frequently wrong or silent and this was read off the picture.
    if let Some(t) = &report.telecine {
        if t.progressive_source {
            config.field_order = FieldOrder::Progressive;
        }
        // A manual film-versus-video choice outranks the measurement, and
        // keeps outranking it: this function runs again on every mode change,
        // so without the guard the user's answer would be overwritten by the
        // detection a second after they gave it.
        //
        // 3:2 and 2:2 are both inverse telecine; the .vpy generator picks the
        // right handling from the video standard, so the pattern itself does
        // not need carrying into the config.
        if !config.ivtc_user_override {
            config.ivtc_mode = !t.progressive_source && t.pattern.is_some();
        }
    }
}

/// Findings that only `EnhanceAutomatically` acts on.
fn apply_recommendations(config: &mut Config, report: &Report) {
    if let Some(n) = &report.noise {
        // Only trust the verdict when enough frames were measured.
        if n.analyzed {
            config.noise_level = match n.recommendation {
                Recommendation::Heavy => NoiseLevel::Heavy,
                Recommendation::Moderate => NoiseLevel::Moderate,
                // The noise probe never recommends light or strong; map them
                // conservatively rather than silently ignoring them.
                Recommendation::Light => NoiseLevel::Moderate,
                Recommendation::Strong => NoiseLevel::Heavy,
                Recommendation::None => NoiseLevel::None,
            };
        }
    }

    if let Some(h) = &report.halo
        && h.analyzed {
            config.dehalo_mode = match h.recommendation {
                Recommendation::Strong => DehaloMode::Strong,
                Recommendation::Light => DehaloMode::Light,
                Recommendation::Moderate => DehaloMode::Light,
                Recommendation::Heavy => DehaloMode::Strong,
                Recommendation::None => DehaloMode::None,
            };
        }

    if let Some(c) = &report.color {
        // Only a cast the probe judged *consistent* is corrected; a large but
        // scattered offset is scene variation, not a cast.
        if c.color_cast.is_some() {
            config.color_correction = ColorCorrection::AutoFix;
            config.u_correction = c.u_correction;
            config.v_correction = c.v_correction;
        }
    }

    // Levels are deliberately *not* auto-corrected, even when the picture
    // runs outside the legal range.
    //
    // Clamping is the one correction here that destroys data: the
    // super-blacks and super-whites on a tape capture are usually real
    // picture the deck recorded, and once clipped they cannot come back. A
    // mode called "enhance automatically" should not be the thing that
    // quietly throws them away — and anyone who needs broadcast compliance
    // knows to ask for it. The Levels page shows the range, draws it, and
    // leaves the decision there.

    // Color bleeding is deliberately *not* auto-applied. The correction is a
    // chroma shift measured in whole pixels, and the right amount is a visual
    // judgement rather than something the score determines — so it stays
    // informational and the Y/C Delay page is where it gets set.
}

/// Re-impose the facts that describe *this* file, after a settings file
/// written for another one has been loaded over the configuration.
///
/// A preset is a record of what somebody chose, and most of it travels: the
/// noise strength, the dehalo mode, the output format, the watermark, the
/// crop a deck has needed on every tape so far. A few fields in the same
/// object are not choices at all — they are measurements of the file that
/// produced them — and carrying those across is how a preset breaks a tape.
///
/// Field order is the one that matters. A preset made from an analog capture
/// says TFF; load it onto a DV file, which is bottom-field-first by
/// specification, and QTGMC interpolates every field against the neighbor on
/// the wrong side of it in time. Motion jerks back and forth and the output
/// looks *less* deinterlaced than the source — the exact defect the measured
/// field order exists to prevent. The crop is deliberately *not* in this
/// list: it is the thing a user most wants a preset to carry, it is visible
/// on the Crop page with the overlay drawn over the real frame, and a wrong
/// one is obvious there.
pub fn restore_source_facts(config: &mut Config, report: &Report) {
    // The crop is a choice here, so it is lifted out and put back around
    // `apply_structural`, which would otherwise overwrite it with this
    // file's own measurement.
    let crop = (
        config.crop_preset,
        config.crop_left,
        config.crop_right,
        config.crop_top,
        config.crop_bottom,
        config.head_switching_rows,
    );
    apply_structural(config, report);
    (
        config.crop_preset,
        config.crop_left,
        config.crop_right,
        config.crop_top,
        config.crop_bottom,
        config.head_switching_rows,
    ) = crop;
}
