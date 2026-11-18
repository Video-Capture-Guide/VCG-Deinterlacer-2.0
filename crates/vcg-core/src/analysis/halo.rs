//! Edge-halo detection by luma overshoot.
//!
//! Ported from `analyze_halo_level`. Halos from VHS sharpening circuits and
//! capture-card edge enhancement appear as bright ghost bands a few pixels to
//! the side of strong vertical edges. For each sampled frame this scans luma
//! rows horizontally: at every strong edge between two settled areas it
//! compares the pixels just past the bright side, where a halo would sit,
//! against the plateau further out. An edge "has a halo" when the near zone
//! overshoots the plateau by a share of the step. The score is the fraction
//! of such edges that overshoot.
//!
//! Rewritten 17 Nov 2026: it used to read 4-7 pixels from wherever the scan
//! first met an edge and demand a dark dip too. That counted leaves and twigs
//! and missed the real rim, so no dehalo setting ever changed its verdict.
//!
//! Horizontal-only scanning is deliberate: analog sharpening operates along the
//! scanline, and row-wise reads are immune to interlacing comb.
//!
//! This is the one probe that is pure pixel work rather than text parsing, so
//! it takes a luma plane directly. The Python needs Pillow and gives up
//! entirely when it is missing (`if not HAS_PIL: return unanalyzed`); here the
//! scan is plain indexing with no image library involved.

/// Minimum luma gradient to count as a strong edge.
const EDGE_THRESH: i32 = 40;

/// The two settled sides must differ by at least this much to be an edge
/// that can carry a halo.
const MIN_STEP: f64 = 50.0;

/// A rim this share of the step, or more, is a halo...
const RIM_SHARE: f64 = 0.05;

/// ...and never less than this, so grain on a soft edge does not count.
const MIN_RIM: f64 = 6.0;

/// A far zone varying by more than this is busy texture, not a settled plateau.
const PLATEAU_FLATNESS: i32 = 12;

/// How far past the first sight of an edge to look for its steepest point.
const EDGE_SEEK: usize = 6;

/// Where the halo sits, counted from the middle of the edge: just past the
/// edge's own slope. Tape and camera sharpening puts its rim here — measured
/// on a Video8 and a MiniDV tape, the brightest pixel was 2-4 out.
const NEAR: std::ops::RangeInclusive<isize> = 2..=5;

/// The settled level the rim is compared with.
const FAR: std::ops::RangeInclusive<isize> = 8..=11;

/// A single-channel 8-bit image.
#[derive(Debug, Clone, Copy)]
pub struct LumaPlane<'a> {
    pub data: &'a [u8],
    pub width: usize,
    pub height: usize,
}

impl LumaPlane<'_> {
    fn at(&self, x: usize, y: usize) -> i32 {
        self.data[y * self.width + x] as i32
    }
}

/// Running tally across all sampled frames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeTally {
    pub total_edges: u64,
    pub halo_edges: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaloLevel {
    Clean,
    Light,
    Moderate,
    Strong,
    /// Could not be judged at all.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HaloAnalysis {
    pub analyzed: bool,
    pub halo_ratio: f64,
    pub edges_analyzed: u64,
    pub halo_level: HaloLevel,
    pub recommendation: super::Recommendation,
}

/// Classify the pixel column at (x, y).
///
/// Returns `None` when there is no strong edge here, or either side's far zone
/// is not a settled plateau — busy texture is indeterminate and must not vote,
/// or detailed scenes all read as haloed.
///
/// Also `None` when the two sides are about the same level: a twig or a gap
/// between leaves, not a step a halo can sit on.
///
/// Returns `Some(true)` when the bright side rims above its own plateau.
fn probe(img: &LumaPlane, x: usize, y: usize) -> Option<bool> {
    let grad = img.at(x + 2, y) - img.at(x - 2, y);
    if grad.abs() < EDGE_THRESH {
        return None;
    }
    // The scan meets an edge at its leading foot, a few pixels before the
    // middle of it. Measure from the steepest point instead, or the zones
    // land in front of the edge on one side and inside it on the other.
    let steep = |c: usize| (img.at(c + 1, y) - img.at(c - 1, y)).abs();
    let x = (x..=x + EDGE_SEEK).max_by_key(|&c| (steep(c), std::cmp::Reverse(c))).unwrap();
    let (bright_dir, dark_dir): (isize, isize) = if grad > 0 { (1, -1) } else { (-1, 1) };

    let sample = |dir: isize, offset: isize| -> i32 {
        img.at((x as isize + dir * offset) as usize, y)
    };

    let b_near = NEAR.map(|o| sample(bright_dir, o)).max().unwrap();
    let b_far: Vec<i32> = FAR.map(|o| sample(bright_dir, o)).collect();
    let d_far: Vec<i32> = FAR.map(|o| sample(dark_dir, o)).collect();

    let spread = |v: &[i32]| v.iter().max().unwrap() - v.iter().min().unwrap();
    if spread(&b_far) > PLATEAU_FLATNESS || spread(&d_far) > PLATEAU_FLATNESS {
        return None;
    }

    let mean = |v: &[i32]| v.iter().sum::<i32>() as f64 / 4.0;
    // A halo is a rim on a step between two settled areas. A thin twig or a
    // gap between leaves also rises and falls either side of an "edge", but
    // the levels on its two sides are about the same — that is texture, which
    // dehalo is built to leave alone, and it must not vote.
    let step = mean(&b_far) - mean(&d_far);
    if step < MIN_STEP {
        return None;
    }
    // The rim is judged against the size of the step, because sharpening is
    // proportional: a dark post on a bright path rings far more than a
    // grey-on-grey edge, and both are the same halo.
    //
    // Only the bright rim is asked for. That is the white outline people
    // see, and the one both dehalo settings remove; on the Video8 and MiniDV
    // tapes it was sampled from, the dark side had no matching dip at all.
    let rim = (step * RIM_SHARE).max(MIN_RIM);
    Some(b_near as f64 - mean(&b_far) >= rim)
}

