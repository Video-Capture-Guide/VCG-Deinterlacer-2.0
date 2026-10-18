//! A luma histogram, for showing the levels rather than stating them.
//!
//! The levels probe already reports the darkest and lightest values it found,
//! but two numbers cannot show *shape* — whether the picture is bunched into
//! the middle, clipped flat against black, or spread evenly. That shape is
//! the whole reason to look at levels, and a graph shows it at a glance where
//! a pair of numbers does not.
//!
//! Counts are kept per 8-bit value so the broadcast limits at 16 and 235 fall
//! on exact bins and can be ruled on the drawing.

/// The legal broadcast range for 8-bit video.
pub const LEGAL_MIN: u8 = 16;
pub const LEGAL_MAX: u8 = 235;

/// How often each luma value occurs across the sampled frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LumaHistogram {
    pub bins: Vec<u64>,
    pub frames: u32,
}

impl Default for LumaHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LumaHistogram {
    pub fn new() -> Self {
        Self {
            bins: vec![0; 256],
            frames: 0,
        }
    }

    /// Fold one grayscale plane in.
    pub fn add_plane(&mut self, luma: &[u8]) {
        if luma.is_empty() {
            return;
        }
        for &v in luma {
            self.bins[v as usize] += 1;
        }
        self.frames += 1;
    }

    /// How far in from each edge the levels measurement ignores the picture.
    ///
    /// Standard practice when reading levels off a tape, and for a concrete
    /// reason: capture-card blanking is not dark picture, it is *no signal*,
    /// and it sits at or below the legal floor on every frame. Measured over
    /// the whole raster it drags the black point to 16 or under and reports
    /// a tape as crushed when the picture inside the frame is nowhere near
    /// black. The head-switching band at the bottom does the same at the
    /// other extreme, with torn signal that can read as either.
    ///
    /// Twenty is comfortably past the worst blanking seen on the samples
    /// here — the measured crop on the development tape is fourteen columns
    /// a side — while still leaving 97% of a 720x480 frame, which is far
    /// more picture than a histogram needs.
    pub const MEASURE_INSET: usize = 20;

    /// Fold in one plane, ignoring a margin at every edge.
    ///
    /// The margin is skipped rather than cropped: nothing is copied, the
    /// rows are simply read from their interior.
    pub fn add_plane_inset(&mut self, luma: &[u8], width: usize, height: usize) {
        let inset = Self::MEASURE_INSET;
        // Too small to inset is not an error — measure what there is.
        if width <= inset * 2 + 2 || height <= inset * 2 + 2 || luma.len() < width * height {
            self.add_plane(luma);
            return;
        }
        for y in inset..height - inset {
            let row = &luma[y * width + inset..y * width + width - inset];
            for &v in row {
                self.bins[v as usize] += 1;
            }
        }
        self.frames += 1;
    }

    pub fn total(&self) -> u64 {
        self.bins.iter().sum()
    }

    pub fn is_empty(&self) -> bool {
        self.frames == 0 || self.total() == 0
    }

    /// The lowest value holding at least `fraction` of the pixels below it.
    ///
    /// A percentile rather than the outright minimum, because a single stuck
    /// pixel or one frame of head-switching noise would otherwise put the
    /// black point at 0 on a picture that never goes near it.
    pub fn percentile(&self, fraction: f64) -> u8 {
        let total = self.total();
        if total == 0 {
            return 0;
        }
        let want = (total as f64 * fraction.clamp(0.0, 1.0)).ceil() as u64;
        let mut seen = 0u64;
        for (v, &n) in self.bins.iter().enumerate() {
            seen += n;
            if seen >= want {
                return v as u8;
            }
        }
        255
    }

