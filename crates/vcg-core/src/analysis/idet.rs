//! Telecine detection via FFmpeg's `idet` filter.
//!
//! Ported from `_idet_counts`, `_telecine_sample_plan` and `detect_telecine`.
//! The spawning is left to the caller: `IdetCounts::parse` takes one window's
//! stderr and `verdict` takes the parsed windows, so every threshold and the
//! majority rule can be exercised without ffmpeg.

use crate::config::VideoFormat;

/// The two summaries `idet` prints at the end of a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdetCounts {
    /// From "Multi frame detection".
    pub tff: u64,
    pub bff: u64,
    pub prog: u64,
    /// Frames carrying a repeated field (idet's Top + Bottom) — the mechanical
    /// signature of 3:2 pulldown, where four film frames become five video
    /// frames and two repeat a field.
    pub rep: u64,
    /// Neither + Top + Bottom, i.e. every frame the repeated-field pass saw.
    pub rep_total: u64,
}

/// Read `Label:<whitespace><digits>` out of a line, case-insensitively.
fn field(line: &str, label: &str) -> Option<u64> {
    let lower = line.to_ascii_lowercase();
    let at = lower.find(&format!("{}:", label.to_ascii_lowercase()))?;
    let rest = &line[at + label.len() + 1..];
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

impl IdetCounts {
    /// Parse one window's stderr.
    ///
    /// Returns `None` when the window could not be analyzed at all — a seek
    /// past the end of file, a decode error, or a timeout — which the caller
    /// distinguishes from an all-zero result.
    ///
    /// ffmpeg emits these summaries more than once (a zeroed pair during seek
    /// setup, then the real pair), and can glue the second onto the tail of a
    /// `frame=…` progress line with no newline between. Both are handled the
    /// same way the Python is: match the label anywhere in the line, and let
    /// the last occurrence win.
    pub fn parse(stderr: &str) -> Option<Self> {
        let mut c = Self::default();
        let mut found = false;

        for line in stderr.split('\n') {
            if line.contains("Multi frame detection") {
                found = true;
                if let Some(v) = field(line, "TFF") {
                    c.tff = v;
                }
                if let Some(v) = field(line, "BFF") {
                    c.bff = v;
                }
                if let Some(v) = field(line, "Progressive") {
                    c.prog = v;
                }
            } else if line.contains("Repeated Fields") {
                let neither = field(line, "Neither").unwrap_or(0);
                let top = field(line, "Top").unwrap_or(0);
                let bottom = field(line, "Bottom").unwrap_or(0);
                c.rep = top + bottom;
                c.rep_total = neither + top + bottom;
            }
        }

        // Absence of the "Multi frame detection" summary means the run produced
        // nothing usable; a present-but-zeroed summary is a real (empty) result.
        found.then_some(c)
    }

    /// Frames classified by the multi-frame pass.
    pub fn total(&self) -> u64 {
        self.tff + self.bff + self.prog
    }

    fn prog_ratio(&self) -> f64 {
        if self.total() == 0 {
            0.0
        } else {
            self.prog as f64 / self.total() as f64
        }
    }

    fn rep_ratio(&self) -> f64 {
        if self.rep_total == 0 {
            0.0
        } else {
            self.rep as f64 / self.rep_total as f64
        }
    }
}

/// One window to sample: where to start, and how many frames to read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub start_seconds: f64,
    pub frames: u32,
}

/// ~8s NTSC / ~9.6s PAL per window.
const WINDOW_FRAMES: u32 = 240;