/// Scan one frame, adding to the tally.
///
/// Reads the central 75% of rows, every 4th row. An edge only counts if it
/// persists two rows down: real halos are continuous lines that track their
/// edge, while overshoot from random texture (foliage, straw, fabric) is
/// incoherent.
pub fn scan_frame(img: &LumaPlane, tally: &mut EdgeTally) {
    // The probe reads up to 14 pixels either side, so the frame must be big
    // enough for the scan window to stay in bounds.
    if img.width < 64 || img.height < 64 {
        return;
    }

    let mut y = img.height / 8;
    while y < img.height * 7 / 8 {
        let mut x = 16usize;
        // The probe seeks up to EDGE_SEEK right, then reads FAR beyond that.
        while x < img.width - 20 {
            let Some(here) = probe(img, x, y) else {
                x += 2;
                continue;
            };
            // Look for the same edge two rows down, allowing a little drift.
            let mut below = None;
            for dx in [0isize, -1, 1, -2, 2] {
                below = probe(img, (x as isize + dx) as usize, y + 2);
                if below.is_some() {
                    break;
                }
            }
            if let Some(below) = below {
                tally.total_edges += 1;
                if here && below {
                    tally.halo_edges += 1;
                }
            }
            x += 10; // skip past this edge
        }
        y += 4;
    }
}

/// Below this many strong edges the content is too flat or dark to judge.
const MIN_EDGES: u64 = 50;

/// Turn the tally into a verdict.
///
/// Thresholds are experimental. With the probe as it is now (rim 2-5 past
/// the steepest point, on a step between settled sides) the sample captures
/// measured: 8 mm film transfers 0.6-1.9%, a DVD 0.9%, an HD camcorder 4.2%,
/// a telecined DVD 12%, a HuffYUV capture 16%, MiniDV NTSC and PAL 18%, and
/// a Video8 tape kept as the halo test 35%. After the app's Strong dehalo
/// the MiniDV render fell from 22% to 16% and the Video8 from 28% to 25%;
/// what is left there is a glow 6-8 pixels wide, beyond either setting.
pub fn verdict(tally: EdgeTally) -> HaloAnalysis {
    if tally.total_edges < MIN_EDGES {
        return HaloAnalysis {
            // Still reports as analyzed if *any* edge was seen, matching the
            // Python's `analyzed: total_edges > 0`.
            analyzed: tally.total_edges > 0,
            halo_ratio: 0.0,
            edges_analyzed: tally.total_edges,
            halo_level: HaloLevel::Clean,
            recommendation: super::Recommendation::None,
        };
    }

    let halo_ratio = tally.halo_edges as f64 / tally.total_edges as f64;
    let (halo_level, recommendation) = if halo_ratio > 0.30 {
        (HaloLevel::Strong, super::Recommendation::Strong)
    } else if halo_ratio > 0.18 {
        (HaloLevel::Moderate, super::Recommendation::Light)
    } else if halo_ratio > 0.10 {
        (HaloLevel::Light, super::Recommendation::Light)
    } else {
        (HaloLevel::Clean, super::Recommendation::None)
    };

    HaloAnalysis {
        analyzed: true,
        halo_ratio,
        edges_analyzed: tally.total_edges,
        halo_level,
        recommendation,
    }
}
