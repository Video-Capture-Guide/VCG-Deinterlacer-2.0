//! Film cadence, read from the differences between bobbed fields.
//!
//! A home movie that was transferred to tape by filming a projected picture
//! carries no pulldown for `idet` to find: the projector ran at 16, 18 or 24
//! frames a second with no relation to the field rate, so each film frame
//! lands on three or four fields and a frame change mid-exposure leaves a
//! blended field behind. Read as video it is deinterlaced to a stutter; read
//! as 3:2 it is field-matched against frames that never shared a film frame.
//! On a tape with a lot of still shots the repeated fields can even cross the
//! 3:2 bar in `idet` and be mis-read as a telecine.
//!
//! What such a transfer does have is a rhythm. Bob the file to one frame per
//! field and measure how much each field differs from the one before: the
//! fields within a film frame barely differ, and the first field of the next
//! frame differs a lot. Those changes come at a steady period — 3.33 fields
//! for 18 fps on NTSC — and a steady period is what a periodogram finds, no
//! matter how many quiet changes a slow scene hides. The spawning is left to
//! the caller: `field_diffs` parses one window's stderr and `verdict` judges
//! the measured windows, so every threshold here is exercised against real
//! ffmpeg text in `tests/fixtures/` without ffmpeg.

use crate::config::FilmRate;

/// Every `lavfi.signalstats.YDIF` value in one window's stderr, in order.
///
/// Read from a `yadif=mode=1,signalstats,metadata=print` chain, so each
/// value is one field's luma difference from the field before it. The first
/// is always 0 — there is nothing before it — and is kept so that the index
/// of a value is the index of its field.
pub fn field_diffs(stderr: &str) -> Vec<f64> {
    stderr
        .split('\n')
        // Matched as a substring, because ffmpeg can glue a `frame=…`
        // progress line onto the front of a filter line with no newline
        // between.
        .filter(|line| line.contains("signalstats.YDIF="))
        .filter_map(|line| line.rsplit_once('=').and_then(|(_, v)| v.trim().parse().ok()))
        .collect()
}

/// Fewer fields than this and a window says nothing.
const MIN_FIELDS: usize = 40;

/// Fewer picture changes than this and a rhythm cannot be told from chance.
///
/// In a 240-field window at 18 fps there are 72 changes; a slow scene hides
/// many of them below the floor, and twelve still gives a periodogram more
/// than ten cycles to line up over.
const MIN_CHANGES: usize = 12;

/// How well the changes have to keep to one period.
///
/// The coherence of a change train is the length of the sum of its unit
/// phasors over the number of changes: 1.0 when every change lands at exactly
/// the same phase, and for a train with no rhythm at all it stays near
/// `sqrt(1 / changes)`, which at twelve changes is 0.29 and sinks from there.
/// Measured on real transfers, 8 mm home movies sit at 0.77–0.90 and a 3:2
/// telecine at 0.62–0.81, with a blended field at every third change costing
/// a little of each; the busiest ordinary video window reached 0.26.
const MIN_COHERENCE: f64 = 0.6;

/// The periods searched, in fields per film frame.
///
/// Wide enough for 14 fps on NTSC and 12 on PAL at the long end, and for 25
/// fps on PAL at the short end — a PAL 2:2 transfer is a 2.0-field period
/// and has to be found so that it can be told apart from a home movie.
const PERIOD_MIN: f64 = 1.9;
const PERIOD_MAX: f64 = 4.3;
const PERIOD_STEP: f64 = 0.01;

/// One sampled window's rhythm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CadenceWindow {
    /// Fields measured.
    pub fields: usize,
    /// Fields that begin a picture change.
    pub changes: usize,
    /// How well the changes keep to `period`, 0 to 1.
    pub coherence: f64,
    /// Fields per film frame at the best-fitting period.
    pub period: f64,
}

