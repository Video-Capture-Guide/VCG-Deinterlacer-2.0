//! Turning analysis results into plain language.
//!
//! The brief's rule: no technical term outside a Details panel. So each check
//! produces three separate strings — what is wrong and why it matters, what we
//! will do about it, and the numbers — and only the first two are ever shown
//! without the user asking.
//!
//! Nothing here reads or writes thresholds. The verdicts come from
//! `vcg_core::analysis`; this module only words them.

use vcg_core::analysis::classify::{
    BleedLevel, ColorCast, NoiseTrigger, NoiseVerdict, SaturationIssue,
};
use vcg_core::analysis::halo::HaloLevel;
use vcg_core::analysis::idet::Pattern;
use vcg_core::config::{Config, LevelsAdjustment, Mode, NoiseLevel};
use vcg_tools::probe::Report;

use crate::nav::Step;
use crate::theme::Tag;

/// Which check a finding came from. Used to route an Apply toggle to the right
/// configuration field and the right Settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Interlacing,
    Brightness,
    Halos,
    ColorTint,
    Noise,
    ColorBleeding,
    /// One-sided or single-channel sound. Not a picture check, and the only
    /// one whose fix is applied in every mode — a file that plays out of one
    /// speaker is broken rather than unenhanced. It is listed here so that
    /// the repair is visible on the page rather than appearing silently in
    /// the finished file.
    Audio,
}

impl Check {
    /// A stable id, for remembering which disclosures are open.
    pub fn id(self) -> &'static str {
        match self {
            Check::Interlacing => "interlacing",
            Check::Brightness => "brightness",
            Check::Halos => "halos",
            Check::ColorTint => "tint",
            Check::Noise => "noise",
            Check::ColorBleeding => "bleeding",
            Check::Audio => "audio",
        }
    }

    /// The Settings page that fine-tunes this, in Full control.
    pub fn settings_step(self) -> Option<Step> {
        match self {
            Check::Interlacing => None,
            Check::Brightness => Some(Step::Levels),
            Check::Halos => Some(Step::Halos),
            Check::ColorTint => Some(Step::ColorTint),
            Check::Noise => Some(Step::Noise),
            Check::ColorBleeding => Some(Step::ColorBleeding),
            Check::Audio => Some(Step::Audio),
        }
    }

    /// Whether the fix is currently switched on.
    ///
    /// Read from the configuration rather than from separate UI state, so a
    /// toggle here and the matching Settings page cannot disagree.
    pub fn is_on(self, config: &Config) -> bool {
        match self {
            // Deinterlacing is not optional.
            Check::Interlacing => true,
            Check::Brightness => config.levels_adjustment != LevelsAdjustment::None,
            Check::Halos => config.dehalo_mode != vcg_core::config::DehaloMode::None,
            Check::ColorTint => {
                config.color_correction != vcg_core::config::ColorCorrection::None
            }
            Check::Noise => config.noise_level != NoiseLevel::None,
            Check::ColorBleeding => config.yc_delay != 0 || config.chroma_shift,
            Check::Audio => config.fix_one_sided_audio,
        }
    }

    /// Switch the fix on or off, writing the same field the Settings page reads.
    ///
    /// `report` supplies the measured strength, so turning a fix back on
    /// restores what the analysis recommended rather than a guess.
    pub fn set(self, config: &mut Config, report: &Report, on: bool) {
        match self {
            Check::Interlacing => {}
            Check::Brightness => {
                config.levels_adjustment = if on {
                    LevelsAdjustment::Clamp
                } else {
                    LevelsAdjustment::None
                };
            }
            Check::Halos => {
                config.dehalo_mode = if on {
                    report
                        .halo
                        .as_ref()
                        .map(|h| match h.halo_level {
                            HaloLevel::Strong => vcg_core::config::DehaloMode::Strong,
                            _ => vcg_core::config::DehaloMode::Light,
                        })
                        .unwrap_or(vcg_core::config::DehaloMode::Light)
                } else {
                    vcg_core::config::DehaloMode::None
                };
            }
            Check::ColorTint => {
                if on {
                    config.color_correction = vcg_core::config::ColorCorrection::AutoFix;
                    if let Some(c) = &report.color {
                        config.u_correction = c.u_correction;
                        config.v_correction = c.v_correction;
                    }
                } else {
                    config.color_correction = vcg_core::config::ColorCorrection::None;
                    config.u_correction = 0.0;
                    config.v_correction = 0.0;
                }
            }
            Check::Noise => {
                config.noise_level = if on {
                    report
                        .noise
                        .as_ref()
                        .map(|n| match n.noise_level {
                            NoiseVerdict::Heavy => NoiseLevel::Heavy,
                            _ => NoiseLevel::Moderate,
                        })
                        .unwrap_or(NoiseLevel::Moderate)
                } else {
                    NoiseLevel::None
                };
            }
            // The shift itself is a visual judgement, so this only arms the
            // legacy flag; the amount is set on the Color bleeding page.
            Check::ColorBleeding => config.chroma_shift = on,
            Check::Audio => {
                config.fix_one_sided_audio = on;
                if on && let Some(a) = &report.audio {
                    config.audio_source_channel = a.verdict.source_channel();
                }
            }
        }
    }
}

