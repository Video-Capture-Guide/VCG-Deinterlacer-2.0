//! A short side-by-side clip of the tape before and after.
//!
//! Fifteen seconds from the middle of the finished video, the untouched
//! source on the left and the restored file on the right, both magnified to
//! 200% and split by a yellow line. It exists to be *shown* — to a relative,
//! on a forum, in a review — which is why it is one small MP4 with no sound
//! rather than two files to play in turn.
//!
//! Three decisions are load-bearing:
//!
//! * **It is rendered from the finished file, not from a second pass of the
//!   pipeline.** What goes in the right-hand panel is exactly what the user
//!   has on disk, so the clip cannot flatter the export.
//! * **The magnification is nearest-neighbor.** Interpolating on the way up
//!   would smooth away comb teeth and grain, which is the entire thing being
//!   compared. The geometry match that comes first — putting the source's
//!   non-square pixels onto the output's square ones — is a real resample
//!   and uses a real filter, and both sides go through the identical zoom
//!   afterwards so neither is favored.
//! * **Both panels show the same moment and the same part of the frame.** A
//!   trimmed export does not run on the same clock as its source, so the
//!   source timestamp is mapped back through the trim ranges rather than
//!   assumed equal.

use std::path::{Path, PathBuf};
use std::time::Duration;

use vcg_core::config::Config;
use vcg_core::watermark::{escape_drawtext, fontfile_arg, find_system_font};

use crate::paths::ToolPaths;
use crate::run;

/// How long the clip runs. Long enough to see motion — which is where
/// deinterlacing shows — and short enough to sit in a forum post.
pub const CLIP_SECONDS: f64 = 15.0;

/// A seek into a long capture plus a short encode. Generous, because the
/// seek is the slow half on a two-hour AVI and the video is already safely
/// written by the time this runs.
const TIMEOUT: Duration = Duration::from_secs(300);

/// Where a moment in the finished video sits in the source.
///
/// With no trim the two clocks are the same, and the double-rate option does
/// not change that: bobbing doubles the frame count and leaves the duration
/// alone. With trims they are not: output frame *n* is the *n*-th frame of
/// the kept ranges laid end to end, which can be anywhere in the source.
/// Getting this wrong puts two different scenes side by side and labels them
/// before and after.
pub fn source_time_for_output(ranges: &[(i64, i64)], fps: f64, out_seconds: f64) -> f64 {
    if ranges.is_empty() || fps <= 0.0 {
        return out_seconds;
    }
    let mut want = (out_seconds * fps).round() as i64;
    for &(first, last) in ranges {
        let length = (last - first + 1).max(0);
        if want < length {
            return (first + want) as f64 / fps;
        }
        want -= length;
    }
    // Past the last kept frame, which rounding at the very end can reach.
    ranges
        .last()
        .map(|&(_, last)| last as f64 / fps)
        .unwrap_or(out_seconds)
}

/// The name the clip is written under, beside the finished video.
pub fn comparison_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "export".to_string());
    output
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join(format!("{stem}.comparison.mp4"))
}

/// Render the clip. Returns where it was written.
///
/// Every failure is an `Err` the caller is free to ignore: the export itself
/// has already succeeded by the time this runs, and losing an hour's render
/// over a bonus file would be absurd.
pub fn render(
    tools: &ToolPaths,
    config: &Config,
    source: &Path,
    output: &Path,
    source_fps: f64,
) -> Result<PathBuf, String> {
    let ffmpeg = tools.ffmpeg_path().ok_or("ffmpeg was not found")?;
    let (width, height, duration) = probe_video(tools, output)?;
    if width < 32 || height < 32 {
        return Err(format!("the finished video is only {width}×{height}"));
    }

    let clip = CLIP_SECONDS.min(duration);
    let out_start = ((duration - clip) / 2.0).max(0.0);
    let source_start = source_time_for_output(&config.trim_ranges, source_fps, out_start);

    let dest = comparison_path(output);
    let args = command_args(
        source,
        output,
        &dest,
        source_start,
        out_start,
        clip,
        width,
        height,
        find_system_font().as_deref(),
    );

    let mut cmd = run::command(ffmpeg);
    cmd.args(&args);
    let result = run::run(cmd, TIMEOUT).map_err(|e| e.to_string())?;
    if !result.success() || !dest.exists() {
        return Err(format!(
            "the comparison clip could not be made.\n{}",
            result.combined()
        ));
    }
    Ok(dest)
}

