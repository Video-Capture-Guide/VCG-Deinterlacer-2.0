//! What the sound track is, and whether one side of it is missing.
//!
//! No counterpart in the Python, which never looks at the audio. It matters
//! on tape: a deck with one channel recorded, or a capture cable with a dead
//! core, produces a file that plays out of one speaker. On headphones that is
//! unmissable and infuriating, and the fix — copy the live channel to both —
//! is lossless and obvious once you know.
//!
//! Three outcomes, and only two of them are faults:
//!
//! * **One channel.** A genuinely mono file. Most players center it, so this
//!   is a convenience rather than a defect.
//! * **Two channels, one silent.** The real fault, and the one worth
//!   catching.
//! * **Two channels carrying the same thing.** Dual mono, which is what a
//!   mono tape captured over a stereo cable gives you. It already plays from
//!   both speakers; there is nothing to fix, and "fixing" it would be a
//!   no-op dressed up as a repair.
//!
//! Levels come from FFmpeg's `astats`, which reports per-channel RMS in dB
//! and uses `-inf` for digital silence.

/// A channel's measured level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelLevel {
    /// RMS in dBFS. `None` for `-inf`, i.e. pure digital silence.
    pub rms_db: Option<f64>,
    pub peak_db: Option<f64>,
}

impl ChannelLevel {
    /// Whether this channel carries anything a listener would hear.
    ///
    /// Digital silence is the clear case; the threshold catches a channel
    /// carrying only the noise floor of a dead input, which measures very
    /// low but is not exactly zero.
    pub fn is_silent(&self, floor_db: f64) -> bool {
        match self.rms_db {
            None => true,
            Some(db) => db < floor_db,
        }
    }
}

/// Below this a channel is treated as carrying nothing. A dead input on a
/// capture card still measures around -80 dB of its own noise.
pub const SILENCE_FLOOR_DB: f64 = -70.0;

/// How far below the other channel a side must sit to count as missing.
///
/// A real stereo recording with the action on one side can be 15-20 dB apart;
/// a dead channel is far further away than that.
pub const IMBALANCE_DB: f64 = 40.0;

/// What should be done with the sound, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioVerdict {
    /// No audio track at all.
    None,
    /// Nothing worth changing.
    Fine,
    /// A single channel, which can be presented as stereo.
    Mono,
    /// Two channels with one of them empty. `live` is the one with the sound.
    SilentSide { live: u8 },
}

impl AudioVerdict {
    /// Whether copying one channel to both would change anything.
    pub fn can_fix(self) -> bool {
        matches!(self, AudioVerdict::Mono | AudioVerdict::SilentSide { .. })
    }

    /// The channel to take the sound from when fixing.
    pub fn source_channel(self) -> u8 {
        match self {
            AudioVerdict::SilentSide { live } => live,
            _ => 0,
        }
    }
}

/// Everything measured about the sound.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioAnalysis {
    pub channels: u32,
    pub levels: Vec<ChannelLevel>,
    pub verdict: AudioVerdict,
}

/// Parse `astats` output into per-channel levels.
///
/// `astats` prints a block per channel, then an `Overall` block repeating the
/// same field names. Everything from `Overall` onward is ignored.
pub fn parse_astats(text: &str) -> Vec<ChannelLevel> {
    let mut levels: Vec<ChannelLevel> = Vec::new();
    let mut current: Option<ChannelLevel> = None;

    for line in text.lines() {
        // Strip FFmpeg's "[Parsed_astats_0 @ 0x...]" prefix if present.
        let body = match line.rfind("] ") {
            Some(i) => &line[i + 2..],
            None => line,
        }
        .trim();

        // astats closes with an "Overall" block repeating the same field
        // names. Without this its figures overwrite the last channel's — and
        // the last channel is exactly the one most likely to be the silent
        // side, so the fault being looked for would be erased by the summary
        // describing it.
        if body == "Overall" {
            if let Some(done) = current.take() {
                levels.push(done);
            }
            continue;
        }

        if let Some(rest) = body.strip_prefix("Channel:") {
            if let Some(done) = current.take() {
                levels.push(done);
            }
            let _ = rest;
            current = Some(ChannelLevel {
                rms_db: None,
                peak_db: None,
            });
        } else if let Some(rest) = body.strip_prefix("RMS level dB:") {
            if let Some(c) = current.as_mut() {
                c.rms_db = parse_db(rest);
            }
        } else if let Some(rest) = body.strip_prefix("Peak level dB:")
            && let Some(c) = current.as_mut()
        {
            c.peak_db = parse_db(rest);
        }
    }
    if let Some(done) = current.take() {
        levels.push(done);
    }
    levels
}