/// One check, worded for the screen.
#[derive(Clone)]
pub struct Finding {
    pub check: Check,
    /// Plain-language name, e.g. "Bright outlines around edges".
    pub title: String,
    /// Status, and the text to show on it.
    pub tag: Tag,
    pub tag_text: Option<String>,
    /// What is wrong and why it matters. One or two sentences, no jargon.
    pub explanation: String,
    /// What the pipeline will do, if anything. Without the leading "We'll".
    pub action: Option<String>,
    /// The numbers, for the Why?/Details disclosure.
    pub details: String,
    /// A blue observation shown under the row.
    pub note: Option<String>,
    /// A short label for the summary pills, e.g. "Brightness fixed".
    /// Title for a "We also noticed" row, phrased as the symptom the viewer
    /// would see rather than as the name of a measurement.
    pub notice_title: String,
    /// One sentence on what that looks like in practice.
    pub notice_text: String,
    /// The "What does this mean?" explainer. Only interlacing has one, because
    /// it is the only thing that always happens and the only one a newcomer
    /// has to understand to trust the result.
    pub meaning: Option<String>,
    /// A plain sentence from the film check, shown under the explainer.
    pub film_line: Option<String>,
}

impl Finding {
    /// Whether this is a recommended fix rather than a passed check.
    pub fn is_fix(&self) -> bool {
        matches!(self.tag, Tag::Required | Tag::FixRecommended)
    }

}

/// Everything the report found, worded, in display order.
///
/// Interlacing comes first because it is the one fix that always applies.
pub fn collect(report: &Report) -> Vec<Finding> {
    let mut out = Vec::new();
    out.push(interlacing(report));
    if let Some(f) = brightness(report) {
        out.push(f);
    }
    if let Some(f) = halos(report) {
        out.push(f);
    }
    if let Some(f) = color_tint(report) {
        out.push(f);
    }
    if let Some(f) = noise(report) {
        out.push(f);
    }
    if let Some(f) = color_bleeding(report) {
        out.push(f);
    }
    if let Some(f) = audio(report) {
        out.push(f);
    }
    out
}

/// Whether the probes behind brightness, color, noise, halos and bleeding
/// were actually run.
///
/// `Just deinterlace` skips all five — they are the bulk of the analysis time
/// and nothing in that mode acts on them. The page must not then report that
/// those things look fine: it did not look. Reporting a clean bill of health
/// from an examination that never happened is the one thing that would make
/// every other finding untrustworthy.
pub fn enhancement_probes_ran(report: &Report) -> bool {
    report.noise.is_some()
        || report.halo.is_some()
        || report.bleeding.is_some()
        || report.levels.is_some()
        || report.color.is_some()
}

