//! Video scopes: RGB parade, stacked RGB histogram, and vectorscope.
//!
//! These answer questions no number can. A parade shows *where across the
//! frame* a channel clips — a window blown out on one side of the picture
//! looks nothing like an overexposed whole frame, and the two want different
//! responses. A vectorscope shows a color cast as a cloud pushed off center,
//! which is far quicker to read than "U is 6.2 away from neutral".
//!
//! Each builds a small RGB image the caller uploads as a texture, rather than
//! returning counts for the caller to draw. Drawing a scope is most of the
//! work in making one, and putting it here keeps it testable and keeps the
//! same scope looking identical wherever it appears.
//!
//! Input is always an 8-bit RGB frame, which is what the preview path already
//! produces.

/// A drawn scope, ready to upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeImage {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

impl ScopeImage {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            rgb: vec![0; width * height * 3],
        }
    }

    /// Add to a pixel, saturating. Scopes accumulate, so writes pile up
    /// rather than replace.
    fn add(&mut self, x: usize, y: usize, color: [u8; 3]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = (y * self.width + x) * 3;
        for (slot, add) in self.rgb[i..i + 3].iter_mut().zip(color) {
            *slot = slot.saturating_add(add);
        }
    }

    fn set(&mut self, x: usize, y: usize, color: [u8; 3]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = (y * self.width + x) * 3;
        self.rgb[i..i + 3].copy_from_slice(&color);
    }
}

/// Height of the parade and vectorscope panels.
const PANEL: usize = 256;

/// How much one sample brightens a scope pixel.
///
/// Low, because a scope is a density plot: the interesting thing is where
/// samples pile up, and a value high enough to make single samples visible
/// saturates the busy areas into a solid block.
const INK: u8 = 18;

/// The luma levels worth ruling on a waveform: the legal floor and ceiling.
const GUIDES: [u8; 2] = [16, 235];

/// Build an RGB parade — three waveforms side by side, one per channel.
///
/// Each panel plots frame column against value, so the shape of the picture
/// is preserved left to right and a problem can be located in the frame
/// rather than merely detected.
pub fn parade(rgb: &[u8], width: usize, height: usize) -> ScopeImage {
    let mut out = ScopeImage::new(PANEL * 3, PANEL);
    if width == 0 || height == 0 || rgb.len() < width * height * 3 {
        return out;
    }

    // The guide lines first, so samples draw over them. Dim, because a
    // guide brighter than the data turns the scope into a picture of its
    // own graticule.
    for panel in 0..3 {
        for guide in GUIDES {
            let y = PANEL - 1 - (guide as usize * (PANEL - 1) / 255);
            for x in 0..PANEL {
                out.set(panel * PANEL + x, y, [0, 38, 0]);
            }
        }
    }

    // Each channel writes its own component and nothing else. A trace of
    // ink in the other two looks harmless on one sample and turns the whole
    // scope white on real footage: a panel column takes hundreds of samples,
    // and saturating_add carries a secondary of 2 to 255 after 128 of them.
    // The result was three white waveforms under a label promising red,
    // green and blue.
    let channel_ink = [[INK, 0, 0], [0, INK, 0], [0, 0, INK]];
    for y in 0..height {
        for x in 0..width {
            // Each source column fills the span of panel columns it covers.
            // Mapping to a single column instead leaves vertical gaps
            // whenever the source is narrower than the panel, which reads as
            // a combed scope rather than a narrow one.
            let from = x * PANEL / width;
            let to = (((x + 1) * PANEL / width).max(from + 1)).min(PANEL);
            let i = (y * width + x) * 3;
            for (c, ink) in channel_ink.iter().enumerate() {
                let v = rgb[i + c] as usize;
                let py = PANEL - 1 - (v * (PANEL - 1) / 255);
                for px in from..to {
                    out.add(c * PANEL + px, py, *ink);
                }
            }
        }
    }
    out
}