/// `-inf` and anything unparseable become `None`, which reads as silence.
fn parse_db(raw: &str) -> Option<f64> {
    let t = raw.trim();
    if t.eq_ignore_ascii_case("-inf") {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Decide what, if anything, is wrong with the sound.
pub fn classify(channels: u32, levels: &[ChannelLevel]) -> AudioVerdict {
    if channels == 0 {
        return AudioVerdict::None;
    }
    if channels == 1 {
        return AudioVerdict::Mono;
    }
    // Only the first two channels are considered. Surround on a tape capture
    // does not happen, and guessing which of six channels to duplicate would
    // be worse than leaving it alone.
    let (Some(a), Some(b)) = (levels.first(), levels.get(1)) else {
        return AudioVerdict::Fine;
    };

    let a_silent = a.is_silent(SILENCE_FLOOR_DB);
    let b_silent = b.is_silent(SILENCE_FLOOR_DB);
    match (a_silent, b_silent) {
        // Both empty: there is no sound to copy anywhere.
        (true, true) => AudioVerdict::Fine,
        (false, true) => AudioVerdict::SilentSide { live: 0 },
        (true, false) => AudioVerdict::SilentSide { live: 1 },
        (false, false) => {
            // Neither is silent outright, but one may still be far enough
            // down to be a dead channel picking up crosstalk.
            match (a.rms_db, b.rms_db) {
                (Some(x), Some(y)) if x - y > IMBALANCE_DB => {
                    AudioVerdict::SilentSide { live: 0 }
                }
                (Some(x), Some(y)) if y - x > IMBALANCE_DB => {
                    AudioVerdict::SilentSide { live: 1 }
                }
                _ => AudioVerdict::Fine,
            }
        }
    }
}

/// Fold one window's levels into the running measurement, keeping the loudest
/// reading seen for each channel.
///
/// The question a window answers is "did this channel carry anything here",
/// and the answer that matters across the whole file is "anywhere". A tape
/// whose first reel is a camcorder's mono mic and whose second is a stereo
/// feed must come out as stereo: averaging would drag the live side down
/// towards the dead one, and taking the last window would let whichever
/// window happened to be last decide.
///
/// Silence — `None` — never displaces a measured level, so a window that
/// landed on a gap cannot make a live channel look dead.
pub fn merge_loudest(into: &mut Vec<ChannelLevel>, window: &[ChannelLevel]) {
    if into.len() < window.len() {
        into.resize(
            window.len(),
            ChannelLevel {
                rms_db: None,
                peak_db: None,
            },
        );
    }
    for (slot, new) in into.iter_mut().zip(window) {
        slot.rms_db = louder(slot.rms_db, new.rms_db);
        slot.peak_db = louder(slot.peak_db, new.peak_db);
    }
}

/// The louder of two readings, treating silence as quieter than any level.
fn louder(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) => Some(x),
        (None, b) => b,
    }
}