/// How many frames the analysis actually measured, for the header line.
pub fn frames_measured(report: &Report) -> usize {
    report
        .noise
        .as_ref()
        .map(|n| n.samples_analyzed)
        .filter(|n| *n > 0)
        .or_else(|| {
            report
                .telecine
                .as_ref()
                .map(|t| t.windows as usize * 240)
                .filter(|n| *n > 0)
        })
        .unwrap_or(0)
}

// ── Scan type ───────────────────────────────────────────────────────────────

/// The "What does this mean?" explainer, shared by every scan-type finding.
///
/// One constant rather than the four copies this used to be: the wording is
/// the user's, and four copies is three places for an edit to be missed.
///
/// "Software method" rather than "method" because the best deinterlacers are
/// hardware — a broadcast line doubler beats anything running on a laptop —
/// and the unqualified claim is one we cannot keep.
const WHAT_INTERLACING_MEANS: &str = "Old TVs drew every picture in two passes \
    — odd lines, then even lines — a split second apart. Computers show both \
    at once, so moving objects look torn. Deinterlacing rebuilds each picture \
    so it's whole. We use QTGMC, the highest-quality software method available, \
    set up to deinterlace and nothing else: no denoising, almost no sharpening, \
    and source matching that keeps the result true to the lines your deck \
    actually recorded. Removing noise is a separate step you choose yourself.";

fn interlacing(report: &Report) -> Finding {
    let Some(t) = &report.telecine else {
        return Finding {
            check: Check::Interlacing,
            title: "Interlacing lines".into(),
            tag: Tag::Required,
            tag_text: None,
            explanation: "Tape video stores each frame as two interleaved halves. \
                          These are combined into whole frames."
                .into(),
            action: Some("deinterlace with QTGMC (best quality)".into()),
            details: "Scan type was not measured.".into(),
            note: None,
            notice_title: "Movement looks jagged".into(),
            notice_text: "Anything that moves shows comb-like lines on a computer or phone."
                .into(),
            meaning: Some(WHAT_INTERLACING_MEANS.into()),
            film_line: Some(
                "Your video was recorded on a video camera, not film, so no extra film correction is needed.".into(),
            ),
        };
    };

    if t.progressive_source {
        return Finding {
            check: Check::Interlacing,
            title: "Interlacing lines".into(),
            tag: Tag::Info,
            tag_text: Some("Already progressive".into()),
            explanation: "This file already holds whole frames rather than \
                          interleaved halves, so there are no comb-like lines \
                          to remove."
                .into(),
            action: None,
            details: format!(
                "{:.0}% of sampled frames are progressive across {} window(s) \
                 → no deinterlacing needed.",
                t.prog_ratio * 100.0,
                t.windows
            ),
            note: None,
            notice_title: "Movement looks jagged".into(),
            notice_text: "Anything that moves shows comb-like lines on a computer or phone."
                .into(),
            meaning: Some(WHAT_INTERLACING_MEANS.into()),
            film_line: Some(
                "This file already holds whole pictures rather than interleaved halves, so there is nothing to rebuild.".into(),
            ),
        };
    }

    match t.pattern {
        Some(pattern) => Finding {
            check: Check::Interlacing,
            title: "Interlacing lines".into(),
            tag: Tag::Required,
            tag_text: Some("Film — required".into()),
            explanation: "This was shot on film and then transferred to tape, \
                          which repeats some frames to fill the video frame rate. \
                          Those repeats are removed to get back the original film \
                          frames."
                .into(),
            action: Some(format!(
                "remove the pulldown and restore the original {:.3} frames per second",
                pattern.target_fps()
            )),
            details: format!(
                "{} pattern found in {} of {} window(s) · {:.0}% progressive · \
                 {:.0}% repeated fields → film-sourced; pulldown removal used.",
                match pattern {
                    Pattern::ThreeTwo => "3:2",
                    Pattern::TwoTwo => "2:2",
                },
                t.film_windows,
                t.windows,
                t.prog_ratio * 100.0,
                t.rep_ratio * 100.0
            ),
            note: None,
            notice_title: "Movement looks jagged".into(),
            notice_text: "Anything that moves shows comb-like lines on a computer or phone."
                .into(),
            meaning: Some(WHAT_INTERLACING_MEANS.into()),
            film_line: Some(
                "Parts of this video came from film, so we'll also undo the film-to-video conversion.".into(),
            ),
        },
        None => Finding {
            check: Check::Interlacing,
            title: "Interlacing lines".into(),
            tag: Tag::Required,
            tag_text: None,
            explanation: "The video is interlaced, which is normal for video \
                          captured from VHS and similar formats. The software \
                          will deinterlace the video so that the noticeable \
                          “stepping” artifact is removed."
                .into(),
            action: Some("deinterlace with QTGMC (best quality)".into()),
            details: format!(
                "No film pattern found in {} window(s) · {:.0}% progressive · \
                 {:.0}% repeated fields → Not film-sourced; no pulldown removal needed.",
                t.windows,
                t.prog_ratio * 100.0,
                t.rep_ratio * 100.0
            ),
            note: None,
            notice_title: "Movement looks jagged".into(),
            notice_text: "Anything that moves shows comb-like lines on a computer or phone."
                .into(),
            meaning: Some(WHAT_INTERLACING_MEANS.into()),
            film_line: Some(
                "Your video was recorded on a video camera, not film, so no extra film correction is needed.".into(),
            ),
        },
    }
}