    /// Build from one RGB frame, in the same stored-value scale as a plane
    /// read straight off the file.
    ///
    /// The subtlety that made this wrong once: a preview frame arrives as
    /// `rgb24`, and converting YUV to RGB *expands* limited range — a stored
    /// 16 comes out as 0 and a stored 235 as 255. Taking BT.601 luma of that
    /// and plotting it beside a histogram of stored values puts the two on
    /// different scales, so the same picture appeared to run far wider at one
    /// moment than across the whole video. Worse, the legal markers at 16 and
    /// 235 meant nothing against it: in expanded space the floor is 0.
    ///
    /// So the expansion is undone. 0 maps back to 16 and 255 back to 235,
    /// which is the scale legality is judged in and the scale the rest of
    /// this module speaks.
    pub fn from_rgb(rgb: &[u8], width: usize, height: usize) -> Self {
        let mut h = Self::new();
        if width == 0 || height == 0 || rgb.len() < width * height * 3 {
            return h;
        }
        let span = (LEGAL_MAX - LEGAL_MIN) as f64 / 255.0;
        for p in rgb.as_chunks::<3>().0.iter().take(width * height) {
            let full = 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64;
            let stored = LEGAL_MIN as f64 + full * span;
            h.bins[(stored.round().clamp(0.0, 255.0)) as usize] += 1;
        }
        h.frames = 1;
        h
    }

    /// The darkest and lightest values that actually occur.
    ///
    /// The outright extremes, not percentiles: "does this picture go outside
    /// the legal range" is a question about the extremes, and answering it
    /// with a percentile would quietly say no when the answer is yes.
    pub fn extremes(&self) -> Option<(u8, u8)> {
        let lo = self.bins.iter().position(|&n| n > 0)?;
        let hi = 255 - self.bins.iter().rev().position(|&n| n > 0)?;
        Some((lo as u8, hi as u8))
    }

    /// Fraction of pixels below the legal floor.
    pub fn below_legal(&self) -> f64 {
        self.fraction_in(0..LEGAL_MIN as usize)
    }

    /// Fraction of pixels above the legal ceiling.
    pub fn above_legal(&self) -> f64 {
        self.fraction_in(LEGAL_MAX as usize + 1..256)
    }