/// Decide where in the file to sample for telecine, and how much.
///
/// Films and TV transfers almost always open with a distributor logo or a CGI
/// network bumper rendered at the video frame rate rather than carrying the
/// pulldown of the feature that follows, so reading only the first few seconds
/// reads a film source as video. This skips the head and spreads several short
/// windows across the whole running time.
pub fn sample_plan(duration: f64, fps: f64) -> Vec<Window> {
    let fps = if fps > 1.0 { fps } else { 25.0 };

    // Unknown or very short duration — a single pass from the top is all there
    // is to read.
    if duration <= 0.0 || duration < 30.0 {
        return vec![Window { start_seconds: 0.0, frames: 500 }];
    }

    // Skip the opening logo or leader: 8% of the running time, at least 5s,
    // capped at 60s, and never more than a fifth of a short file.
    let head_skip = 60.0_f64
        .min(5.0_f64.max(duration * 0.08))
        .min(duration * 0.20);

    let fractions: &[f64] = if duration < 120.0 {
        &[0.10, 0.45, 0.78]
    } else if duration < 600.0 {
        &[0.08, 0.26, 0.44, 0.62, 0.80]
    } else {
        &[0.06, 0.20, 0.35, 0.50, 0.65, 0.80, 0.92]
    };

    let window_seconds = WINDOW_FRAMES as f64 / fps;
    let mut plan: Vec<Window> = Vec::new();
    for f in fractions {
        let mut t = head_skip.max(duration * f);
        // Keep the whole window inside the file.
        t = t.min(0.0_f64.max(duration - window_seconds - 1.0));
        // Drop a window that overlaps the previous one.
        if plan
            .last()
            .is_some_and(|last| (t - last.start_seconds).abs() < window_seconds * 0.5)
        {
            continue;
        }
        plan.push(Window { start_seconds: t, frames: WINDOW_FRAMES });
    }

    if plan.is_empty() {
        vec![Window { start_seconds: 0.0, frames: 500 }]
    } else {
        plan
    }
}

/// The pulldown pattern found, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// NTSC 3:2 pulldown — 24fps film carried at 29.97.
    ThreeTwo,
    /// PAL 2:2 — 25fps film carried as interlaced fields.
    TwoTwo,
}

impl Pattern {
    /// The frame rate inverse telecine restores.
    pub fn target_fps(self) -> f64 {
        match self {
            Pattern::ThreeTwo => 24000.0 / 1001.0,
            Pattern::TwoTwo => 25.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// A clear repeated-field ratio is high confidence on its own, because
    /// repeated fields are a mechanical marker rather than a heuristic.
    High,
    Low,
}

/// What the sampled windows add up to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Telecine {
    pub pattern: Option<Pattern>,
    pub confidence: Option<Confidence>,
    /// Already progressive throughout — needs neither IVTC nor QTGMC.
    pub progressive_source: bool,
    /// Windows successfully analyzed.
    pub windows: u32,
    /// Windows that individually read as film.
    pub film_windows: u32,
    pub prog_ratio: f64,
    pub rep_ratio: f64,
    /// Frames idet read as top-field-first and bottom-field-first.
    ///
    /// Kept rather than summed away, because which of the two is larger is
    /// the only measurement of field order there is on a file whose container
    /// does not declare one — and getting field order wrong is the single
    /// worst thing this pipeline can do. QTGMC fed the wrong dominance
    /// interpolates each field against its neighbor in the wrong temporal
    /// order: motion jerks back and forth every frame, edges step, and the
    /// result looks less deinterlaced than the source. Reported from testing
    /// on a MiniDV file, which is exactly the case that hits it — DV NTSC is
    /// bottom-field-first and most DV containers say nothing.
    pub tff_frames: u64,
    pub bff_frames: u64,
}

impl Telecine {
    pub fn detected(&self) -> bool {
        self.pattern.is_some()
    }