impl CadenceWindow {
    /// Measure one window's field differences.
    ///
    /// `None` when there were too few fields to measure anything — a seek
    /// past the end, or a decode that stopped early.
    pub fn measure(diffs: &[f64]) -> Option<Self> {
        if diffs.len() < MIN_FIELDS {
            return None;
        }

        // The floor is the median: in any film transfer at least six fields
        // in ten repeat the one before them, so the median is a quiet field
        // and the noise it carries. A change is a field well clear of that.
        // The `+ 1.0` keeps a near-black leader, whose median is nearly
        // zero, from reading its own noise as a rhythm.
        let mut sorted = diffs.to_vec();
        sorted.sort_by(f64::total_cmp);
        let median = sorted[sorted.len() / 2];
        let threshold = (2.0 * median).max(median + 1.0);

        // A change is the first field of a run above the floor. A blended
        // field splits one change over two fields; counting only the first
        // keeps that one change rather than two.
        let changes: Vec<usize> = (0..diffs.len())
            .filter(|&k| diffs[k] > threshold && (k == 0 || diffs[k - 1] <= threshold))
            .collect();

        let mut out = Self {
            fields: diffs.len(),
            changes: changes.len(),
            coherence: 0.0,
            period: 0.0,
        };
        if changes.len() < MIN_CHANGES {
            return Some(out);
        }

        // The period whose phase the changes agree on best. Every change is a
        // unit phasor at its own phase for the trial period; at the right
        // period they add up, at a wrong one they cancel over the window.
        let mut period = PERIOD_MIN;
        while period <= PERIOD_MAX + PERIOD_STEP / 2.0 {
            let (re, im) = changes.iter().fold((0.0_f64, 0.0_f64), |(re, im), &k| {
                let angle = std::f64::consts::TAU * k as f64 / period;
                (re + angle.cos(), im + angle.sin())
            });
            let coherence = re.hypot(im) / changes.len() as f64;
            if coherence > out.coherence {
                out.coherence = coherence;
                out.period = period;
            }
            period += PERIOD_STEP;
        }
        Some(out)
    }

    /// Whether the window has a rhythm at all.
    pub fn is_coherent(&self) -> bool {
        self.changes >= MIN_CHANGES && self.coherence >= MIN_COHERENCE
    }

    /// Film frames per second, given the tape's field rate.
    pub fn fps(&self, field_rate: f64) -> f64 {
        if self.period > 0.0 {
            field_rate / self.period
        } else {
            0.0
        }
    }
}

/// What the sampled windows add up to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cadence {
    /// Windows measured, coherent or not.
    pub windows: u32,
    /// Windows with a coherent rhythm at a home-movie projector speed.
    pub film_windows: u32,
    /// Windows with a coherent rhythm at some other speed — a telecine's
    /// 24, a PAL transfer's 25, or video-rate progressive content. Reported
    /// so that the details can say what was seen, but never acted on here:
    /// those are `idet`'s to judge.
    pub other_windows: u32,
    /// Frames per second measured, as the median over the film windows.
    pub fps: f64,
    /// The projector speed found, if the windows agree on one.
    pub rate: Option<FilmRate>,
}

impl Cadence {
    pub fn detected(&self) -> bool {
        self.rate.is_some()
    }
}

/// Decide the verdict from the measured windows.
///
/// Majority rule, as with the telecine verdict, so a long still shot in one
/// window cannot hide a home movie and one busy window cannot invent one.
/// Only 16 and 18 fps are acted on: a coherent 24 fps rhythm is either a
/// telecine, which `idet` sees and inverse telecine handles better, or a sound
/// film projected in front of a camera, which is rare enough to be answered
/// by hand on the Check page rather than guessed at.
pub fn verdict(windows: &[CadenceWindow], field_rate: f64) -> Cadence {
    let mut out = Cadence {
        windows: windows.len() as u32,
        film_windows: 0,
        other_windows: 0,
        fps: 0.0,
        rate: None,
    };

    let mut fps: Vec<f64> = Vec::new();
    let mut votes = [0u32; 2];
    for w in windows.iter().filter(|w| w.is_coherent()) {
        let f = w.fps(field_rate);
        match FilmRate::nearest(f) {
            Some(FilmRate::Fps16) => {
                votes[0] += 1;
                fps.push(f);
            }
            Some(FilmRate::Fps18) => {
                votes[1] += 1;
                fps.push(f);
            }
            _ => out.other_windows += 1,
        }
    }
    out.film_windows = votes[0] + votes[1];
    if !fps.is_empty() {
        fps.sort_by(f64::total_cmp);
        out.fps = fps[fps.len() / 2];
    }

    // At least half the windows, and more than one of them unless there was
    // only one to read: a single window out of five is a scene, not a tape.
    let majority = out.film_windows * 2 >= out.windows
        && (out.film_windows >= 2 || out.windows == 1);
    if out.film_windows == 0 || !majority {
        return out;
    }
    // Ties go to 18: it is what a dual-8 projector runs at on its usual
    // setting, and what both gauges were projected at on every transfer
    // this was calibrated on.
    out.rate = Some(if votes[0] > votes[1] {
        FilmRate::Fps16
    } else {
        FilmRate::Fps18
    });
    out
}