/// Build the whole command line, so it can be tested without running ffmpeg.
#[allow(clippy::too_many_arguments)]
fn command_args(
    source: &Path,
    output: &Path,
    dest: &Path,
    source_start: f64,
    out_start: f64,
    clip: f64,
    width: u32,
    height: u32,
    font: Option<&Path>,
) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-y".into(),
        // Decode errors in a damaged capture must not stop a bonus file.
        "-fflags".into(),
        "+discardcorrupt".into(),
        // Before -i, so the seek is a keyframe jump rather than a decode of
        // everything up to the mark. On a two-hour tape that is the
        // difference between a few seconds and a few minutes.
        "-ss".into(),
        format!("{source_start:.3}"),
        "-i".into(),
        source.display().to_string(),
        "-ss".into(),
        format!("{out_start:.3}"),
        "-i".into(),
        output.display().to_string(),
        "-filter_complex".into(),
        filter_graph(width, height, font),
        "-map".into(),
        "[v]".into(),
        // No sound: the clip is a picture comparison, and two tracks of the
        // same audio would be the loudest thing about it.
        "-an".into(),
        "-t".into(),
        format!("{clip:.3}"),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        "20".into(),
        // 4:2:0 and even dimensions, or half the players that matter will
        // not open it — and a comparison nobody can play proves nothing.
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-movflags".into(),
        "+faststart".into(),
        dest.display().to_string(),
    ]
}

/// The side-by-side graph.
///
/// Both inputs are put onto the output's geometry first — which is what
/// corrects the source's non-square pixels — and then given the identical
/// 200% nearest-neighbor zoom and the identical center crop, so the only
/// difference left between the panels is the restoration itself.
fn filter_graph(width: u32, height: u32, font: Option<&Path>) -> String {
    // Every dimension even, because yuv420p subsamples by two and an odd one
    // is rejected outright.
    let half = (width / 2) & !1;
    let big_w = width * 2;
    let big_h = height * 2;
    // The crop *offsets* have to be even as well as the sizes: 4:2:0 stores
    // one chroma sample per two luma columns, so an odd offset either shifts
    // the color against the picture or is silently rounded, and which of the
    // two happens depends on the build.
    let crop_x = ((big_w - half) / 2) & !1;
    let crop_y = ((big_h - height) / 2) & !1;
    let canvas = half * 2;

    let side = |input: &str, label: &str| {
        format!(
            "[{input}]scale={width}:{height}:flags=lanczos,setsar=1,\
             scale={big_w}:{big_h}:flags=neighbor,\
             crop={half}:{height}:{crop_x}:{crop_y},\
             format=yuv420p[{label}]"
        )
    };

    // A box behind the caption, because a white label on tape footage can
    // land on a white shirt and vanish.
    let caption = |input: &str, text: &str, label: &str| {
        format!(
            "[{input}]drawtext={}text='{}':\
             x=14:y=14:fontsize=20:fontcolor=white:\
             box=1:boxcolor=black@0.55:boxborderw=8[{label}]",
            fontfile_arg(font),
            escape_drawtext(text)
        )
    };

    [
        side("0:v", "l"),
        side("1:v", "r"),
        caption("l", "Original at 200%", "lt"),
        caption("r", "VCGD at 200%", "rt"),
        "[lt][rt]hstack=inputs=2[s]".to_string(),
        // The divider straddles the seam rather than sitting to one side of
        // it, so neither panel loses more picture than the other.
        format!("[s]drawbox=x={}:y=0:w=4:h={height}:color=yellow:t=fill[v]", canvas / 2 - 2),
    ]
    .join(";")
}

