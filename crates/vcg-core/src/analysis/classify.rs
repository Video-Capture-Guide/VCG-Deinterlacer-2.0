//! Threshold classifiers for the signalstats-based probes.
//!
//! Ported from `analyze_video_levels`, `analyze_color_data`,
//! `analyze_noise_level` and `analyze_color_bleeding`. Each takes the parsed
//! runs and returns a verdict — no process spawning, so every threshold can be
//! exercised directly.
//!
//! The numeric cut-offs are the Python's, including the comments explaining how
//! they were calibrated. They are deliberately not "tidied": they encode
//! measurements against real captures.

use super::signalstats::{mean, std_dev, SignalStatsRun};

// ── Levels ──────────────────────────────────────────────────────────────────

/// Luma range across the sampled frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levels {
    /// `None` when no frame was measured.
    pub min_y: Option<i64>,
    pub max_y: Option<i64>,
    /// Luma falls outside the legal 16-235 range.
    pub needs_adjustment: bool,
}

pub fn levels(runs: &[SignalStatsRun]) -> Levels {
    // Seeded at the opposite extremes so the first measurement always wins;
    // a sentinel left untouched reads back as `None`.
    let mut min_y = 255i64;
    let mut max_y = 0i64;
    for run in runs {
        for v in &run.ymin {
            min_y = min_y.min(*v);
        }
        for v in &run.ymax {
            max_y = max_y.max(*v);
        }
    }
    Levels {
        min_y: (min_y != 255).then_some(min_y),
        max_y: (max_y != 0).then_some(max_y),
        // Note this is computed from the sentinels too, matching the Python:
        // a file where nothing could be measured reports no adjustment needed.
        needs_adjustment: min_y < 16 || max_y > 235,
    }
}

// ── Color cast ─────────────────────────────────────────────────────────────

/// Which way a persistent color cast leans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCast {
    Blue,
    Yellow,
    RedMagenta,
    GreenCyan,
}

/// How far saturation sits from normal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaturationIssue {
    VeryLow,
    Low,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorData {
    pub u_avg: f64,
    pub v_avg: f64,
    pub sat_avg: f64,
    pub u_offset: f64,
    pub v_offset: f64,
    pub u_std: f64,
    pub v_std: f64,
    pub color_cast: Option<ColorCast>,
    pub sat_issue: Option<SaturationIssue>,
    /// The shift to apply to cancel the cast — the negated offset.
    pub u_correction: f64,
    pub v_correction: f64,
}

/// 10 units = a 7.8% shift — visibly warm or cool, worth flagging.
///
/// The original threshold of 3 (2.3%) was normal analog variation and far too
/// aggressive.
const CAST_THRESHOLD: f64 = 10.0;

/// Returns `None` when no U or V samples were collected, as the Python does.
pub fn color_data(runs: &[SignalStatsRun]) -> Option<ColorData> {
    let u: Vec<f64> = runs.iter().flat_map(|r| r.uavg.iter().copied()).collect();
    let v: Vec<f64> = runs.iter().flat_map(|r| r.vavg.iter().copied()).collect();
    let sat: Vec<f64> = runs.iter().flat_map(|r| r.satavg.iter().copied()).collect();

    if u.is_empty() || v.is_empty() {
        return None;
    }

    let u_avg = mean(&u);
    let v_avg = mean(&v);
    let sat_avg = mean(&sat);
    let u_offset = u_avg - 128.0;
    let v_offset = v_avg - 128.0;
    let u_std = std_dev(&u);
    let v_std = std_dev(&v);
    let n = u.len();

    // A real cast is persistent, so require it to be consistent across frames
    // (stddev under 1.5x the offset) — one bright or dark scene must not
    // mislead the analysis.
    let notable = |offset: f64, sd: f64| {
        offset.abs() > CAST_THRESHOLD && (sd < offset.abs() * 1.5 || n <= 1)
    };

    let color_cast = if notable(u_offset, u_std) || notable(v_offset, v_std) {
        Some(if u_offset.abs() >= v_offset.abs() {
            if u_offset > 0.0 {
                ColorCast::Blue
            } else {
                ColorCast::Yellow
            }
        } else if v_offset > 0.0 {
            ColorCast::RedMagenta
        } else {
            ColorCast::GreenCyan
        })
    } else {
        None
    };

    let sat_issue = if sat_avg < 20.0 {
        Some(SaturationIssue::VeryLow)
    } else if sat_avg < 35.0 {
        Some(SaturationIssue::Low)
    } else if sat_avg > 80.0 {
        Some(SaturationIssue::High)
    } else {
        None
    };

    Some(ColorData {
        u_avg,
        v_avg,
        sat_avg,
        u_offset,
        v_offset,
        u_std,
        v_std,
        color_cast,
        sat_issue,
        u_correction: -u_offset,
        v_correction: -v_offset,
    })
}

