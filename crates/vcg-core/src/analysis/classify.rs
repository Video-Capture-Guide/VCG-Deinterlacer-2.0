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
/// stays low.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseTrigger {
    Tout,
    Ydif,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseAnalysis {
    pub noise_level: NoiseVerdict,
    /// The filter strength to recommend. Note Light recommends Moderate.
    pub recommendation: super::Recommendation,
    pub trigger: NoiseTrigger,
    pub avg_diff: f64,
    pub std_diff: f64,
    pub avg_variance: f64,
    pub samples_analyzed: usize,
    /// At least three good samples were collected.
    pub analyzed: bool,
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
    let (noise_level, recommendation, trigger) = if avg_variance > 0.08 || avg_diff > 30.0 {
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

    NoiseAnalysis {
        noise_level,
        recommendation,
        trigger,
        avg_diff,
        std_diff,
        avg_variance,
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