/// Build three stacked histograms, one per channel.
///
/// Stacked rather than overlaid because overlaid channels on a tape capture
/// sit almost on top of each other, and the question — is one channel
/// shifted or clipped relative to the others — is exactly the one an overlay
/// makes hardest to answer.
pub fn rgb_histogram(rgb: &[u8], width: usize, height: usize) -> ScopeImage {
    const BAND: usize = 80;
    let mut out = ScopeImage::new(256, BAND * 3);
    if width == 0 || height == 0 || rgb.len() < width * height * 3 {
        return out;
    }

    let mut bins = [[0u64; 256]; 3];
    for p in rgb.as_chunks::<3>().0.iter().take(width * height) {
        for (bin, v) in bins.iter_mut().zip(p) {
            bin[*v as usize] += 1;
        }
    }

    let colors = [[235u8, 60, 60], [60, 235, 60], [90, 120, 245]];
    for (c, channel) in bins.iter().enumerate() {
        // Scaled per channel against a high percentile, so one spike does
        // not flatten the rest — the same reasoning as the luma histogram.
        let mut sorted: Vec<u64> = channel.iter().copied().filter(|&n| n > 0).collect();
        if sorted.is_empty() {
            continue;
        }
        sorted.sort_unstable();
        let idx = ((sorted.len() as f64 * 0.98) as usize).min(sorted.len() - 1);
        let ceiling = sorted[idx].max(1) as f64;

        let base = c * BAND;
        for (v, &n) in channel.iter().enumerate() {
            let h = ((n as f64 / ceiling).min(1.0) * (BAND - 6) as f64) as usize;
            for dy in 0..h {
                out.set(v, base + BAND - 2 - dy, colors[c]);
            }
        }
        // A rule under each band, so the three read as separate plots.
        for x in 0..256 {
            out.set(x, base + BAND - 1, [60, 60, 70]);
        }
    }
    out
}

/// What a filter changed, and by how much.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Difference {
    /// The largest change to any one channel of any pixel, in 8-bit levels.
    pub peak: u8,
    /// Share of pixels the filter altered at all.
    pub changed: f64,
    /// Mean change across every pixel and channel.
    pub mean: f64,
}

/// Draw what a filter did: the difference between two frames, amplified.
///
/// Some of these filters are meant to be subtle, and subtle is
/// indistinguishable from broken when the before and after are two pictures
/// side by side. Dehaloing a tape moves a handful of levels on a handful of
/// edges; at any zoom, two panes of the same shoulder look identical, and the
/// honest question — "is this doing anything at all" — has no answer on the
/// page. This gives it one.
///
/// Amplified, because an unamplified difference of four levels is black. The
/// gain is stated on screen next to it, so nobody reads the brightness here
/// as the size of the effect.
///
/// Returns the drawing and the numbers, because a picture of a faint change
/// still needs a figure beside it: "the most any pixel moved is 7 levels" is
/// the sentence that settles it.
pub fn difference(
    before: &[u8],
    after: &[u8],
    width: usize,
    height: usize,
    gain: f32,
) -> (ScopeImage, Difference) {
    let mut out = ScopeImage::new(width, height);
    let stats = Difference {
        peak: 0,
        changed: 0.0,
        mean: 0.0,
    };
    let n = width * height;
    if width == 0 || height == 0 || before.len() < n * 3 || after.len() < n * 3 {
        return (out, stats);
    }

    let gain = gain.max(1.0);
    let mut peak = 0u8;
    let mut changed = 0u64;
    let mut total = 0u64;
    for i in 0..n {
        let (b, a) = (&before[i * 3..i * 3 + 3], &after[i * 3..i * 3 + 3]);
        let mut moved = false;
        for c in 0..3 {
            let d = b[c].abs_diff(a[c]);
            if d > 0 {
                moved = true;
            }
            peak = peak.max(d);
            total += d as u64;
            // Amplified and clamped, per channel: a change that is only in
            // the blue channel should look blue, not gray. On a chroma shift
            // that is the whole story.
            let lit = ((d as f32 * gain).round() as u32).min(255) as u8;
            out.rgb[i * 3 + c] = lit;
        }
        if moved {
            changed += 1;
        }
    }

    (
        out,
        Difference {
            peak,
            changed: changed as f64 / n as f64,
            mean: total as f64 / (n as f64 * 3.0),
        },
    )
}

/// How far from the center the rim of the vectorscope sits, at unity gain.
///
/// Chroma from 8-bit RGB runs to about 127 on each axis, so at unity the rim
/// is as far out as a sample can go and the panel holds everything.
const CHROMA_FULL: f64 = 127.5;