    fn fraction_in(&self, range: std::ops::Range<usize>) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        let n: u64 = self.bins[range].iter().sum();
        n as f64 / total as f64
    }

    /// Bin heights scaled to 0..1 for drawing.
    ///
    /// Scaled against a high percentile of the bin counts rather than the
    /// single tallest bin: one enormous spike — a letterboxed black bar, a
    /// flat studio backdrop — would otherwise flatten everything else to
    /// invisibility, which is precisely the shape the graph exists to show.
    pub fn normalized(&self) -> Vec<f32> {
        let mut sorted: Vec<u64> = self.bins.iter().copied().filter(|&n| n > 0).collect();
        if sorted.is_empty() {
            return vec![0.0; 256];
        }
        sorted.sort_unstable();
        let idx = ((sorted.len() as f64 * 0.98) as usize).min(sorted.len() - 1);
        let ceiling = sorted[idx].max(1) as f32;
        self.bins
            .iter()
            .map(|&n| (n as f32 / ceiling).min(1.0))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(values: &[(u8, usize)]) -> Vec<u8> {
        let mut v = Vec::new();
        for (value, count) in values {
            v.extend(std::iter::repeat_n(*value, *count));
        }
        v
    }

    /// The question the Levels page asks is about the extremes, so they are
    /// reported as extremes. A file whose darkest pixel is 9 is outside the
    /// legal range even if only a handful of pixels are down there — the
    /// first version of this screen hid exactly that behind a tolerance and
    /// told the user everything was fine while the card beside it said the
    /// opposite.
    #[test]
    fn the_extremes_are_extremes_not_percentiles() {
        let mut h = LumaHistogram::new();
        h.add_plane(&plane(&[(9, 1), (120, 100_000), (229, 1)]));
        assert_eq!(h.extremes(), Some((9, 229)));
        assert!(h.below_legal() > 0.0, "one pixel still counts");
    }

    #[test]
    fn a_frame_of_known_gray_lands_in_one_bin() {
        // BT.601 luma of a neutral gray is the gray itself, then the limited
        // range is restored: 90 of 255 sits 90/255 of the way up the 16-235
        // window, which is 93.
        let rgb: Vec<u8> = std::iter::repeat_n(90u8, 32 * 32 * 3).collect();
        let h = LumaHistogram::from_rgb(&rgb, 32, 32);
        assert_eq!(h.extremes(), Some((93, 93)));
        assert_eq!(h.total(), 32 * 32);
    }

    #[test]
    fn a_short_rgb_buffer_is_refused() {
        assert!(LumaHistogram::from_rgb(&[1, 2, 3], 64, 64).is_empty());
    }

    #[test]
    fn an_empty_histogram_has_no_extremes() {
        assert_eq!(LumaHistogram::new().extremes(), None);
    }

    #[test]
    fn an_empty_histogram_reports_nothing_rather_than_dividing_by_zero() {
        let h = LumaHistogram::new();
        assert!(h.is_empty());
        assert_eq!(h.total(), 0);
        assert_eq!(h.percentile(0.5), 0);
        assert_eq!(h.below_legal(), 0.0);
        assert_eq!(h.above_legal(), 0.0);
        assert_eq!(h.normalized().len(), 256);
    }

    #[test]
    fn counts_land_in_the_right_bins() {
        let mut h = LumaHistogram::new();
        h.add_plane(&plane(&[(16, 10), (128, 20), (235, 5)]));
        assert_eq!(h.bins[16], 10);
        assert_eq!(h.bins[128], 20);
        assert_eq!(h.bins[235], 5);
        assert_eq!(h.total(), 35);
        assert_eq!(h.frames, 1);
    }

    #[test]
    fn out_of_range_pixels_are_measured_on_both_sides() {
        let mut h = LumaHistogram::new();
        h.add_plane(&plane(&[(0, 10), (128, 80), (255, 10)]));
        assert!((h.below_legal() - 0.1).abs() < 1e-9);
        assert!((h.above_legal() - 0.1).abs() < 1e-9);
    }

    /// 16 and 235 are legal, so they must not count as outside.
    #[test]
    fn the_legal_limits_themselves_are_inside() {
        let mut h = LumaHistogram::new();
        h.add_plane(&plane(&[(16, 50), (235, 50)]));
        assert_eq!(h.below_legal(), 0.0);
        assert_eq!(h.above_legal(), 0.0);
    }

    /// A percentile, not a minimum: one stray pixel must not define the
    /// black point of the whole picture.
    #[test]
    fn a_single_stray_pixel_does_not_move_the_percentile() {
        let mut h = LumaHistogram::new();
        h.add_plane(&plane(&[(0, 1), (100, 9_999)]));
        assert_eq!(h.percentile(0.01), 100, "the stray is below the 1% mark");
        assert_eq!(h.bins[0], 1, "but it is still counted");
    }

    /// One huge spike must not flatten the rest of the graph.
    #[test]
    fn a_dominant_spike_does_not_flatten_everything_else() {
        let mut h = LumaHistogram::new();
        h.add_plane(&plane(&[(0, 1_000_000)]));
        for v in 60..200u8 {
            h.add_plane(&plane(&[(v, 500)]));
        }
        let n = h.normalized();
        assert_eq!(n[0], 1.0, "the spike is clipped to the top");
        assert!(
            n[120] > 0.2,
            "ordinary content must stay visible, got {}",
            n[120]
        );
    }

    #[test]
    fn adding_an_empty_plane_is_not_a_frame() {
        let mut h = LumaHistogram::new();
        h.add_plane(&[]);
        assert_eq!(h.frames, 0);
        assert!(h.is_empty());
    }

    /// A preview frame arrives as expanded RGB: converting YUV to RGB turns a
    /// stored 16 into 0 and a stored 235 into 255. Histogramming that raw put
    /// the moment graph on a different scale from the whole-video one — the
    /// same picture looked far darker and far brighter at one moment than it
    /// ever was — and left the legal markers at 16 and 235 pointing at
    /// nothing. Black and white must land back on the legal limits.
    #[test]
    fn an_expanded_rgb_frame_comes_back_in_stored_values() {
        let black = LumaHistogram::from_rgb(&[0, 0, 0], 1, 1);
        assert_eq!(black.extremes(), Some((LEGAL_MIN, LEGAL_MIN)));

        let white = LumaHistogram::from_rgb(&[255, 255, 255], 1, 1);
        assert_eq!(white.extremes(), Some((LEGAL_MAX, LEGAL_MAX)));

        // And mid gray lands mid legal window, not at 128.
        let gray = LumaHistogram::from_rgb(&[128, 128, 128], 1, 1);
        let (lo, _) = gray.extremes().expect("one value");
        assert!((124..=128).contains(&lo), "mid gray landed at {lo}");
    }

    /// The point of the scale fix: a frame of ordinary picture content must
    /// report the same legality as the plane it came from.
    #[test]
    fn the_two_sources_agree_about_the_legal_range() {
        // A preview frame whose darkest pixel is RGB 0 — which is stored 16,
        // exactly legal, not below it.
        let frame = [0u8, 0, 0, 255, 255, 255];
        let h = LumaHistogram::from_rgb(&frame, 2, 1);
        assert_eq!(h.below_legal(), 0.0, "RGB 0 is the legal floor, not under it");
        assert_eq!(h.above_legal(), 0.0, "RGB 255 is the legal ceiling");
    }
}