// ── Brightness ──────────────────────────────────────────────────────────────

fn brightness(report: &Report) -> Option<Finding> {
    let l = report.levels.as_ref()?;
    let (min, max) = (l.min_y?, l.max_y?);
    let blacks_low = min < 16;
    let whites_high = max > 235;

    let details = format!(
        "Luma measured {min}–{max} · legal range 16–235 → {}",
        match (blacks_low, whites_high) {
            (true, true) => format!("blacks {} below legal; whites {} above.", 16 - min, max - 235),
            (true, false) => format!("blacks {} below legal; whites within range.", 16 - min),
            (false, true) => format!("whites {} above legal; blacks within range.", max - 235),
            (false, false) => "both within range.".to_string(),
        }
    );

    if !l.needs_adjustment {
        return Some(Finding {
            check: Check::Brightness,
            title: "Brightness range".into(),
            tag: Tag::LooksGood,
            tag_text: None,
            explanation: "Blacks and whites sit inside the standard range.".into(),
            action: None,
            details,
            note: None,
            notice_title: "Shadows are a little too dark".into(),
            notice_text: "Some dark areas may look like solid black with no detail on TVs and phones."
                .into(),
            meaning: None,
            film_line: None,
        });
    }

    let explanation = match (blacks_low, whites_high) {
        (true, true) => "The darkest parts of the picture go below standard black \
                         and the brightest go above standard white. Some players \
                         and TVs will show crushed shadows and blown-out highlights."
            .to_string(),
        (true, false) => "The darkest parts of the picture go below standard black. \
                          Some players and TVs will show crushed, detail-less shadows."
            .to_string(),
        _ => "The brightest parts go above standard white and may look blown out."
            .to_string(),
    };

    Some(Finding {
        check: Check::Brightness,
        title: "Brightness range".into(),
        tag: Tag::FixRecommended,
        tag_text: None,
        explanation,
        action: Some("bring blacks and whites into the standard range".into()),
        details,
        // The one recommended fix that starts switched off, and the only
        // card that has to say why: every other correction here can be
        // undone by exporting again, and this one cannot. Blacks below 16
        // and whites above 235 on a tape capture are usually picture the
        // deck really recorded; bringing them inside the legal window
        // clips them, and nothing gets them back. The old software says
        // the same and is right. The decision stays with the reader rather
        // than being made for them by a mode called "enhance
        // automatically".
        note: Some(
            "This one is left off by default. Unlike the others it throws \
             data away — the detail outside the standard range is clipped, \
             and exporting again will not bring it back."
                .into(),
        ),
        notice_title: "Shadows are a little too dark".into(),
        notice_text: "Some dark areas may look like solid black with no detail on TVs and phones."
            .into(),
        meaning: None,
        film_line: None,
    })
}