/// Measure and classify in one step.
pub fn analyze(channels: u32, astats: &str) -> AudioAnalysis {
    let levels = parse_astats(astats);
    let verdict = classify(channels, &levels);
    AudioAnalysis {
        channels,
        levels,
        verdict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUAL_MONO: &str = include_str!("../../../../tests/fixtures/astats_dual_mono.txt");
    const SILENT_RIGHT: &str =
        include_str!("../../../../tests/fixtures/astats_silent_right.txt");
    const MONO: &str = include_str!("../../../../tests/fixtures/astats_mono.txt");

    #[test]
    fn a_real_stereo_block_parses_into_two_channels() {
        let levels = parse_astats(DUAL_MONO);
        assert_eq!(levels.len(), 2, "{levels:?}");
        // Both carry sound, and within a decibel of each other.
        let a = levels[0].rms_db.expect("left has a level");
        let b = levels[1].rms_db.expect("right has a level");
        assert!((a - b).abs() < 2.0, "{a} vs {b}");
    }

    /// The trailing "Overall" block repeats the same field names and must not
    /// be counted as another channel.
    #[test]
    fn the_overall_summary_is_not_a_channel() {
        assert_eq!(parse_astats(DUAL_MONO).len(), 2);
        assert_eq!(parse_astats(SILENT_RIGHT).len(), 2);
        assert_eq!(parse_astats(MONO).len(), 1);
    }

    #[test]
    fn digital_silence_parses_as_none_rather_than_a_number() {
        let levels = parse_astats(SILENT_RIGHT);
        assert!(levels[0].rms_db.is_some());
        assert_eq!(levels[1].rms_db, None, "-inf must not parse as a value");
        assert!(levels[1].is_silent(SILENCE_FLOOR_DB));
    }

    // ── The three outcomes, on real measurements ────────────────────────

    #[test]
    fn dual_mono_needs_no_fixing() {
        let a = analyze(2, DUAL_MONO);
        assert_eq!(a.verdict, AudioVerdict::Fine);
        assert!(!a.verdict.can_fix());
    }

    #[test]
    fn a_silent_side_is_caught_and_the_live_one_named() {
        let a = analyze(2, SILENT_RIGHT);
        assert_eq!(a.verdict, AudioVerdict::SilentSide { live: 0 });
        assert!(a.verdict.can_fix());
        assert_eq!(a.verdict.source_channel(), 0);
    }

    #[test]
    fn a_single_channel_is_mono() {
        let a = analyze(1, MONO);
        assert_eq!(a.verdict, AudioVerdict::Mono);
        assert!(a.verdict.can_fix());
    }

    #[test]
    fn no_audio_track_is_not_a_fault() {
        assert_eq!(analyze(0, "").verdict, AudioVerdict::None);
        assert!(!AudioVerdict::None.can_fix());
    }

    // ── The edges ───────────────────────────────────────────────────────

    /// Silence on both sides means there is nothing to copy. Duplicating
    /// silence into silence would report a repair that did nothing.
    #[test]
    fn two_silent_channels_cannot_be_repaired() {
        let levels = [
            ChannelLevel { rms_db: None, peak_db: None },
            ChannelLevel { rms_db: None, peak_db: None },
        ];
        assert_eq!(classify(2, &levels), AudioVerdict::Fine);
    }

    /// A real stereo mix can be lopsided — someone speaking on one side —
    /// without either channel being dead. That must not be "fixed", because
    /// the fix would throw away the other channel entirely.
    #[test]
    fn a_lopsided_but_real_stereo_mix_is_left_alone() {
        let levels = [
            ChannelLevel { rms_db: Some(-20.0), peak_db: Some(-6.0) },
            ChannelLevel { rms_db: Some(-38.0), peak_db: Some(-24.0) },
        ];
        assert_eq!(classify(2, &levels), AudioVerdict::Fine, "18 dB apart");
    }

    /// A dead channel picking up crosstalk is not digitally silent but is far
    /// below the live one.
    #[test]
    fn a_channel_far_below_the_other_counts_as_dead() {
        let levels = [
            ChannelLevel { rms_db: Some(-22.0), peak_db: Some(-8.0) },
            ChannelLevel { rms_db: Some(-75.0), peak_db: Some(-60.0) },
        ];
        assert_eq!(classify(2, &levels), AudioVerdict::SilentSide { live: 0 });
    }

    #[test]
    fn a_dead_left_channel_is_found_too() {
        let levels = [
            ChannelLevel { rms_db: None, peak_db: None },
            ChannelLevel { rms_db: Some(-24.0), peak_db: Some(-9.0) },
        ];
        assert_eq!(classify(2, &levels), AudioVerdict::SilentSide { live: 1 });
        assert_eq!(
            AudioVerdict::SilentSide { live: 1 }.source_channel(),
            1
        );
    }

    /// Surround is not a tape problem, and guessing which of six channels to
    /// duplicate would be worse than doing nothing.
    #[test]
    fn more_than_two_channels_is_judged_on_the_first_two_only() {
        let levels = [
            ChannelLevel { rms_db: Some(-20.0), peak_db: Some(-6.0) },
            ChannelLevel { rms_db: Some(-21.0), peak_db: Some(-7.0) },
            ChannelLevel { rms_db: None, peak_db: None },
        ];
        assert_eq!(classify(6, &levels), AudioVerdict::Fine);
    }

    /// Output that never reached the channel blocks must not panic or invent
    /// a verdict.
    #[test]
    fn unparseable_output_is_not_a_fault() {
        assert!(parse_astats("").is_empty());
        assert_eq!(classify(2, &[]), AudioVerdict::Fine);
    }

    /// A tape whose first reel is mono and whose second is stereo is a
    /// stereo tape. Judging it on one window at the head would copy the left
    /// channel over the right for the whole file and destroy the second
    /// reel's sound.
    #[test]
    fn a_channel_that_is_live_in_any_window_counts_as_live() {
        let mut merged = Vec::new();
        // Head: right side dead.
        merged_window(&mut merged, Some(-18.0), None);
        // Later: both sides carrying.
        merged_window(&mut merged, Some(-20.0), Some(-19.0));
        assert_eq!(classify(2, &merged), AudioVerdict::Fine);
        // And the louder reading is the one kept.
        assert_eq!(merged[0].rms_db, Some(-18.0));
    }

    /// The opposite case must still be caught: a side dead in every window
    /// is a dead side.
    #[test]
    fn a_side_dead_in_every_window_is_still_reported() {
        let mut merged = Vec::new();
        merged_window(&mut merged, Some(-18.0), None);
        merged_window(&mut merged, Some(-21.0), None);
        merged_window(&mut merged, Some(-19.0), Some(-96.0));
        assert_eq!(classify(2, &merged), AudioVerdict::SilentSide { live: 0 });
    }

    /// A window that landed on a gap measures nothing, and must not be able
    /// to erase what the other windows heard.
    #[test]
    fn a_silent_window_does_not_undo_a_live_one() {
        let mut merged = Vec::new();
        merged_window(&mut merged, Some(-18.0), Some(-19.0));
        merged_window(&mut merged, None, None);
        assert_eq!(classify(2, &merged), AudioVerdict::Fine);
    }

    fn merged_window(into: &mut Vec<ChannelLevel>, left: Option<f64>, right: Option<f64>) {
        merge_loudest(
            into,
            &[
                ChannelLevel { rms_db: left, peak_db: left },
                ChannelLevel { rms_db: right, peak_db: right },
            ],
        );
    }
}
