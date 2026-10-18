//! Detecting how much of the frame edge is not picture.
//!
//! This has no counterpart in the Python, which only ever offers fixed presets:
//! BT.601 crops 8 columns from each side whatever the file looks like. A real
//! capture card does not oblige. The horizontal blanking it digitises lands
//! wherever the timing of that particular card, tape and deck put it, so the
//! black margin is routinely wider on one side than the other — and on a
//! measured capture here it is 14 columns on both sides, so the 8-column preset
//! leaves blanking inside the picture.
//!
//! Three different things sit at the edge of a tape capture and each needs its
//! own test:
//!
//! 1. **Blanking.** Flat and near-black. Brightness alone cannot find it,
//!    because a night scene is also near-black: the distinguishing property is
//!    that blanking carries no *detail*. So a line counts as blanking only when
//!    it is both dark relative to the picture and flat relative to the picture.
//! 2. **The analog edge ramp.** Past the blanking the picture does not begin
//!    abruptly; it ramps up over a few columns of dim, smeared video. A
//!    brightness threshold run across this eats into genuinely dark content —
//!    it did, by six columns, on a test clip. What actually separates a ramp
//!    from dark content is that a ramp *climbs steadily inward*, so the scan
//!    extends only while each line is meaningfully brighter than the last, and
//!    for at most a few lines.
//! 3. **Head-switching noise.** The torn band in the last few lines of a VHS
//!    frame. It is not dark, so neither test above sees it.
//!
//!    The first version of this looked for high *horizontal* detail, modelled
//!    on snow. That was wrong about what the band is. On a real capture it is
//!    a band of smeared horizontal streaks with the color torn out of it:
//!    horizontally it is *smooth*, and it measured 1.13-1.33x the picture's
//!    horizontal detail against a 2.0x threshold — it could never have fired.
//!    Measured on the same rows, the signals that do separate it are how
//!    unlike the row above each line is, in luma (2.1-6.5x) and especially in
//!    chroma (3.1-9.1x): the band is torn out of sequence with the picture and
//!    its chroma phase is wrecked. Horizontal detail is kept as a third
//!    trigger, because a badly worn tape does also produce actual snow.
//!
//! Columns are measured over the vertical middle of the frame and rows over the
//! horizontal middle. That is not tidiness: a head-switching band spans the
//! full width, and letting it into the column statistics made black bars read
//! as a 99th-percentile brightness of 140.
//!
//! Everything here is a *recommendation*. The caller decides whether to apply
//! it, which is the whole point — a user who disagrees can see the numbers and
//! override them.

use super::halo::LumaPlane;

/// The three planes of one frame, all at full resolution.
///
/// Chroma is needed at luma resolution (so, 4:4:4 from the decoder) because
/// the head-switching test compares a chroma line against the line above it,
/// and subsampled chroma would merge the two.
#[derive(Debug, Clone, Copy)]
pub struct FramePlanes<'a> {
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub width: usize,
    pub height: usize,
}