/// Build a vectorscope: chroma plotted as hue angle and saturation radius.
///
/// A color cast shows as the whole cloud displaced from the center, which
/// is the quickest read there is on whether a tape has gone warm or cool.
///
/// `gain` magnifies the plot about the center. A tape's cast is a handful of
/// chroma units on a scale that runs to 127, so at unity the entire finding
/// is a smudge a dozen pixels wide in the middle of an empty circle — which
/// is what the Color tint page was showing, side by side, as its answer to
/// "did the correction fix the cast". Above unity the cloud is large enough
/// to read and to compare.
///
/// Both halves of a before/after are always drawn at the same gain. A scale
/// fitted to each frame would redraw the corrected one larger and make a
/// successful correction look like it had done nothing.
pub fn vectorscope(rgb: &[u8], width: usize, height: usize, gain: f64) -> ScopeImage {
    let mut out = ScopeImage::new(PANEL, PANEL);
    if width == 0 || height == 0 || rgb.len() < width * height * 3 {
        return out;
    }
    let center = (PANEL / 2) as i32;
    let edge = PANEL as f64 / 2.0 - 2.0;

    // Graticule: the neutral axes, the rim, and a ring at half of it. The
    // half ring is what makes a displacement readable as a quantity — with
    // the rim alone there is nothing between the center and the edge to
    // judge "how far" against.
    for (radius, ink) in [(edge, [0, 60, 0]), (edge / 2.0, [0, 34, 0])] {
        for a in 0..720 {
            let t = (a as f64 / 2.0).to_radians();
            let x = center + (radius * t.cos()) as i32;
            let y = center + (radius * t.sin()) as i32;
            if x >= 0 && y >= 0 {
                out.set(x as usize, y as usize, ink);
            }
        }
    }
    for d in 0..PANEL {
        out.set(d, center as usize, [0, 45, 0]);
        out.set(center as usize, d, [0, 45, 0]);
    }

    // Sample rather than read every pixel: a vectorscope is a density plot
    // and a quarter of a standard-definition frame is already far more
    // samples than the 256x256 grid can distinguish.
    let step = if width * height > 200_000 { 3 } else { 1 };
    let scale = edge * gain.max(0.1) / CHROMA_FULL;
    for p in rgb.as_chunks::<3>().0.iter().take(width * height).step_by(step) {
        let (r, g, b) = (p[0] as f64, p[1] as f64, p[2] as f64);
        // BT.601, the matrix a tape capture is in.
        let u = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
        let v = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
        // U right, V up, which is the orientation every scope uses: red
        // lands upper left, blue lower right, as on a broadcast graticule.
        let (mut dx, mut dy) = (u * scale, -v * scale);
        // A sample past the rim is pulled back along its own direction
        // rather than dropped or clamped to the panel's corner. Dropping it
        // silently empties the scope of exactly the colors worth seeing;
        // clamping each axis separately moves it to a hue it never had.
        let r = dx.hypot(dy);
        if r > edge {
            dx *= edge / r;
            dy *= edge / r;
        }
        let x = center + dx as i32;
        let y = center + dy as i32;
        if x >= 0 && y >= 0 {
            out.add(x as usize, y as usize, [INK, INK, INK]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of one flat color.
    fn flat(color: [u8; 3], width: usize, height: usize) -> Vec<u8> {
        color
            .iter()
            .copied()
            .cycle()
            .take(width * height * 3)
            .collect()
    }

    fn brightest_row(img: &ScopeImage, x: usize) -> Option<usize> {
        (0..img.height)
            .max_by_key(|&y| {
                let i = (y * img.width + x) * 3;
                img.rgb[i] as u32 + img.rgb[i + 1] as u32 + img.rgb[i + 2] as u32
            })
            .filter(|&y| {
                let i = (y * img.width + x) * 3;
                img.rgb[i] as u32 + img.rgb[i + 1] as u32 + img.rgb[i + 2] as u32 > 0
            })
    }

    #[test]
    fn every_scope_survives_an_empty_frame() {
        for img in [
            parade(&[], 0, 0),
            rgb_histogram(&[], 0, 0),
            vectorscope(&[], 0, 0, 2.0),
        ] {
            assert_eq!(img.rgb.len(), img.width * img.height * 3);
        }
    }

    /// A short buffer must not be read past its end.
    #[test]
    fn a_truncated_frame_is_refused_rather_than_read_past() {
        let short = vec![0u8; 10];
        let img = parade(&short, 640, 480);
        assert_eq!(img.width, PANEL * 3);
    }

    /// White sits at the top of a waveform and black at the bottom. Getting
    /// this upside down is the classic scope bug and invisible in a thumbnail.
    #[test]
    fn the_parade_puts_white_at_the_top() {
        let white = parade(&flat([255, 255, 255], 64, 64), 64, 64);
        let black = parade(&flat([0, 0, 0], 64, 64), 64, 64);
        assert_eq!(brightest_row(&white, 10), Some(0));
        assert_eq!(brightest_row(&black, 10), Some(PANEL - 1));
    }

    /// Each channel gets its own panel, in R, G, B order.
    #[test]
    fn the_parade_separates_the_channels() {
        // Red full, green half, blue off.
        let img = parade(&flat([255, 128, 0], 64, 64), 64, 64);
        assert_eq!(brightest_row(&img, 10), Some(0), "red panel, full");
        let g = brightest_row(&img, PANEL + 10).expect("green panel");
        assert!((120..=136).contains(&g), "green panel, mid: {g}");
        assert_eq!(
            brightest_row(&img, PANEL * 2 + 10),
            Some(PANEL - 1),
            "blue panel, empty"
        );
    }

    /// A flat gray frame is neutral, so its vectorscope cloud sits on the
    /// center. A cast is read as displacement from there, so if neutral did
    /// not land in the middle every reading would be wrong.
    /// The parade's whole claim is that each panel is one channel, and the
    /// color is how that reads at a glance. Ink bleeding into the other two
    /// components is invisible on one sample and turns every dense column
    /// white — which is what real footage produced.
    #[test]
    fn the_parade_keeps_each_panel_its_own_color() {
        // Tall enough that each panel column takes hundreds of samples,
        // which is the condition that whitened it.
        let img = parade(&flat([255, 255, 255], 64, 400), 64, 400);
        for (panel, channel) in [(0, 0), (1, 1), (2, 2)] {
            let x = panel * PANEL + 10;
            let lit: Vec<u8> = img.rgb[x * 3..x * 3 + 3].to_vec();
            assert!(lit[channel] > 200, "panel {panel} should be saturated: {lit:?}");
            for other in 0..3 {
                if other != channel {
                    assert_eq!(lit[other], 0, "panel {panel} bled into {other}: {lit:?}");
                }
            }
        }
    }

    #[test]
    fn neutral_gray_lands_at_the_center_of_the_vectorscope() {
        let img = vectorscope(&flat([128, 128, 128], 64, 64), 64, 64, 2.0);
        let c = PANEL / 2;
        let i = (c * img.width + c) * 3;
        assert!(img.rgb[i] > 60, "the center should be the densest point");
    }

    /// A strong blue cast must push the cloud towards +U, which is right of
    /// center. The direction is the whole value of the scope.
    #[test]
    fn a_blue_cast_pushes_the_cloud_right_of_center() {
        let img = vectorscope(&flat([40, 40, 220], 64, 64), 64, 64, 1.0);
        let c = PANEL / 2;
        assert!(
            densest_column(&img) > c,
            "blue should sit right of center, got {}",
            densest_column(&img)
        );
    }

    /// Where the cloud is in the panel is the reading, so gain has to move
    /// it: the Color tint page shows a cast that is a few chroma units
    /// wide, and at unity it is a smudge on the center with nothing to
    /// compare against.
    #[test]
    fn gain_pushes_the_cloud_further_from_the_center() {
        let cast = flat([150, 128, 110], 64, 64);
        let c = (PANEL / 2) as i32;
        let near = (densest_column(&vectorscope(&cast, 64, 64, 1.0)) as i32 - c).abs();
        let far = (densest_column(&vectorscope(&cast, 64, 64, 3.0)) as i32 - c).abs();
        assert!(near > 0, "the cast should be off center at all");
        assert!(far > near * 2, "gain 3 should be much further out: {near} to {far}");
    }

    /// A saturated color past the rim is pulled back along its own
    /// direction. Dropping it empties the scope of the colors most worth
    /// seeing; clamping each axis separately moves it to a hue it never had
    /// — a red would slide towards the corner and read as magenta.
    #[test]
    fn a_sample_past_the_rim_keeps_its_hue() {
        // Pure red sits upper left, at about 109 degrees. At gain 4 it is
        // far outside the panel.
        let img = vectorscope(&flat([255, 0, 0], 32, 32), 32, 32, 4.0);
        let c = (PANEL / 2) as f64;
        let mut hit = None;
        for y in 0..img.height {
            for x in 0..img.width {
                let i = (y * img.width + x) * 3;
                if img.rgb[i] > 200 {
                    hit = Some((x as f64, y as f64));
                }
            }
        }
        let (x, y) = hit.expect("the sample must still be drawn");
        let angle = (c - y).atan2(x - c).to_degrees().rem_euclid(360.0);
        assert!(
            (104.0..=114.0).contains(&angle),
            "red must stay at its own angle, got {angle:.0}"
        );
        let r = (x - c).hypot(y - c);
        assert!(r > 100.0 && r < 128.0, "and sit on the rim, got {r:.0}");
    }

    /// Two identical frames are a black picture and a zero reading. This is
    /// the answer to "is the filter doing anything", so it has to be able to
    /// say no.
    #[test]
    fn an_unchanged_frame_differences_to_nothing() {
        let f = flat([120, 130, 140], 8, 8);
        let (img, d) = difference(&f, &f, 8, 8, 8.0);
        assert!(img.rgb.iter().all(|&v| v == 0), "all black");
        assert_eq!(d.peak, 0);
        assert_eq!(d.changed, 0.0);
    }

    /// And it has to be able to say yes, for a change far too small to see
    /// in the two pictures side by side — which is the case that made this
    /// necessary.
    #[test]
    fn a_change_of_three_levels_is_visible_once_amplified() {
        let before = flat([100, 100, 100], 8, 8);
        let after = flat([103, 100, 100], 8, 8);
        let (img, d) = difference(&before, &after, 8, 8, 16.0);
        assert_eq!(d.peak, 3, "the reading is in real levels, not amplified");
        assert_eq!(d.changed, 1.0, "every pixel moved");
        // 3 x 16 = 48, which is plainly visible against black.
        assert_eq!(img.rgb[0], 48);
        assert_eq!(img.rgb[1], 0, "only the red channel moved");
    }

    /// Amplification saturates rather than wrapping. A wrap would draw a big
    /// change as black, which is the one reading that must never be wrong.
    #[test]
    fn a_large_change_saturates_instead_of_wrapping() {
        let before = flat([0, 0, 0], 4, 4);
        let after = flat([200, 0, 0], 4, 4);
        let (img, d) = difference(&before, &after, 4, 4, 8.0);
        assert_eq!(d.peak, 200);
        assert_eq!(img.rgb[0], 255);
    }

    #[test]
    fn a_short_frame_is_refused_rather_than_read_past() {
        let (img, d) = difference(&[0; 4], &[0; 4], 64, 64, 4.0);
        assert_eq!(img.rgb.len(), 64 * 64 * 3);
        assert_eq!(d.peak, 0);
    }

    /// The column carrying the most ink.
    fn densest_column(img: &ScopeImage) -> usize {
        let mut best = (0u32, 0usize);
        for x in 0..img.width {
            let total: u32 = (0..img.height)
                .map(|y| img.rgb[(y * img.width + x) * 3] as u32)
                .sum();
            if total > best.0 {
                best = (total, x);
            }
        }
        best.1
    }

    #[test]
    fn the_histogram_puts_each_channel_in_its_own_band() {
        let img = rgb_histogram(&flat([255, 0, 0], 32, 32), 32, 32);
        assert_eq!(img.width, 256);
        // Red is full, so its band has ink at value 255.
        let red_col = (0..80)
            .map(|y| img.rgb[(y * img.width + 255) * 3])
            .max()
            .unwrap_or(0);
        assert!(red_col > 100, "red band should be lit at 255");
        // Green is zero, so its band has ink at value 0.
        let green_col = (80..160)
            .map(|y| img.rgb[(y * img.width) * 3 + 1])
            .max()
            .unwrap_or(0);
        assert!(green_col > 100, "green band should be lit at 0");
    }
}