// ── Halos ───────────────────────────────────────────────────────────────────

fn halos(report: &Report) -> Option<Finding> {
    let h = report.halo.as_ref()?;
    if !h.analyzed {
        return Some(Finding {
            check: Check::Halos,
            title: "Bright outlines around edges".into(),
            tag: Tag::Info,
            tag_text: Some("Not enough edges".into()),
            explanation: "There weren't enough sharp edges in the sampled frames \
                          to judge this."
                .into(),
            action: None,
            details: format!("{} strong edges found — too few to measure.", h.edges_analyzed),
            note: None,
            notice_title: "Faint outlines around edges".into(),
            notice_text: "Some edges have a thin bright line around them, added by the VCR or camcorder."
                .into(),
            meaning: None,
            film_line: None,
        });
    }

    let details = format!(
        "{:.1}% of {} strong edges overshoot",
        h.halo_ratio * 100.0,
        h.edges_analyzed
    );

    if h.halo_level == HaloLevel::Clean {
        return Some(Finding {
            check: Check::Halos,
            title: "Bright outlines around edges".into(),
            tag: Tag::LooksGood,
            tag_text: None,
            explanation: "Edges are clean — no sharpening outlines to remove.".into(),
            action: None,
            details,
            note: None,
            notice_title: "Faint outlines around edges".into(),
            notice_text: "Some edges have a thin bright line around them, added by the VCR or camcorder."
                .into(),
            meaning: None,
            film_line: None,
        });
    }

    let severity = match h.halo_level {
        HaloLevel::Strong => "strong",
        HaloLevel::Moderate => "moderate",
        _ => "light",
    };
    // A percentage means little to most people; "1 in N edges" does.
    let one_in = if h.halo_ratio > 0.0 {
        (1.0 / h.halo_ratio).round().max(2.0) as u32
    } else {
        0
    };

    Some(Finding {
        check: Check::Halos,
        title: "Bright outlines around edges".into(),
        tag: Tag::FixRecommended,
        tag_text: Some(format!("{} — fix recommended", capitalise(severity))),
        explanation: format!(
            "About 1 in {one_in} sharp edges has a faint bright or dark outline. \
             This usually comes from sharpening in the VCR or camcorder."
        ),
        action: Some(format!("apply {severity} halo removal")),
        details,
        note: None,
        notice_title: "Faint outlines around edges".into(),
        notice_text: "Some edges have a thin bright line around them, added by the VCR or camcorder."
            .into(),
        meaning: None,
        film_line: None,
    })
}

// ── Color tint ─────────────────────────────────────────────────────────────

fn color_tint(report: &Report) -> Option<Finding> {
    let c = report.color.as_ref()?;
    let details = format!(
        "U {:+.1} · V {:+.1} from neutral (avg. of sampled frames) · saturation {:.1}",
        c.u_offset, c.v_offset, c.sat_avg
    );

    // Very faint color is worth mentioning but is not a fault.
    let note = match c.sat_issue {
        Some(SaturationIssue::VeryLow) => Some(
            "Colors are very faint. That's normal for black-and-white or faded footage."
                .to_string(),
        ),
        _ => None,
    };

    match c.color_cast {
        None => Some(Finding {
            check: Check::ColorTint,
            title: "Color tint".into(),
            tag: Tag::LooksGood,
            tag_text: None,
            explanation: "No color cast — colors are balanced.".into(),
            action: None,
            details,
            note,
            notice_title: "Colors lean one way".into(),
            notice_text: "The whole picture has a slight color wash over it."
                .into(),
            meaning: None,
            film_line: None,
        }),
        Some(cast) => {
            let direction = match cast {
                ColorCast::Blue => "blue",
                ColorCast::Yellow => "yellow",
                ColorCast::RedMagenta => "red",
                ColorCast::GreenCyan => "green",
            };
            Some(Finding {
                check: Check::ColorTint,
                title: "Color tint".into(),
                tag: Tag::FixRecommended,
                tag_text: None,
                explanation: format!("The picture has a {direction} tint."),
                action: Some("neutralise the tint".into()),
                details,
                note,
                notice_title: "Colors lean one way".into(),
                notice_text: "The whole picture has a slight color wash over it."
                    .into(),
                meaning: None,
                film_line: None,
            })
        }
    }
}