#[cfg(test)]
mod inset_tests {
    use super::*;

    /// The bug this exists for: capture-card blanking is no signal, not dark
    /// picture. Measured over the whole raster it puts the black point on the
    /// floor and reports a perfectly healthy tape as crushed.
    #[test]
    fn blanking_at_the_edges_does_not_decide_the_black_point() {
        let (w, h) = (200usize, 120usize);
        let mut frame = vec![0u8; w * h];
        // Picture well inside the legal range, with 16 columns of black
        // blanking down each side and 12 rows of it at the bottom.
        for y in 0..h {
            for x in 0..w {
                let blank = x < 16 || x >= w - 16 || y >= h - 12;
                frame[y * w + x] = if blank { 0 } else { 90 };
            }
        }

        let mut whole = LumaHistogram::new();
        whole.add_plane(&frame);
        assert_eq!(
            whole.extremes(),
            Some((0, 90)),
            "the whole raster reads the blanking as the black point"
        );
        // 32 of 200 columns and 12 of 120 rows is a shade under a quarter of
        // the raster, and the whole-frame measurement calls all of it illegal.
        assert!(
            whole.below_legal() > 0.2,
            "a quarter of the raster is blanking: {}",
            whole.below_legal()
        );

        let mut inset = LumaHistogram::new();
        inset.add_plane_inset(&frame, w, h);
        assert_eq!(
            inset.extremes(),
            Some((90, 90)),
            "inset sees only the picture"
        );
        assert_eq!(inset.below_legal(), 0.0);
    }

    /// A frame too small to inset is measured whole rather than refused.
    #[test]
    fn a_tiny_frame_is_measured_rather_than_skipped() {
        let frame = vec![77u8; 8 * 8];
        let mut h = LumaHistogram::new();
        h.add_plane_inset(&frame, 8, 8);
        assert_eq!(h.extremes(), Some((77, 77)));
        assert_eq!(h.frames, 1);
    }

    /// One frame in is one frame counted, whichever way it was added.
    #[test]
    fn the_frame_count_is_the_same_either_way() {
        let frame = vec![100u8; 200 * 120];
        let mut a = LumaHistogram::new();
        a.add_plane_inset(&frame, 200, 120);
        let mut b = LumaHistogram::new();
        b.add_plane(&frame);
        assert_eq!(a.frames, b.frames);
        assert!(a.total() < b.total(), "but fewer pixels are counted");
    }
}
