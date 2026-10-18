//! Edge-halo detection by luma overshoot.
//!
//! Ported from `analyze_halo_level`. Halos from VHS sharpening circuits and
//! capture-card edge enhancement appear as bright ghost bands a few pixels to
//! the side of strong vertical edges. For each sampled frame this scans luma
//! rows horizontally: at every strong edge it compares the pixels just past the
//! bright side, where a halo would sit, against the plateau further out. An
//! edge "has a halo" when the near zone overshoots the plateau. The score is
//! the fraction of strong edges that overshoot.
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

/// Near-zone excess over the plateau that counts as a halo.
const OVERSHOOT_THRESH: f64 = 14.0;

/// A far zone varying by more than this is busy texture, not a settled plateau.
const PLATEAU_FLATNESS: i32 = 20;

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
/// Returns `Some(true)` only for the two-sided sharpening signature:
/// bright-side overshoot AND dark-side undershoot. Content that merely has a
/// bright line near an edge (straw, blinds, text) rings on one side only.
fn probe(img: &LumaPlane, x: usize, y: usize) -> Option<bool> {
    let grad = img.at(x + 2, y) - img.at(x - 2, y);
    if grad.abs() < EDGE_THRESH {
        return None;
    }
    let (bright_dir, dark_dir): (isize, isize) = if grad > 0 { (1, -1) } else { (-1, 1) };

    let sample = |dir: isize, offset: isize| -> i32 {
        img.at((x as isize + dir * offset) as usize, y)
    };

    let b_near = (4..=7).map(|o| sample(bright_dir, o)).max().unwrap();
    let b_far: Vec<i32> = (11..=14).map(|o| sample(bright_dir, o)).collect();
    let d_near = (4..=7).map(|o| sample(dark_dir, o)).min().unwrap();
    let d_far: Vec<i32> = (11..=14).map(|o| sample(dark_dir, o)).collect();

    let spread = |v: &[i32]| v.iter().max().unwrap() - v.iter().min().unwrap();
    if spread(&b_far) > PLATEAU_FLATNESS || spread(&d_far) > PLATEAU_FLATNESS {
        return None;
    }

    let mean = |v: &[i32]| v.iter().sum::<i32>() as f64 / 4.0;
    let overshoot = b_near as f64 - mean(&b_far) >= OVERSHOOT_THRESH;
    let undershoot = mean(&d_far) - d_near as f64 >= OVERSHOOT_THRESH * 0.7;
    Some(overshoot && undershoot)
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
        while x < img.width - 16 {
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
/// Thresholds are experimental, calibrated against sample captures: a clean
/// modern camcorder file measured ~8%, a real VHS capture ~13%, consumer DV
/// with in-camera sharpening ~17%, and the same camcorder file artificially
/// over-sharpened (unsharp la=1.8) ~21%.
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