// ── Noise ───────────────────────────────────────────────────────────────────

fn noise(report: &Report) -> Option<Finding> {
    let n = report.noise.as_ref()?;
    let details = format!(
        "Temporal outliers {:.4} · frame-to-frame difference {:.1} · \
         colour difference {:.2} (ratio {:.2}) · {} frames measured",
        n.avg_variance, n.avg_diff, n.avg_chroma_diff, n.chroma_ratio, n.samples_analyzed
    );

    if !n.analyzed {
        return Some(Finding {
            check: Check::Noise,
            title: "Tape noise".into(),
            tag: Tag::Info,
            tag_text: Some("Not enough frames".into()),
            explanation: "Too few frames could be measured to judge noise.".into(),
            action: None,
            details,
            note: None,
            notice_title: "Grainy, speckled picture".into(),
            notice_text: "Fine specks move about from frame to frame, the way old tape does."
                .into(),
            meaning: None,
            film_line: None,
        });
    }

    if n.noise_level == NoiseVerdict::Clean {
        return Some(Finding {
            check: Check::Noise,
            title: "Tape noise".into(),
            tag: Tag::LooksGood,
            tag_text: None,
            explanation: "Picture is clean — no noise reduction needed.".into(),
            action: None,
            details,
            note: None,
            notice_title: "Grainy, speckled picture".into(),
            notice_text: "Fine specks move about from frame to frame, the way old tape does."
                .into(),
            meaning: None,
            film_line: None,
        });
    }

    let severity = match n.noise_level {
        NoiseVerdict::Heavy => "heavy",
        NoiseVerdict::Moderate => "moderate",
        _ => "light",
    };

    // When only the colour planes are noisy the grain-and-speckle wording is
    // wrong — the luma looked clean, and what the viewer sees is crawling
    // colour rather than speckle.
    let explanation = if n.trigger == NoiseTrigger::Chroma {
        "The grey part of the picture is clean, but the colour is noisy — \
         patches of colour shimmer and crawl from frame to frame."
    } else {
        "The picture has visible tape grain and speckle that moves \
         from frame to frame."
    };

    Some(Finding {
        check: Check::Noise,
        title: "Tape noise".into(),
        tag: Tag::FixRecommended,
        tag_text: Some(format!("{} — fix recommended", capitalise(severity))),
        explanation: explanation.into(),
        action: Some(format!(
            "apply {} noise reduction",
            if severity == "heavy" { "heavy" } else { "moderate" }
        )),
        details,
        note: None,
        notice_title: "Grainy, speckled picture".into(),
        notice_text: "Fine specks move about from frame to frame, the way old tape does."
            .into(),
        meaning: None,
        film_line: None,
    })
}

// ── Color bleeding ─────────────────────────────────────────────────────────