impl<'a> FramePlanes<'a> {
    fn luma(&self) -> LumaPlane<'a> {
        LumaPlane {
            data: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

/// Fraction of each dimension ignored at both ends when measuring the opposite
/// axis, so the two scans cannot contaminate each other.
const INSET: f64 = 0.12;

/// A blanking line is at most this fraction of the picture's brightness.
const BLANK_LEVEL: f64 = 0.35;

/// A blanking line carries at most this fraction of the picture's detail.
const BLANK_DETAIL: f64 = 0.45;

/// …but never less than this in absolute terms, on the 0-255 scale.
///
/// `BLANK_DETAIL` is a fraction of the picture's own detail, which assumes
/// blanking is *flat*. On one side of a real capture it often is not: the
/// left edge of an analog line carries residual noise out of the horizontal
/// blanking interval, so it is dark but grainy, while the right edge is
/// clean. Measured on an ATI All-In-Wonder capture of a Hi8 tape, the left
/// blanking runs 16 columns at brightness 22-36 with a vertical detail of
/// 2.0-3.3, and the right blanking of the same frame sits at 0.3-1.8.
///
/// That picture is soft and dark — its own detail measures 4.3 — so the
/// relative bar came out at 0.45 x 4.3 = 1.93 and the left blanking blew
/// through it at the fourth column. The scan stopped there and reported 6
/// columns against the 18 that are actually blanking, while the right side,
/// being quieter, measured correctly. The result on screen was a crop that
/// was visibly right on one side and wrong on the other.
///
/// This is also why it struck some videos and not others: the bar is a
/// fraction of the picture's detail, so a soft or dark source drags it down
/// until ordinary blanking grain clears it, while a crisp source holds it
/// high enough to stay out of trouble.
///
/// A column this dark — the level test still has to pass — carrying under
/// four levels of variation is blanking grain, not content. Brightness is
/// what separates blanking from picture; this test only has to separate flat
/// black from a dark *scene*, and a dark scene has rather more structure
/// than this.
const BLANK_DETAIL_FLOOR: f64 = 4.0;

/// The edge ramp is only followed while below this fraction of the picture.
const KNEE: f64 = 0.75;

/// Each ramp line must be brighter than the last by this fraction of the
/// picture's brightness. Without it the scan cannot tell a ramp from dark
/// content.
const RISE: f64 = 0.08;

/// A genuine analog edge ramp is a few lines wide, never more.
const RAMP_MAX: u32 = 6;

/// Below this the frame is too dark to judge and the preset is left alone.
const MIN_PLATEAU: f64 = 60.0;

/// A head-switching line is at least this many times the picture's typical
/// break from one row to the next.
const NOISE_RATIO: f64 = 2.0;

/// A single row over the threshold is a hard horizontal edge in the picture —
/// a table edge, a skirting board. Head switching is always several lines.
const NOISE_MIN_ROWS: u32 = 2;

/// A band row must also break from the row above by this much in absolute
/// terms, on the 0-255 scale.
///
/// The ratio alone is not enough. Over a picture whose chroma barely changes
/// from line to line the median break is a fraction of a level, so twice it is
/// still a fraction of a level and the faintest variation clears the bar — on
/// a test frame that pulled in the picture row directly above the band. A row
/// that differs from the one above it by less than a few levels has not been
/// torn out of sequence with anything.
///
/// This is the floor for the luma and detail triggers. Chroma gets its own,
/// below, because the two live on very different scales.
const NOISE_MIN_ABS: f64 = 4.0;

/// The same floor, for the chroma trigger.
///
/// Chroma breaks are far smaller than luma breaks over ordinary picture — on a
/// measured NTSC capture the median row-to-row break is 6.6 in luma and 0.99
/// in chroma, nearly seven to one. One shared floor of 4.0 therefore means
/// something quite different to each trigger: a little over half the picture
/// median in luma, but four times it in chroma. The chroma trigger is the
/// *best* of the three — a head-switching dropout wrecks the chroma phase, and
/// that is the signal that separates the band from a dark scene or a hard
/// horizontal edge — and the shared floor was disqualifying almost all of it.
///
/// Measured on the capture that prompted this: through the band the chroma
/// break runs 2.1 to 5.1 against a picture median of 0.99, so every row of it
/// clears the 2x ratio, but only two scattered rows reach 4.0. They were not
/// adjacent, `NOISE_MIN_ROWS` needs two in a row, and the whole band went
/// unreported — the bottom came back as no crop at all on a tape with a
/// visible torn band. Rows 472-478 clear a floor of 2.0 continuously, which
/// is the band as the eye sees it.
///
/// This is why the band is found on some captures and not others: one whose
/// band tears luma hard enough to clear the luma trigger on consecutive rows
/// was caught, and one whose band shows up mainly as chroma damage was not.
const NOISE_MIN_ABS_CHROMA: f64 = 2.0;

/// How many rows inside a band may fail the test before the band is over.
///
/// A head-switching band is not uniformly torn. Its rows vary, and one in the
/// middle can land just under the threshold while the rows above and below it
/// are plainly band — on the capture this was measured against, the band runs
/// from row 472 to row 478 and row 477 alone reads as picture, by a hair
/// (chroma break 4.29 against a bar of 4.66).
///
/// The scan used to stop at the first such row, which threw away five of the
/// seven rows and left `run` at 1 — below `NOISE_MIN_ROWS` — so the band was
/// reported as absent and the bottom was not cropped at all.
///
/// One row, not more: that is enough for a ragged band and too few to walk up
/// into the picture. A gap row is cropped along with the band either way,
/// since there is no way to keep row 477 while removing 476 and 478.
const NOISE_GAP: u32 = 1;

/// The noise band is bounded to this fraction of the height.
const NOISE_FRACTION: f64 = 0.05;

/// How many rows at the very bottom may be neither blanking nor band before
/// the search gives up.
///
/// A capture does not always put the band flush against the last row, and the
/// scan's own starting point can be one row short of it — see `detect`. Two
/// is enough for every real case and small enough that a hard horizontal edge
/// two rows up cannot be mistaken for the start of a band.
const NOISE_SKIP: u32 = 2;

/// No side is ever cropped by more than this fraction of its dimension.
const MAX_FRACTION: f64 = 0.12;

/// A side must show at least this much blanking to be reported at all, so a
/// single flickering line does not become a crop.
const MIN_SIDE: u32 = 2;

/// Percentile used as a line's brightness. A mean is pulled down by dark
/// content, where what matters is whether the line ever carries picture.
const LEVEL_PERCENTILE: f64 = 0.99;

/// Running tally over the sampled frames.
///
/// Brightness is accumulated as a 256-bin histogram per line rather than by
/// keeping the frames, so the percentile is exact and the cost does not grow
/// with the number of frames sampled.
#[derive(Debug, Clone)]
pub struct EdgeTally {
    pub width: usize,
    pub height: usize,
    frames: u32,
    col_hist: Vec<u32>,
    row_hist: Vec<u32>,
    col_detail: Vec<u64>,
    row_detail: Vec<u64>,
    col_detail_n: u64,
    row_detail_n: u64,
    /// How unlike the row above each row is, in luma and in chroma. These are
    /// what actually find the head-switching band; see the module note.
    row_break: Vec<u64>,
    row_chroma_break: Vec<u64>,
}

/// The inclusive-exclusive band of one axis used when measuring the other.
fn inset(n: usize) -> (usize, usize) {
    let lo = (n as f64 * INSET) as usize;
    let hi = n - lo;
    if hi > lo + 1 {
        (lo, hi)
    } else {
        (0, n)
    }
}

impl EdgeTally {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            frames: 0,
            col_hist: vec![0; width * 256],
            row_hist: vec![0; height * 256],
            col_detail: vec![0; width],
            row_detail: vec![0; height],
            col_detail_n: 0,
            row_detail_n: 0,
            row_break: vec![0; height],
            row_chroma_break: vec![0; height],
        }
    }

    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Fold one frame in. Frames of the wrong size are ignored rather than
    /// panicking: a corrupt capture can hand back a short decode.
    pub fn add_frame(&mut self, frame: FramePlanes) {
        let n = self.width * self.height;
        if frame.width != self.width
            || frame.height != self.height
            || frame.y.len() < n
            || frame.u.len() < n
            || frame.v.len() < n
        {
            return;
        }
        let plane = frame.luma();
        let (y0, y1) = inset(self.height);
        let (x0, x1) = inset(self.width);

        // Columns, over the vertical middle only.
        for y in y0..y1 {
            let row = &plane.data[y * self.width..y * self.width + self.width];
            for (x, &v) in row.iter().enumerate() {
                self.col_hist[x * 256 + v as usize] += 1;
            }
            if y > y0 {
                let above = &plane.data[(y - 1) * self.width..(y - 1) * self.width + self.width];
                for x in 0..self.width {
                    self.col_detail[x] += row[x].abs_diff(above[x]) as u64;
                }
            }
        }
        self.col_detail_n += (y1 - y0 - 1) as u64;

        // Rows, over the horizontal middle only.
        for y in 0..self.height {
            let row = &plane.data[y * self.width..y * self.width + self.width];
            for &v in &row[x0..x1] {
                self.row_hist[y * 256 + v as usize] += 1;
            }
            let mut sum = 0u64;
            for x in x0 + 1..x1 {
                sum += row[x].abs_diff(row[x - 1]) as u64;
            }
            self.row_detail[y] += sum;

            // How unlike the row above. Row zero has nothing above it, so it
            // keeps a break of zero and can never be called a band — which is
            // right, since head switching is a bottom-of-frame artefact.
            if y > 0 {
                let base = y * self.width;
                let up = base - self.width;
                let mut luma = 0u64;
                let mut chroma = 0u64;
                for x in x0..x1 {
                    luma += frame.y[base + x].abs_diff(frame.y[up + x]) as u64;
                    chroma += frame.u[base + x].abs_diff(frame.u[up + x]) as u64;
                    chroma += frame.v[base + x].abs_diff(frame.v[up + x]) as u64;
                }
                self.row_break[y] += luma;
                self.row_chroma_break[y] += chroma;
            }
        }
        self.row_detail_n += (x1 - x0 - 1) as u64;

        self.frames += 1;
    }