// ── Noise ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseVerdict {
    Clean,
    Light,
    Moderate,
    Heavy,
}

/// Which metric crossed the threshold, so the UI can show the right number.
///
/// A quiet tape with heavy motion is classified via YDIF while its TOUT score
/// stays low. `Chroma` means the luma looked clean and only the colour planes
/// were noisy — the case the luma-only classifier used to miss entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseTrigger {
    Tout,
    Ydif,
    Chroma,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseAnalysis {
    /// The overall verdict: the worse of the luma and chroma verdicts.
    pub noise_level: NoiseVerdict,
    /// The filter strength to recommend. Note Light recommends Moderate.
    pub recommendation: super::Recommendation,
    pub trigger: NoiseTrigger,
    pub avg_diff: f64,
    pub std_diff: f64,
    pub avg_variance: f64,
    /// What the luma metrics alone say — what this probe used to report.
    pub luma_level: NoiseVerdict,
    /// Mean of UDIF and VDIF: inter-frame difference on the colour planes.
    pub avg_chroma_diff: f64,
    /// `avg_chroma_diff / avg_diff`, the motion-cancelling ratio. See
    /// `chroma_noise`.
    pub chroma_ratio: f64,
    pub chroma_level: NoiseVerdict,
    pub samples_analyzed: usize,
    /// At least three good samples were collected.
    pub analyzed: bool,
}

/// Judge the colour planes, which TOUT cannot see.
///
/// Calibrated 23 October 2026 by adding known chroma noise to a real VHS
/// capture (`signalstats` over 15s, the same filter the noise probe runs):
///
/// | added chroma sigma | chroma diff | ratio |
/// |---|---|---|
/// | none (as captured) | 0.83 | 0.14 |
/// | 5  | 3.01  | 0.50 |
/// | 10 | 6.18  | 1.03 |
/// | 20 | 12.66 | 2.11 |
/// | 40 | 25.70 | 4.28 |
///
/// Five real captures with no added noise — VHS, DV, an HD camcorder and two
/// restored files — sit at ratios 0.13 to 0.38.
///
/// **Two conditions, not one, and this matters.** UDIF and VDIF carry the same
/// motion contamination as YDIF, so the raw figure cannot stand alone: a
/// restored PAL DV clip full of movement measures a chroma diff of 5.56, above
/// the sigma-5 case, purely from motion. Its *ratio* is 0.18, because motion
/// raises luma and chroma together and dividing cancels it. The ratio alone is
/// no good either — a near-still scene has a tiny YDIF and an inflated ratio —
/// so an absolute floor guards it. Adding luma noise pushes the ratio *down*
/// (0.031 at sigma 40), so a grainy-luma tape cannot false-positive here.
fn chroma_noise(avg_chroma_diff: f64, avg_diff: f64) -> (f64, NoiseVerdict) {
    // Guard the division: a file with no luma movement at all would otherwise
    // produce an infinite ratio and classify everything as heavy.
    let ratio = if avg_diff > 0.01 { avg_chroma_diff / avg_diff } else { 0.0 };

    let verdict = if ratio > 1.0 && avg_chroma_diff > 4.0 {
        NoiseVerdict::Heavy
    } else if ratio > 0.45 && avg_chroma_diff > 2.0 {
        NoiseVerdict::Moderate
    } else {
        NoiseVerdict::Clean
    };
    (ratio, verdict)
}

/// Rank for picking the worse of two verdicts.
fn severity(v: NoiseVerdict) -> u8 {
    match v {
        NoiseVerdict::Clean => 0,
        NoiseVerdict::Light => 1,
        NoiseVerdict::Moderate => 2,
        NoiseVerdict::Heavy => 3,
    }
}