fn color_bleeding(report: &Report) -> Option<Finding> {
    let b = report.bleeding.as_ref()?;
    let details = format!(
        "Chroma spread U {:.1} / V {:.1} · weighted score {:.1}",
        b.avg_h_spread, b.avg_v_spread, b.bleeding_score
    );

    if !b.recommendation || b.bleed_level == BleedLevel::None {
        return Some(Finding {
            check: Check::ColorBleeding,
            title: "Color bleeding".into(),
            tag: Tag::LooksGood,
            tag_text: None,
            explanation: "Color lines up with the picture — no shift to correct.".into(),
            action: None,
            details,
            note: None,
            notice_title: "Color is offset from the picture".into(),
            notice_text: "Color sits slightly to one side of the shapes it belongs to, so edges smear."
                .into(),
            meaning: None,
            film_line: None,
        });
    }

    Some(Finding {
        check: Check::ColorBleeding,
        title: "Color bleeding".into(),
        tag: Tag::FixRecommended,
        tag_text: None,
        explanation: "Color is shifted sideways from the picture, causing \
                      smeared edges."
            .into(),
        action: Some("line the color back up with the picture".into()),
        details,
        note: None,
        notice_title: "Color is offset from the picture".into(),
        notice_text: "Color sits slightly to one side of the shapes it belongs to, so edges smear."
            .into(),
        meaning: None,
        film_line: None,
    })
}

// ── Sound ───────────────────────────────────────────────────────────────────

/// What will happen to the sound, said out loud.
///
/// Reported from testing: a mono file was loaded in auto-fix, the sound was
/// repaired, and nothing on the Check page mentioned it. The repair had been
/// treated as structural — applied in every mode, like the crop — and the
/// page only ever listed picture findings, so the one change the viewer could
/// *hear* was the one change nobody was told about.
///
/// Three outcomes, and the wording matters because two of them sound alike:
///
/// * **One channel.** Real mono. Most players center it, some put it on the
///   left only, and copying it to both sides removes the doubt. A convenience
///   rather than a defect, so the tag says so.
/// * **Two channels, one silent.** The actual fault — headphones get silence
///   on one ear.
/// * **Two channels carrying the same thing.** Dual mono, which is what a mono
///   tape over a stereo cable gives. Nothing to do; "fixing" it would be a
///   no-op dressed up as a repair.
///
/// Returns `None` when there is no audio probe at all, rather than claiming
/// the sound is fine — same rule as the picture checks.
fn audio(report: &Report) -> Option<Finding> {
    use vcg_core::analysis::audio::AudioVerdict;

    let a = report.audio.as_ref()?;
    let details = format!(
        "{} channel(s); {}",
        a.channels,
        a.levels
            .iter()
            .enumerate()
            .map(|(i, l)| match l.rms_db {
                Some(db) => format!("ch{i} RMS {db:.1} dBFS"),
                None => format!("ch{i} silent"),
            })
            .collect::<Vec<_>>()
            .join(" · ")
    );

    const NOTICE_TITLE: &str = "The sound does not come out of both speakers";
    const NOTICE_TEXT: &str =
        "On headphones one ear gets nothing, which is unmissable once heard.";

    let (tag, explanation, action) = match a.verdict {
        AudioVerdict::None => return None,
        AudioVerdict::Fine => (
            Tag::LooksGood,
            "Both channels carry sound. Nothing to change.".to_string(),
            None,
        ),
        AudioVerdict::Mono => (
            Tag::FixRecommended,
            "The file carries one channel rather than two. Most players center it, \
             but some put it on the left only."
                .to_string(),
            Some("copy the single channel to both sides, so it plays the same everywhere".to_string()),
        ),
        AudioVerdict::SilentSide { live } => (
            Tag::FixRecommended,
            format!(
                "One of the two channels is empty, so the sound plays out of one \
                 speaker only. The {} channel is the live one.",
                if live == 0 { "left" } else { "right" }
            ),
            Some("copy the live channel to both sides".to_string()),
        ),
    };

    Some(Finding {
        check: Check::Audio,
        title: "Sound".into(),
        tag,
        tag_text: None,
        explanation,
        action,
        details,
        // Said plainly because it is the one repair that discards nothing: the
        // live channel is duplicated, not mixed, so there is no quality cost
        // to weigh against it.
        note: None,
        notice_title: NOTICE_TITLE.into(),
        notice_text: NOTICE_TEXT.into(),
        meaning: None,
        film_line: None,
    })
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Whether this mode shows a fine-tune link on each fix card.
pub fn shows_fine_tune(mode: Mode) -> bool {
    mode.shows_filter_pages()
}