/// Width, height and duration of a finished file.
fn probe_video(tools: &ToolPaths, file: &Path) -> Result<(u32, u32, f64), String> {
    let ffprobe = tools.ffprobe_path().ok_or("ffprobe was not found")?;
    let out = run::run_tool(
        ffprobe,
        [
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1",
            &file.display().to_string(),
        ],
    )
    .map_err(|e| e.to_string())?;

    let find = |key: &str| {
        out.stdout
            .lines()
            .find_map(|l| l.strip_prefix(key).map(str::trim))
            .and_then(|v| v.parse::<f64>().ok())
    };
    let width = find("width=").ok_or("the finished video has no width")? as u32;
    let height = find("height=").ok_or("the finished video has no height")? as u32;
    let duration = find("duration=").ok_or("the finished video has no duration")?;
    Ok((width, height, duration))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without trims the two clocks are the same one.
    #[test]
    fn an_untrimmed_export_runs_on_the_source_clock() {
        assert_eq!(source_time_for_output(&[], 29.97, 61.5), 61.5);
    }

    /// With trims they are not, and taking the output timestamp as a source
    /// timestamp would put two different scenes side by side under labels
    /// reading "before" and "after".
    #[test]
    fn a_trimmed_export_maps_back_through_the_ranges() {
        let fps = 30.0;
        // Keep frames 300-599 and 1800-2099: ten seconds from 0:10 and ten
        // from 1:00, which the export lays out as one twenty-second file.
        let ranges = [(300, 599), (1800, 2099)];

        // The start of the output is the start of the first kept range.
        assert!((source_time_for_output(&ranges, fps, 0.0) - 10.0).abs() < 0.01);
        // Five seconds in is still in the first range.
        assert!((source_time_for_output(&ranges, fps, 5.0) - 15.0).abs() < 0.01);
        // Eleven seconds in has crossed into the second, which is fifty
        // seconds further down the tape than the naive answer.
        assert!((source_time_for_output(&ranges, fps, 11.0) - 61.0).abs() < 0.01);
    }

    /// Rounding at the very end must not walk off the last range.
    #[test]
    fn the_end_of_the_output_lands_inside_the_source() {
        let got = source_time_for_output(&[(0, 299)], 30.0, 10.0);
        assert!((got - 299.0 / 30.0).abs() < 0.01, "{got}");
    }

    /// yuv420p rejects an odd dimension outright, and the clip is encoded
    /// 4:2:0 so it plays everywhere. An odd source width must not produce a
    /// graph that cannot run.
    #[test]
    fn every_dimension_in_the_graph_is_even() {
        for width in [640u32, 720, 702, 768] {
            let graph = filter_graph(width, 480, None);
            for number in graph
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse::<u32>().ok())
            {
                // The only odd numbers allowed are the stream indices and
                // the small literals in the caption, which are well under
                // the frame size.
                if number > 100 {
                    assert_eq!(number % 2, 0, "odd {number} in graph for {width}: {graph}");
                }
            }
        }
    }

    /// The divider sits on the seam, not beside it.
    #[test]
    fn the_yellow_line_is_centered_on_the_join() {
        let graph = filter_graph(640, 480, None);
        assert!(graph.contains("drawbox=x=318:y=0:w=4:"), "{graph}");
        assert!(graph.contains("color=yellow"), "{graph}");
    }

    /// Nearest-neighbor on the way up: smoothing the magnified picture would
    /// erase the comb teeth and the grain, which is what is being compared.
    #[test]
    fn the_magnification_does_not_interpolate() {
        let graph = filter_graph(640, 480, None);
        assert_eq!(graph.matches("flags=neighbor").count(), 2, "{graph}");
    }

    /// The clip has no sound, and both panels are the same moment.
    #[test]
    fn the_command_is_silent_and_time_aligned() {
        let args = command_args(
            Path::new("in.avi"),
            Path::new("out.mov"),
            Path::new("out.comparison.mp4"),
            12.5,
            12.5,
            15.0,
            640,
            480,
            None,
        );
        assert!(args.iter().any(|a| a == "-an"), "{args:?}");
        assert_eq!(args.iter().filter(|a| *a == "-ss").count(), 2, "{args:?}");
        // Both seeks come before their own input, or ffmpeg decodes the
        // whole file up to the mark instead of jumping to it.
        let ss = args.iter().position(|a| a == "-ss").unwrap();
        assert_eq!(args[ss + 2], "-i", "{args:?}");
    }

    /// The name is predictable, because the finished page lists it and the
    /// user has to find it beside the video a month later.
    #[test]
    fn the_clip_sits_beside_the_video_under_its_own_name() {
        let got = comparison_path(Path::new(r"D:\tapes\Tape 01_VCGD200.mov"));
        assert_eq!(
            got.file_name().unwrap().to_string_lossy(),
            "Tape 01_VCGD200.comparison.mp4"
        );
    }
}