    fn level(hist: &[u32], i: usize) -> f64 {
        let bins = &hist[i * 256..i * 256 + 256];
        let total: u32 = bins.iter().sum();
        if total == 0 {
            return 0.0;
        }
        let want = (total as f64 * LEVEL_PERCENTILE).ceil() as u32;
        let mut seen = 0u32;
        for (v, &n) in bins.iter().enumerate() {
            seen += n;
            if seen >= want {
                return v as f64;
            }
        }
        255.0
    }

    fn col_levels(&self) -> Vec<f64> {
        (0..self.width).map(|x| Self::level(&self.col_hist, x)).collect()
    }

    fn row_levels(&self) -> Vec<f64> {
        (0..self.height).map(|y| Self::level(&self.row_hist, y)).collect()
    }

    fn col_details(&self) -> Vec<f64> {
        let n = (self.col_detail_n.max(1)) as f64;
        self.col_detail.iter().map(|&s| s as f64 / n).collect()
    }

    fn row_details(&self) -> Vec<f64> {
        let n = (self.row_detail_n.max(1)) as f64;
        self.row_detail.iter().map(|&s| s as f64 / n).collect()
    }

    /// Per-row luma break from the row above. The divisor counts the same
    /// samples as `row_detail`, give or take the one pixel the horizontal
    /// difference loses, which is close enough for a ratio against the
    /// picture's own median.
    fn row_breaks(&self) -> Vec<f64> {
        let n = (self.row_detail_n.max(1)) as f64;
        self.row_break.iter().map(|&s| s as f64 / n).collect()
    }

    /// Per-row chroma break, summed over U and V.
    fn row_chroma_breaks(&self) -> Vec<f64> {
        let n = (self.row_detail_n.max(1)) as f64;
        self.row_chroma_break.iter().map(|&s| s as f64 / n).collect()
    }
}

/// What the detector concluded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Detection {
    /// Nothing could be measured; the caller should keep whatever preset is
    /// already selected. The string says why, for the Details panel.
    Inconclusive(&'static str),
    /// A measured recommendation, possibly all zeroes, meaning the frame is
    /// already full-picture.
    Found(EdgeCrop),
}

/// A measured edge crop, in source pixels per side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EdgeCrop {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
    /// How many of `bottom` are head-switching noise rather than blanking.
    /// Worth saying out loud, because it is the one finding a user would not
    /// guess from looking at a still.
    pub noise_rows: u32,
}

/// Extra pixels taken off each edge that has something on it.
///
/// The measurement finds where the junk stops, which is exactly the wrong
/// place to cut: the boundary is a soft ramp on the sides and a ragged tear at
/// the bottom, both of which wander by a line or two between frames. Cutting
/// one pixel past it is the difference between a clean edge and a flickering
/// one. Two rather than one because every crop has to be even for subsampled
/// chroma, so one would be rounded up to two anyway.
pub const SAFETY_MARGIN: u32 = 2;

impl EdgeCrop {
    /// The same crop with a safety margin on every edge that found something.
    ///
    /// An edge that measured zero is left at zero — there is nothing there to
    /// overshoot, and cutting into a clean frame for no reason is worse than
    /// the ragged pixel this guards against.
    pub fn with_margin(&self, margin: u32) -> EdgeCrop {
        let grow = |v: u32| if v == 0 { 0 } else { v + margin };
        EdgeCrop {
            left: grow(self.left),
            right: grow(self.right),
            top: grow(self.top),
            bottom: grow(self.bottom),
            // The band grows with the bottom crop it lives inside, so the two
            // stay consistent when the screen says how much of the bottom is
            // band.
            noise_rows: grow(self.noise_rows),
        }
    }

    pub fn is_nothing(&self) -> bool {
        self.left == 0 && self.right == 0 && self.top == 0 && self.bottom == 0
    }

    /// Whether the sides differ, which is the case the fixed presets cannot
    /// express at all.
    pub fn is_lopsided(&self) -> bool {
        self.left != self.right
    }
}