    /// Which dominance the picture itself shows, if it is clear.
    ///
    /// `None` when the two counts are close enough that the measurement does
    /// not settle it — a mostly-static shot reads as neither — in which case
    /// the caller should fall back on what the format implies rather than on
    /// a coin toss. The margin is deliberately wide: acting on a 51/49 split
    /// would be worse than not acting at all.
    pub fn measured_field_order(&self) -> Option<bool> {
        let total = self.tff_frames + self.bff_frames;
        if total < 20 {
            return None;
        }
        let tff = self.tff_frames as f64 / total as f64;
        if tff > 0.75 {
            Some(true)
        } else if tff < 0.25 {
            Some(false)
        } else {
            None
        }
    }
}

/// Per-format thresholds for "this window looks like film".
///
/// NTSC 3:2 pulldown produces ~40% progressive frames (2 of every 5). PAL film
/// transfers are 2:2 — both fields of a frame come from the same instant — so
/// genuine film reads as overwhelmingly progressive to idet (80%+). Ordinary
/// interlaced PAL video with static shots easily shows 30-50% "progressive"
/// frames, because idet cannot tell a still frame from a progressive one, so
/// the PAL bar must stay high to avoid false positives on true 50i content.
struct Thresholds {
    window: f64,
    high: f64,
}

impl Thresholds {
    fn for_format(format: VideoFormat) -> Self {
        match format {
            VideoFormat::Ntsc => Thresholds { window: 0.28, high: 0.35 },
            VideoFormat::Pal => Thresholds { window: 0.75, high: 0.85 },
        }
    }
}

/// Repeated-field thresholds, NTSC 3:2 only — a PAL 2:2 transfer repeats no
/// fields at all. Textbook 3:2 sits at 0.40; the bar is well below that for
/// noisy analog captures and still far above the ~0.0 of genuine interlaced
/// video.
const REP_WINDOW: f64 = 0.18;
const REP_STRONG: f64 = 0.25;

/// A window with fewer than this many classified frames is ignored.
const MIN_WINDOW_FRAMES: u64 = 20;

/// Decide the verdict from the successfully analyzed windows.
///
/// Content is what decides. The verdict is the verdict of the majority of
/// windows, so neither a CGI studio logo at the head of the file nor one
/// unusual scene in the middle can swing it.
///
/// The codec a file was captured with is deliberately not consulted: an
/// analog-to-DV converter writes film-sourced telecined material into a
/// DV-codec file just as happily as a camcorder writes native interlaced
/// video, so a `dvvideo` FourCC says nothing about whether the content is film.
pub fn verdict(windows: &[IdetCounts], format: VideoFormat) -> Telecine {
    let th = Thresholds::for_format(format);
    let is_ntsc = format == VideoFormat::Ntsc;

    let mut out = Telecine {
        pattern: None,
        confidence: None,
        progressive_source: false,
        windows: 0,
        film_windows: 0,
        prog_ratio: 0.0,
        rep_ratio: 0.0,
        tff_frames: 0,
        bff_frames: 0,
    };

    let (mut tff, mut bff, mut prog) = (0u64, 0u64, 0u64);
    let (mut rep, mut rep_total) = (0u64, 0u64);

    for w in windows {
        if w.total() < MIN_WINDOW_FRAMES {
            continue;
        }
        out.windows += 1;
        tff += w.tff;
        bff += w.bff;
        prog += w.prog;
        rep += w.rep;
        rep_total += w.rep_total;

        // Either signal alone marks the window as film.
        let by_prog = w.prog > 0 && w.prog_ratio() > th.window;
        let by_rep = is_ntsc && w.rep_ratio() > REP_WINDOW;
        if by_prog || by_rep {
            out.film_windows += 1;
        }
    }

    // Carry the dominance counts out before any of the early returns below.
    // These were summed into locals and left there, so `tff_frames` and
    // `bff_frames` stayed at their initial 0 on every path out of this
    // function: `measured_field_order` saw a total of 0, read that as "not
    // enough frames to tell", and returned `None` every single time. Field
    // order was therefore never actually measured — it always fell through
    // to what the format implies, which is the fallback this measurement
    // exists to avoid. It showed up in the export log as the line
    // "0 TFF / 0 BFF frames measured" on a file idet reads as 241 TFF to 0
    // BFF per window.
    out.tff_frames = tff;
    out.bff_frames = bff;

    let total = tff + bff + prog;
    if total < MIN_WINDOW_FRAMES {
        return out;
    }

    let interlaced = tff + bff;
    out.prog_ratio = prog as f64 / total as f64;
    out.rep_ratio = if rep_total == 0 {
        0.0
    } else {
        rep as f64 / rep_total as f64
    };

    // A file that reads as progressive almost everywhere is not telecined — it
    // is simply already progressive.
    if out.prog_ratio > 0.97 && interlaced as f64 <= (2.0_f64).max(total as f64 * 0.02) {
        out.progressive_source = true;
        return out;
    }

    if interlaced == 0 {
        return out;
    }

    // Majority rule: film if at least half the sampled windows read as film.
    // The aggregate ratio is a second route in, for sources where the
    // per-window reads are borderline but the overall signal is strong.
    let majority = out.film_windows * 2 >= out.windows && out.film_windows >= 1;
    let strong_aggregate = out.film_windows >= 1
        && (out.prog_ratio > th.high || (is_ntsc && out.rep_ratio > REP_STRONG));

    if !(majority || strong_aggregate) {
        return out;
    }

    out.confidence = Some(
        if (is_ntsc && out.rep_ratio > REP_STRONG)
            || (out.prog_ratio > th.high && out.film_windows * 10 >= out.windows * 6)
        {
            Confidence::High
        } else {
            Confidence::Low
        },
    );
    out.pattern = Some(match format {
        VideoFormat::Pal => Pattern::TwoTwo,
        VideoFormat::Ntsc => Pattern::ThreeTwo,
    });
    out
}