pub fn noise(runs: &[SignalStatsRun]) -> NoiseAnalysis {
    let diff: Vec<f64> = runs.iter().flat_map(|r| r.ydif.iter().copied()).collect();
    let variance: Vec<f64> = runs.iter().flat_map(|r| r.tout.iter().copied()).collect();

    let avg_diff = mean(&diff);
    let std_diff = std_dev(&diff);
    let avg_variance = mean(&variance);

    // YDIF measures inter-frame pixel difference, which reflects BOTH scene
    // motion AND noise — a busy scene has high YDIF even on clean video. TOUT
    // (temporal outliers) counts pixels changing inconsistently with their
    // neighbors, i.e. speckle rather than smooth motion, so the classifier
    // leads on TOUT and keeps YDIF as a secondary trigger.
    //
    // The TOUT cut-offs were originally far too conservative (0.06 for
    // moderate) and missed most genuine VHS tape noise.
    let (luma_level, recommendation, trigger) = if avg_variance > 0.08 || avg_diff > 30.0 {
        (
            NoiseVerdict::Heavy,
            super::Recommendation::Heavy,
            if avg_variance > 0.08 { NoiseTrigger::Tout } else { NoiseTrigger::Ydif },
        )
    } else if avg_variance > 0.03 || avg_diff > 20.0 {
        (
            NoiseVerdict::Moderate,
            super::Recommendation::Moderate,
            if avg_variance > 0.03 { NoiseTrigger::Tout } else { NoiseTrigger::Ydif },
        )
    } else if avg_variance > 0.012 || avg_diff > 12.0 {
        // Light noise still recommends moderate denoising.
        (
            NoiseVerdict::Light,
            super::Recommendation::Moderate,
            if avg_variance > 0.012 { NoiseTrigger::Tout } else { NoiseTrigger::Ydif },
        )
    } else {
        (NoiseVerdict::Clean, super::Recommendation::None, NoiseTrigger::Tout)
    };

    // Chroma is judged separately, because TOUT is blind to it and YDIF
    // measures the wrong planes. The two verdicts then combine: the worse one
    // wins, so a tape with clean luma and noisy colour finally gets a
    // recommendation instead of being called clean.
    let udif: Vec<f64> = runs.iter().flat_map(|r| r.udif.iter().copied()).collect();
    let vdif: Vec<f64> = runs.iter().flat_map(|r| r.vdif.iter().copied()).collect();
    let avg_chroma_diff = (mean(&udif) + mean(&vdif)) / 2.0;
    let (chroma_ratio, chroma_level) = chroma_noise(avg_chroma_diff, avg_diff);

    let (noise_level, recommendation, trigger) =
        if severity(chroma_level) > severity(luma_level) {
            let rec = match chroma_level {
                NoiseVerdict::Heavy => super::Recommendation::Heavy,
                _ => super::Recommendation::Moderate,
            };
            (chroma_level, rec, NoiseTrigger::Chroma)
        } else {
            (luma_level, recommendation, trigger)
        };

    NoiseAnalysis {
        noise_level,
        recommendation,
        trigger,
        avg_diff,
        std_diff,
        avg_variance,
        luma_level,
        avg_chroma_diff,
        chroma_ratio,
        chroma_level,
        samples_analyzed: diff.len(),
        analyzed: diff.len() >= 3,
    }
}

// ── Color bleeding ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleedLevel {
    None,
    Light,
    Moderate,
    Significant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BleedingAnalysis {
    pub bleed_level: BleedLevel,
    /// Whether to actually recommend the correction — Light does not.
    pub recommendation: bool,
    pub avg_h_spread: f64,
    pub avg_v_spread: f64,
    pub avg_saturation: f64,
    pub bleeding_score: f64,
    pub samples_analyzed: usize,
}

pub fn bleeding(runs: &[SignalStatsRun]) -> BleedingAnalysis {
    // Unlike the other probes, this keeps one scalar per run (last value wins,
    // seeded at chroma-neutral 128) and appends one spread per run — but it
    // collects saturation from *every* frame. That asymmetry is the Python's.
    let mut h_spread = Vec::new();
    let mut v_spread = Vec::new();
    let mut saturation = Vec::new();

    for run in runs {
        let uhigh = SignalStatsRun::last_or(&run.uhigh, 128.0);
        let ulow = SignalStatsRun::last_or(&run.ulow, 128.0);
        let vhigh = SignalStatsRun::last_or(&run.vhigh, 128.0);
        let vlow = SignalStatsRun::last_or(&run.vlow, 128.0);
        saturation.extend(run.satavg.iter().copied());
        h_spread.push(uhigh - ulow);
        v_spread.push(vhigh - vlow);
    }

    let avg_h_spread = mean(&h_spread);
    let avg_v_spread = mean(&v_spread);
    let avg_saturation = mean(&saturation);
    let total_spread = avg_h_spread + avg_v_spread;

    // Wide chroma spread means fuzzy color edges; combined with high
    // saturation the bleeding is actually visible. Low saturation makes it less
    // noticeable even when present, so the score scales by it.
    let bleeding_score = if avg_saturation > 0.0 {
        total_spread * (avg_saturation / 100.0)
    } else {
        total_spread
    };

    let (bleed_level, recommendation) = if bleeding_score > 80.0 || total_spread > 120.0 {
        (BleedLevel::Significant, true)
    } else if bleeding_score > 40.0 || total_spread > 80.0 {
        (BleedLevel::Moderate, true)
    } else if bleeding_score > 20.0 || total_spread > 50.0 {
        // Light bleeding is reported but not recommended for correction.
        (BleedLevel::Light, false)
    } else {
        (BleedLevel::None, false)
    };

    BleedingAnalysis {
        bleed_level,
        recommendation,
        avg_h_spread,
        avg_v_spread,
        avg_saturation,
        bleeding_score,
        samples_analyzed: h_spread.len(),
    }
}