/// Median of a slice, used for the picture's own plateau and detail level.
fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// Lines to drop from the start of `level`, scanning inward.
///
/// Returns the blanking run and the ramp that follows it separately, because
/// they are reported differently: blanking is "black bars", the ramp is "a soft
/// edge".
fn scan(level: &[f64], detail: &[f64], plateau: f64, pic_detail: f64) -> (u32, u32) {
    let half = level.len() / 2;
    let flat = (pic_detail * BLANK_DETAIL).max(BLANK_DETAIL_FLOOR);
    let mut k = 0usize;
    while k < half && level[k] < plateau * BLANK_LEVEL && detail[k] < flat {
        k += 1;
    }
    let blank = k as u32;

    let mut ramp = 0u32;
    while ramp < RAMP_MAX
        && k + 1 < half
        && level[k] < plateau * KNEE
        && level[k + 1] - level[k] >= plateau * RISE
    {
        k += 1;
        ramp += 1;
    }
    (blank, ramp)
}

/// One side's total, after the minimum-run rule.
fn side(blank: u32, ramp: u32) -> u32 {
    if blank < MIN_SIDE {
        0
    } else {
        blank + ramp
    }
}

/// Round outward to an even number of lines.
///
/// The horizontal crop is applied before deinterlacing, on subsampled chroma,
/// so it must be even; the vertical crop is deferred until the clip is
/// progressive, where even keeps the height legal for 4:2:0.
fn even(v: u32) -> u32 {
    v + (v % 2)
}

/// Measure the edges and recommend a crop.
pub fn detect(tally: &EdgeTally) -> Detection {
    if tally.frames == 0 {
        return Detection::Inconclusive("no frames could be read");
    }
    if tally.width < 16 || tally.height < 16 {
        return Detection::Inconclusive("the frame is too small to measure");
    }

    let col_level = tally.col_levels();
    let row_level = tally.row_levels();
    let col_detail = tally.col_details();
    let row_detail = tally.row_details();

    let (x0, x1) = inset(tally.width);
    let (y0, y1) = inset(tally.height);
    let plateau = median(&col_level[x0..x1]);
    let pic_col_detail = median(&col_detail[x0..x1]);
    let pic_row_detail = median(&row_detail[y0..y1]);

    if plateau < MIN_PLATEAU {
        return Detection::Inconclusive("the picture is too dark to measure reliably");
    }

    let rev = |v: &[f64]| -> Vec<f64> { v.iter().rev().copied().collect() };
    let (lb, lr) = scan(&col_level, &col_detail, plateau, pic_col_detail);
    let (rb, rr) = scan(&rev(&col_level), &rev(&col_detail), plateau, pic_col_detail);
    let (tb, tr) = scan(&row_level, &row_detail, plateau, pic_row_detail);
    let (bb, br) = scan(&rev(&row_level), &rev(&row_detail), plateau, pic_row_detail);

    let mut crop = EdgeCrop {
        left: side(lb, lr),
        right: side(rb, rr),
        top: side(tb, tr),
        bottom: side(bb, br),
        noise_rows: 0,
    };

    // Head-switching noise, searched upward from above any blanking already
    // found: on a tape with both, the band sits inside the black.
    //
    // Three triggers, any one of which is enough. The band tears the picture
    // out of sequence, so a line of it does not follow the line above — in
    // luma, and much more strongly in chroma, whose phase the dropout wrecks.
    // Horizontal detail stays as a third trigger for the worn tapes that
    // really do produce snow.
    let row_break = tally.row_breaks();
    let row_chroma = tally.row_chroma_breaks();
    let (iy0, iy1) = (y0, y1);
    let pic_break = median(&row_break[iy0..iy1]);
    let pic_chroma = median(&row_chroma[iy0..iy1]);

    // Each trigger has to clear both the ratio against this picture and an
    // absolute floor, so a very smooth picture cannot make the ratio
    // meaningless. The floor is per-signal: see `NOISE_MIN_ABS_CHROMA`.
    let over = |value: f64, reference: f64, floor: f64| -> bool {
        value > reference * NOISE_RATIO && value > floor
    };
    let is_band = |y: usize| -> bool {
        over(row_break[y], pic_break, NOISE_MIN_ABS)
            || over(row_chroma[y], pic_chroma, NOISE_MIN_ABS_CHROMA)
            || over(row_detail[y], pic_row_detail, NOISE_MIN_ABS)
    };

    // The band is not reliably flush with the last row, and the scan used to
    // assume it was: it started at `crop.bottom` and stopped at the first row
    // that was not band, so a single unlike row underneath hid the whole
    // thing. Two ways that happens on a real capture, both reported from
    // testing on a Video8 tape whose band was obvious on screen and which
    // produced a side-only crop:
    //
    // * **One row of blanking below the band.** `side()` discards a blank run
    //   shorter than `MIN_SIDE`, so `crop.bottom` comes back 0, the scan looks
    //   at the black row, and breaks. The run was measured — it is just not
    //   reportable on its own — so start above it rather than above zero.
    // * **A transitional row.** Neither black enough to be blanking nor torn
    //   enough to be band. Allowing a couple of those before giving up costs
    //   nothing: a picture row only ever sits under a *band*, and if no band
    //   is found the rows are not cropped at all.
    //
    // Anything skipped on the way up is cropped with the band, because
    // removing a torn band and leaving the rows beneath it is not a crop
    // anybody wants.
    let start = (crop.bottom as usize).max(bb as usize);
    let limit = (tally.height as f64 * NOISE_FRACTION) as u32;

    // Band rows found, and the distance from `start` to the top of them.
    let mut noise = 0u32;
    let mut reach = 0u32;
    let mut run = 0u32;
    let mut gap = 0u32;
    let mut offset = 0u32;
    while offset < limit {
        let from_bottom = start + offset as usize;
        if from_bottom + 1 >= tally.height {
            break;
        }
        let y = tally.height - 1 - from_bottom;
        if is_band(y) {
            gap = 0;
            run += 1;
            // One row over the line is a hard horizontal edge in the picture —
            // a table edge, a skirting board. Head switching is always several
            // lines, so nothing is committed until the run clears the minimum.
            if run >= NOISE_MIN_ROWS {
                noise = run;
                reach = offset + 1;
            }
        } else if run > 0 && gap < NOISE_GAP {
            // A row inside the band that did not quite clear the bar. Carry on
            // up, counting it as part of the run, because it is inside the
            // band and will be cropped with it. `noise` and `reach` are not
            // advanced here, so a gap at the very top of the band is not
            // mistaken for another row of it.
            gap += 1;
            run += 1;
        } else if run > 0 {
            // Two in a row: the band really has ended. Above it is picture.
            break;
        } else if offset >= NOISE_SKIP {
            // Far enough from the bottom that anything found now would not be
            // a head-switching band.
            break;
        }
        offset += 1;
    }

    if noise >= NOISE_MIN_ROWS {
        crop.noise_rows = noise;
        crop.bottom = start as u32 + reach;
    }

    // Caps, then even rounding. The cap is a backstop against a pathological
    // frame, not a tuning knob: anything near it means the measurement is
    // wrong, and a twelfth of the frame is already far more than any real
    // blanking.
    let cap_w = (tally.width as f64 * MAX_FRACTION) as u32;
    let cap_h = (tally.height as f64 * MAX_FRACTION) as u32;
    crop.left = even(crop.left.min(cap_w));
    crop.right = even(crop.right.min(cap_w));
    crop.top = even(crop.top.min(cap_h));
    crop.bottom = even(crop.bottom.min(cap_h));

    Detection::Found(crop)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What kind of band to put along the bottom.
    #[derive(Clone, Copy, PartialEq)]
    enum Band {
        None,
        /// Smeared horizontal streaks with the chroma torn out — what a real
        /// capture actually shows, and the case the first version of this
        /// detector could not see: it is horizontally *smooth*.
        Streaks,
        /// Uniform static. Worn tapes do produce this, so it stays covered.
        Snow,
    }

    /// Build a frame as three full-size planes.
    ///
    /// The picture must look like picture, which means *moderate* detail. A
    /// checkerboard was the first attempt and it broke the band test by
    /// carrying more detail than the band did. This is a broad gradient with a
    /// little hash on top, which lands in the range a real capture measures.
    fn frame(
        w: usize,
        h: usize,
        (l, r, t, b): (usize, usize, usize, usize),
        band: Band,
        band_rows: usize,
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut y_p = vec![0u8; w * h];
        let mut u_p = vec![128u8; w * h];
        let mut v_p = vec![128u8; w * h];

        for y in t..h - b {
            for x in l..w - r {
                // A smooth gradient, texture that repeats every row, and a
                // small per-row wobble. Smooth matters: an earlier fixture
                // used a wrapping `% 50` ramp, which put a hard horizontal
                // step every third row and made a *picture* row read as a
                // 4.4x break — tripping the band test on the fixture rather
                // than on anything real.
                let ramp = x * 150 / w + y * 40 / h;
                let texture = (x * 37) % 13;
                let wobble = (y * 7) % 5;
                y_p[y * w + x] = (40 + ramp + texture + wobble) as u8;
                // Slowly varying color, as a picture has.
                u_p[y * w + x] = (128 + ((x / 24 + y / 24) % 9) as i32 - 4) as u8;
                v_p[y * w + x] = (128 + ((x / 31 + y / 19) % 9) as i32 - 4) as u8;
            }
        }

        for (i, y) in (h.saturating_sub(band_rows)..h).enumerate() {
            for x in 0..w {
                match band {
                    Band::None => {}
                    Band::Streaks => {
                        // Long horizontal smears: almost no horizontal detail,
                        // but each line is unrelated to the one above and the
                        // chroma is wild. This is the shape that matters.
                        let smear = ((x / 60 + i * 7) % 6) * 34;
                        y_p[y * w + x] = (30 + smear) as u8;
                        u_p[y * w + x] = (40 + ((i * 53 + x / 80) % 170)) as u8;
                        v_p[y * w + x] = (220 - ((i * 37 + x / 70) % 170)) as u8;
                    }
                    Band::Snow => {
                        y_p[y * w + x] = if x % 2 == 0 { 250 } else { 10 };
                    }
                }
            }
        }
        (y_p, u_p, v_p)
    }

    fn run_band(
        w: usize,
        h: usize,
        m: (usize, usize, usize, usize),
        band: Band,
        rows: usize,
    ) -> EdgeCrop {
        let mut tally = EdgeTally::new(w, h);
        let (y_p, u_p, v_p) = frame(w, h, m, band, rows);
        for _ in 0..4 {
            tally.add_frame(FramePlanes {
                y: &y_p,
                u: &u_p,
                v: &v_p,
                width: w,
                height: h,
            });
        }
        match detect(&tally) {
            Detection::Found(c) => c,
            Detection::Inconclusive(why) => panic!("inconclusive: {why}"),
        }
    }

    /// Build a frame whose band has one picture row buried inside it.
    ///
    /// `gap_from_bottom` counts up from the last row, so a band of 8 rows with
    /// a gap at 2 is torn everywhere except the third row from the bottom.
    fn run_band_with_gap(
        w: usize,
        h: usize,
        band: Band,
        rows: usize,
        gap_from_bottom: usize,
    ) -> EdgeCrop {
        let mut tally = EdgeTally::new(w, h);
        let (mut y_p, mut u_p, mut v_p) = frame(w, h, (0, 0, 0, 0), band, rows);
        // The gap row is a *copy of the row above it*, not a row of picture.
        // That is what an untorn row inside a band actually looks like to
        // this detector, and the distinction matters: dropping picture in
        // here instead leaves the row wildly unlike the band row above it, so
        // it still reads as band and the fixture tests nothing. Copying the
        // row above drives its break to zero, which is the only way to make a
        // row inside a band fail the test.
        let y = h - 1 - gap_from_bottom;
        for x in 0..w {
            y_p[y * w + x] = y_p[(y - 1) * w + x];
            u_p[y * w + x] = u_p[(y - 1) * w + x];
            v_p[y * w + x] = v_p[(y - 1) * w + x];
        }
        for _ in 0..4 {
            tally.add_frame(FramePlanes {
                y: &y_p,
                u: &u_p,
                v: &v_p,
                width: w,
                height: h,
            });
        }
        match detect(&tally) {
            Detection::Found(c) => c,
            Detection::Inconclusive(why) => panic!("inconclusive: {why}"),
        }
    }

    #[test]
    fn dark_but_grainy_blanking_is_still_blanking() {
        // Taken from an ATI All-In-Wonder capture of a Hi8 tape, where the
        // left blanking is noisy and the right is clean. The picture is soft,
        // so its own detail is low (4.3) and the relative bar lands at 1.93 —
        // under the blanking's own grain. The scan stopped at the fourth
        // column and reported 6 where 18 are blanking, while the quiet right
        // side measured correctly: one crop, right on one side, wrong on the
        // other.
        let plateau = 152.0;
        let pic_detail = 4.3;

        // 17 columns of dark, grainy blanking, then the picture ramps up.
        let mut level = vec![22.0, 22.0, 23.0, 24.0, 26.0, 28.0, 28.0, 28.0, 28.0];
        level.extend([28.0, 28.0, 29.0, 30.0, 32.0, 31.0, 27.0, 36.0]);
        level.extend([103.0, 159.0, 175.0, 179.0, 195.0, 208.0, 216.0, 221.0]);
        level.extend(std::iter::repeat_n(221.0, 40));

        let mut detail = vec![0.17, 0.38, 0.90, 1.21, 1.96, 2.79, 2.69, 2.70, 2.67];
        detail.extend([2.56, 2.71, 2.88, 2.89, 3.00, 3.30, 1.88, 4.03]);
        detail.extend([13.79, 13.66, 11.85, 10.46, 9.91, 9.57, 8.83, 7.88]);
        detail.extend(std::iter::repeat_n(6.0, 40));

        let (blank, ramp) = scan(&level, &detail, plateau, pic_detail);
        assert_eq!(blank, 16, "the grainy blanking is counted, not cut short");
        assert!(
            (16..=20).contains(&(blank + ramp)),
            "and the ramp stops in the picture's shoulder, not inside it:              blank={blank} ramp={ramp}"
        );
    }

    #[test]
    fn a_dark_scene_is_not_mistaken_for_blanking() {
        // The floor must not swallow genuinely dark *content*. A dark scene
        // carries structure, which is the whole basis of the detail test.
        let plateau = 152.0;
        let pic_detail = 30.0;
        let level: Vec<f64> = std::iter::repeat_n(40.0, 60).collect();
        let detail: Vec<f64> = std::iter::repeat_n(18.0, 60).collect();
        let (blank, _) = scan(&level, &detail, plateau, pic_detail);
        assert_eq!(blank, 0, "dark content with structure is not blanking");
    }

    #[test]
    fn one_untorn_row_inside_a_band_does_not_end_it() {
        // A head-switching band is ragged, and a row in the middle of it can
        // fall just under the threshold. The scan used to stop dead at that
        // row: with the gap two rows up, `run` was still 1 — under
        // `NOISE_MIN_ROWS` — so nothing was committed and the bottom came
        // back uncropped on a tape with a plainly visible band.
        let whole = run_band(720, 480, (0, 0, 0, 0), Band::Streaks, 8);
        let gapped = run_band_with_gap(720, 480, Band::Streaks, 8, 2);

        assert!(whole.bottom >= 8, "the unbroken band is found: {whole:?}");
        assert_eq!(
            gapped.bottom, whole.bottom,
            "a single untorn row inside the band must not change the crop"
        );
    }

    #[test]
    fn two_untorn_rows_do_end_the_band() {
        // The allowance is one row. Two in a row is the band genuinely
        // ending, and the scan must not keep walking up into the picture.
        let c = run_band(720, 480, (0, 0, 0, 0), Band::Streaks, 4);
        assert!(
            c.bottom <= 8,
            "the crop stays near the band rather than climbing: {c:?}"
        );
    }

    fn run(w: usize, h: usize, m: (usize, usize, usize, usize), noise: usize) -> EdgeCrop {
        let band = if noise == 0 { Band::None } else { Band::Snow };
        run_band(w, h, m, band, noise)
    }

    /// Build a frame with a band that does *not* reach the last row, with
    /// `under` rows of flat black beneath it. This is the shape the bottom
    /// scan used to miss entirely.
    fn run_band_lifted(
        w: usize,
        h: usize,
        band: Band,
        rows: usize,
        under: usize,
    ) -> EdgeCrop {
        let mut tally = EdgeTally::new(w, h);
        let (mut y_p, mut u_p, mut v_p) = frame(w, h, (0, 0, 0, 0), band, rows + under);
        // Overwrite the lowest `under` rows of the band with picture, taken
        // from the same formula the rest of the frame uses. Deliberately not
        // black: flat black rows *are* blanking, the blanking scan finds them
        // and starts the band search above them, which is already correct
        // behaviour. The case that was broken is a row the blanking scan
        // cannot claim.
        for y in h - under..h {
            for x in 0..w {
                let ramp = x * 150 / w + y * 40 / h;
                let texture = (x * 37) % 13;
                let wobble = (y * 7) % 5;
                y_p[y * w + x] = (40 + ramp + texture + wobble) as u8;
                u_p[y * w + x] = (128 + ((x / 24 + y / 24) % 9) as i32 - 4) as u8;
                v_p[y * w + x] = (128 + ((x / 31 + y / 19) % 9) as i32 - 4) as u8;
            }
        }
        for _ in 0..4 {
            tally.add_frame(FramePlanes {
                y: &y_p,
                u: &u_p,
                v: &v_p,
                width: w,
                height: h,
            });
        }
        match detect(&tally) {
            Detection::Found(c) => c,
            Detection::Inconclusive(why) => panic!("inconclusive: {why}"),
        }
    }

    #[test]
    fn the_margin_only_grows_edges_that_found_something() {
        let c = EdgeCrop {
            left: 14,
            right: 12,
            top: 0,
            bottom: 10,
            noise_rows: 10,
        };
        let m = c.with_margin(SAFETY_MARGIN);
        assert_eq!((m.left, m.right, m.bottom), (16, 14, 12));
        assert_eq!(m.top, 0, "a clean edge must not be cut into");
        assert_eq!(m.noise_rows, 12, "the band tracks the bottom it lives in");
    }

    #[test]
    fn a_clean_frame_stays_clean_with_a_margin() {
        let m = EdgeCrop::default().with_margin(SAFETY_MARGIN);
        assert!(m.is_nothing());
    }

    /// Every crop has to be even for subsampled chroma, and the detector
    /// already rounds outward, so the margin must preserve that.
    #[test]
    fn the_margin_keeps_every_edge_even() {
        let c = EdgeCrop {
            left: 14,
            right: 12,
            top: 2,
            bottom: 10,
            noise_rows: 8,
        };
        let m = c.with_margin(SAFETY_MARGIN);
        for v in [m.left, m.right, m.top, m.bottom, m.noise_rows] {
            assert_eq!(v % 2, 0, "{m:?}");
        }
    }

    #[test]
    fn finds_lopsided_bars() {
        let c = run(720, 480, (20, 12, 0, 0), 0);
        assert_eq!(c.left, 20);
        assert_eq!(c.right, 12);
        assert_eq!(c.top, 0);
        assert_eq!(c.bottom, 0);
        assert!(c.is_lopsided());
    }

    #[test]
    fn finds_nothing_on_a_full_frame() {
        let c = run(720, 480, (0, 0, 0, 0), 0);
        assert!(c.is_nothing(), "{c:?}");
    }

    #[test]
    fn finds_the_head_switching_band() {
        let c = run(720, 480, (0, 0, 0, 0), 10);
        assert_eq!(c.noise_rows, 10);
        assert_eq!(c.bottom, 10);
        assert_eq!(c.left, 0);
        assert_eq!(c.right, 0);
    }

    /// The band spans the full width, so it must not be allowed to make the
    /// side bars look like picture. This is the bug the inset exists for.
    #[test]
    fn a_noise_band_does_not_hide_side_bars() {
        let c = run(720, 480, (20, 12, 0, 0), 10);
        assert_eq!((c.left, c.right), (20, 12), "{c:?}");
        assert_eq!(c.bottom, 10);
    }

    /// A row that is neither blanking nor band must not hide the band.
    ///
    /// The scan used to start at the last row and stop at the first row that
    /// was not torn, so one transitional row underneath ended it before it
    /// began. Reported from testing on a Video8 capture: an obvious band at
    /// the bottom, and the only crop the software offered was from the sides.
    #[test]
    fn one_row_under_the_band_does_not_hide_it() {
        let c = run_band_lifted(720, 480, Band::Streaks, 8, 1);
        assert!(c.noise_rows >= 8, "the band was missed: {c:?}");
        // The row under it goes as well: removing a torn band and leaving
        // what sits beneath it is not a crop anybody wants.
        assert!(c.bottom >= 9, "the row under the band was left behind: {c:?}");
    }

    /// Two rows is the documented limit and must still work.
    #[test]
    fn two_rows_under_the_band_still_finds_it() {
        let c = run_band_lifted(720, 480, Band::Streaks, 8, 2);
        assert!(c.noise_rows >= 8, "{c:?}");
        assert!(c.bottom >= 10, "{c:?}");
    }

    /// The skip is a short reach from the bottom, not a licence to hunt.
    ///
    /// Head switching is at the bottom of the frame by definition. Something
    /// torn-looking six rows up is in the picture, and cropping to it would
    /// take real content with it.
    #[test]
    fn a_band_too_far_from_the_bottom_is_not_head_switching() {
        let c = run_band_lifted(720, 480, Band::Streaks, 8, 6);
        assert_eq!(c.noise_rows, 0, "picture was mistaken for a band: {c:?}");
    }

    /// Flat black under a band is blanking, and that path was already right:
    /// the blanking scan claims those rows and the band search starts above
    /// them. Pinned so the fix above cannot be "simplified" into breaking it.
    #[test]
    fn blanking_under_a_band_is_cropped_and_the_band_still_found() {
        let mut tally = EdgeTally::new(720, 480);
        let (mut y_p, mut u_p, mut v_p) = frame(720, 480, (0, 0, 0, 0), Band::Streaks, 12);
        for y in 480 - 4..480 {
            for x in 0..720 {
                y_p[y * 720 + x] = 0;
                u_p[y * 720 + x] = 128;
                v_p[y * 720 + x] = 128;
            }
        }
        for _ in 0..4 {
            tally.add_frame(FramePlanes {
                y: &y_p,
                u: &u_p,
                v: &v_p,
                width: 720,
                height: 480,
            });
        }
        let Detection::Found(c) = detect(&tally) else {
            panic!("inconclusive")
        };
        assert!(c.noise_rows >= 6, "the band above the blanking was missed: {c:?}");
        assert!(c.bottom >= 10, "{c:?}");
    }

    #[test]
    #[ignore = "diagnostic"]
    fn dump_fixture_metrics() {
        let (w, h) = (720usize, 480usize);
        let mut tally = EdgeTally::new(w, h);
        let (y_p, u_p, v_p) = frame(w, h, (0, 0, 0, 0), Band::Streaks, 8);
        for _ in 0..4 {
            tally.add_frame(FramePlanes {
                y: &y_p,
                u: &u_p,
                v: &v_p,
                width: w,
                height: h,
            });
        }
        let br = tally.row_breaks();
        let cb = tally.row_chroma_breaks();
        let rd = tally.row_details();
        let (y0, y1) = inset(h);
        let (pb, pc, pd) = (
            median(&br[y0..y1]),
            median(&cb[y0..y1]),
            median(&rd[y0..y1]),
        );
        println!("picture medians: break {pb:.2} chroma {pc:.2} detail {pd:.2}");
        for y in h - 12..h {
            println!(
                "  {y}  break {:7.2} ({:5.2}x)  chroma {:7.2} ({:5.2}x)  detail {:7.2} ({:5.2}x)",
                br[y], br[y] / pb, cb[y], cb[y] / pc, rd[y], rd[y] / pd
            );
        }
    }

    /// The regression test for the real bug: a user's capture had a visible
    /// head-switching band and the detector reported none, because the band
    /// was smeared horizontal streaks rather than snow and the only trigger
    /// was horizontal detail.
    #[test]
    fn finds_a_streaky_band_not_just_snow() {
        let c = run_band(720, 480, (0, 0, 0, 0), Band::Streaks, 8);
        assert_eq!(c.noise_rows, 8, "the streaky band was missed again");
        assert_eq!(c.bottom, 8);
    }

    /// And the horizontal-detail trigger must survive, because worn tapes do
    /// produce real snow.
    #[test]
    fn still_finds_a_snow_band() {
        let c = run_band(720, 480, (0, 0, 0, 0), Band::Snow, 10);
        assert_eq!(c.noise_rows, 10);
    }

    #[test]
    fn a_streaky_band_is_found_alongside_lopsided_bars() {
        let c = run_band(720, 480, (20, 12, 0, 0), Band::Streaks, 8);
        assert_eq!((c.left, c.right), (20, 12), "{c:?}");
        assert_eq!(c.noise_rows, 8, "{c:?}");
    }

    /// A hard horizontal edge at the very bottom — a table edge, a skirting
    /// board — breaks from the row above just like a band does, but only for
    /// one line.
    #[test]
    fn a_single_hard_edge_at_the_bottom_is_not_a_band() {
        let w = 720;
        let h = 480;
        let mut tally = EdgeTally::new(w, h);
        let (mut y_p, u_p, v_p) = frame(w, h, (0, 0, 0, 0), Band::None, 0);
        // One bright line across the bottom row.
        for x in 0..w {
            y_p[(h - 1) * w + x] = 250;
        }
        for _ in 0..4 {
            tally.add_frame(FramePlanes {
                y: &y_p,
                u: &u_p,
                v: &v_p,
                width: w,
                height: h,
            });
        }
        let Detection::Found(c) = detect(&tally) else {
            panic!("inconclusive");
        };
        assert_eq!(c.noise_rows, 0, "a single picture edge was called a band");
    }

    #[test]
    fn ignores_a_single_flickering_line() {
        let c = run(720, 480, (1, 0, 1, 0), 0);
        assert!(c.is_nothing(), "{c:?}");
    }

    #[test]
    fn a_dark_frame_is_inconclusive_rather_than_cropped() {
        let w = 720;
        let h = 480;
        let mut tally = EdgeTally::new(w, h);
        let mut data = vec![0u8; w * h];
        // Dim but detailed throughout: no blanking at all, and far too dark to
        // judge. The wrong answer here is to crop it to nothing.
        for y in 0..h {
            for x in 0..w {
                data[y * w + x] = if (x + y) % 2 == 0 { 8 } else { 34 };
            }
        }
        let flat = vec![128u8; w * h];
        tally.add_frame(FramePlanes {
            y: &data,
            u: &flat,
            v: &flat,
            width: w,
            height: h,
        });
        assert!(matches!(detect(&tally), Detection::Inconclusive(_)));
    }

    #[test]
    fn no_frames_is_inconclusive() {
        let tally = EdgeTally::new(720, 480);
        assert!(matches!(detect(&tally), Detection::Inconclusive(_)));
    }

    #[test]
    fn a_wrong_sized_frame_is_ignored_not_fatal() {
        let mut tally = EdgeTally::new(720, 480);
        let data = vec![128u8; 16];
        tally.add_frame(FramePlanes {
            y: &data,
            u: &data,
            v: &data,
            width: 4,
            height: 4,
        });
        assert_eq!(tally.frames(), 0);
    }

    #[test]
    fn the_cap_bounds_a_pathological_frame() {
        // Blanking wider than the cap: the recommendation is clamped rather
        // than eating an eighth of the picture.
        let c = run(720, 480, (200, 200, 0, 0), 0);
        assert_eq!(c.left, 86);
        assert_eq!(c.right, 86);
    }

    #[test]
    fn odd_margins_round_outward() {
        let c = run(720, 480, (15, 9, 0, 0), 0);
        assert_eq!((c.left, c.right), (16, 10), "{c:?}");
    }
}
