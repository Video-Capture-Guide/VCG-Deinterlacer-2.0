//! Running the export: VapourSynth into FFmpeg, with progress.
//!
//! Ported from `_process_single_file`, with one deliberate change.
//!
//! The Python encodes a video-only file, then **rewrites the whole thing** to
//! mux the audio back in, guarded by a disk-space pre-flight whose comment
//! says a late-dying mux "is exactly what destroyed a 2.5-hour render". Audio
//! can simply be a second input to the same FFmpeg pass, so the file is
//! written once. That removes the failure mode instead of checking for it, and
//! halves the disk traffic on a long capture.
//!
//! Progress comes from `-progress pipe:1`: because the encode writes to a file
//! path, FFmpeg's stdout is free for machine-readable progress, which is far
//! more reliable than scraping `frame=` out of the human-readable stderr.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vcg_core::colorspace::output_cs_tag;
use vcg_core::config::{Config, OutputFormat};
use vcg_core::ffmpeg::{output_args, output_frame_rate};
use vcg_core::vpy::{generate_vpy, DepsPaths};
use vcg_core::watermark::{merge_video_filter, watermark_args, VideoFilter};

use crate::paths::ToolPaths;
use crate::run::run_tool;

/// A long capture can take hours, so the encode has no deadline of its own —
/// it is stopped by the user or not at all. See `Request::cancel`.
const NO_DEADLINE: Duration = Duration::from_secs(60 * 60 * 24);

/// What a stopped render returns.
///
/// A sentinel the interface compares against, rather than a string it
/// matches on by eye: "you stopped this" and "this failed" are different
/// screens, and deciding which to show by searching an error message for a
/// word would break the first time the wording was improved.
pub const STOPPED: &str = "the export was stopped";

/// What stage the export has reached.
#[derive(Debug, Clone)]
pub enum Phase {
    ExtractingAudio,
    /// A home movie's film frames are being chosen: the script reads the
    /// whole tape before it produces its first frame, and for those minutes
    /// ffmpeg has nothing to report. Without a name for it the page would
    /// sit at 0% looking hung.
    FindingFilmFrames,
    /// Seconds of output written, against the total expected.
    Encoding {
        out_seconds: f64,
        total_seconds: f64,
        fps: f64,
        speed: f64,
    },
    Verifying,
    /// Rendering the optional side-by-side clip, after the video itself is
    /// finished and safely on disk.
    Comparing,
}

impl Phase {
    /// Fraction complete, for a progress bar.
    pub fn fraction(&self) -> f32 {
        match self {
            Phase::ExtractingAudio | Phase::FindingFilmFrames => 0.0,
            Phase::Encoding {
                out_seconds,
                total_seconds,
                ..
            } => {
                if *total_seconds > 0.0 {
                    (*out_seconds / *total_seconds).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                }
            }
            Phase::Verifying | Phase::Comparing => 1.0,
        }
    }
}

/// What the export produced.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub output: PathBuf,
    /// Duration ffprobe measured in the finished file.
    pub duration: f64,
    pub had_audio: bool,
    /// The side-by-side clip, if one was asked for and it worked. A failure
    /// here is not a failed export: the video is already written.
    pub comparison: Option<PathBuf>,
    /// What VapourSynth and FFmpeg said while they worked.
    ///
    /// Returned on success as well as failure. It was already being captured
    /// for the error path and thrown away whenever the export worked, which
    /// is exactly the case where someone later asks why the picture looks
    /// the way it does. Trimmed, because a long export's stderr is mostly
    /// repetition and a log nobody can read is not a log.
    pub tool_output: String,
}

type Progress<'a> = &'a mut dyn FnMut(Phase);

/// Whether the source has a sound track at all. `None` when it could not be
/// asked — no ffprobe, or one that failed on this file.
fn has_audio_stream(tools: &ToolPaths, file: &Path) -> Option<bool> {
    let ffprobe = tools.ffprobe_path()?;
    let out = run_tool(
        ffprobe,
        [
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
            &file.display().to_string(),
        ],
    )
    .ok()?;
    out.success().then(|| !out.stdout.trim().is_empty())
}

/// What the extraction amounts to: sound, no sound, or a failure.
///
/// These were once two outcomes, and the missing third is what shipped silent
/// files. Any extraction that did not leave a WAV behind was read as "the
/// source has no sound track", which is a legitimate thing for a tape to be —
/// so a killed, failed or timed-out extraction of a tape that plainly had
/// sound was reported, on the finished page, as a source with none.
///
/// `has_stream` is what ffprobe said beforehand. When it could not say,
/// ffmpeg's own complaint about having nothing to map is taken as the answer;
/// any other failure is a failure.
fn audio_result(
    has_stream: Option<bool>,
    out: &crate::run::Output,
    wav_bytes: u64,
) -> Result<bool, String> {
    if out.success() {
        // A track with nothing in it leaves a bare header.
        return Ok(wav_bytes > 1000);
    }
    let nothing_to_map = out.stderr.contains("does not contain any stream")
        || out.stderr.contains("matches no streams");
    if has_stream == Some(false) || (has_stream.is_none() && nothing_to_map) {
        return Ok(false);
    }
    Err(format!(
        "the sound could not be read from the source, so nothing was exported — \
         a video with its sound missing is not a finished export.\n{}",
        tail(&out.stderr, 1200)
    ))
}

/// Extract the source audio to a temporary WAV.
///
/// Decoded rather than copied, because the source codec may not be legal in
/// the output container and because a tape capture's audio is small next to
/// its video.
///
/// `Ok(false)` is a source with no sound; anything that goes wrong on a
/// source that has some is an error. See `audio_result`.
fn extract_audio(
    tools: &ToolPaths,
    config: &Config,
    file: &Path,
    dest: &Path,
    ranges: &[(f64, f64)],
    cancel: &AtomicBool,
) -> Result<bool, String> {
    let ffmpeg = tools.ffmpeg_path().ok_or("ffmpeg was not found")?;
    let has_stream = has_audio_stream(tools, file);
    if has_stream == Some(false) {
        return Ok(false);
    }

    let args = audio_args(config, file, dest, ranges);

    // No deadline: this reads the whole capture, and how long that takes is
    // the disk's business. See `run_until_stopped`.
    let mut cmd = crate::run::command(ffmpeg);
    cmd.args(&args);
    let out = crate::run::run_until_stopped(cmd, cancel)
        .map_err(|e| format!("ffmpeg could not be started to read the sound ({e})"))?;
    if cancel.load(Ordering::Relaxed) {
        return Err(STOPPED.to_string());
    }
    let wav_bytes = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    audio_result(has_stream, &out, wav_bytes)
}

/// The ffmpeg arguments that write the export's sound to `dest`.
fn audio_args(config: &Config, file: &Path, dest: &Path, ranges: &[(f64, f64)]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-i".into(),
        file.display().to_string(),
        "-vn".into(),
    ];

    // The audio has to be cut to exactly the ranges the video keeps. Without
    // this a trimmed export carries the whole tape's audio, and because a MOV
    // is as long as its longest stream the result reads as the full duration
    // with two seconds of picture.
    // Copy the live channel to both sides, if that was asked for.
    //
    // It has to join whichever route the audio is already taking. -af and
    // -filter_complex cannot both apply to the same stream: adding -af
    // beside the trim graph made ffmpeg fail outright, and the export
    // carried on and produced a file with no sound at all, because an
    // absent audio track is a legitimate outcome here.
    let pan = config.fix_one_sided_audio.then(|| {
        let c = config.audio_source_channel.min(1);
        format!("pan=stereo|c0=c{c}|c1=c{c}")
    });
    // A home movie conformed to its nominal speed runs longer or shorter
    // than the tape, and the sound has to run with it. `atempo` changes the
    // speed and keeps the pitch; the alternative, resampling, would drop a
    // narrator's voice by a third of a semitone for the sake of 3%.
    let stretch = config.film_stretch();
    let tempo = ((stretch - 1.0).abs() >= 0.0005).then(|| format!("atempo={:.6}", 1.0 / stretch));
    let pan = match (pan, tempo) {
        (Some(p), Some(t)) => Some(format!("{p},{t}")),
        (p, t) => p.or(t),
    };

    if !ranges.is_empty() {
        let mut graph = String::new();
        for (i, (start, end)) in ranges.iter().enumerate() {
            graph.push_str(&format!(
                "[0:a]atrim=start={start}:end={end},asetpts=N/SR/TB[a{i}];"
            ));
        }
        for i in 0..ranges.len() {
            graph.push_str(&format!("[a{i}]"));
        }
        graph.push_str(&format!("concat=n={}:v=0:a=1", ranges.len()));
        // Chained after the concat, before the output label.
        if let Some(p) = &pan {
            graph.push(',');
            graph.push_str(p);
        }
        graph.push_str("[out]");
        args.push("-filter_complex".into());
        args.push(graph);
        args.push("-map".into());
        args.push("[out]".into());
    } else if let Some(p) = pan {
        args.push("-af".into());
        args.push(p);
    }

    args.extend([
        "-acodec".into(),
        "pcm_s16le".into(),
        "-f".into(),
        "wav".into(),
        "-y".into(),
        dest.display().to_string(),
    ]);
    args
}

/// Seconds of output the export should produce.
///
/// Measured in output time rather than frames, because the frame count changes
/// with the mode — double-rate doubles it, inverse telecine reduces it — while
/// the running time does not.
///
/// One mode does change it: a home movie conformed to its nominal speed. See
/// `Config::film_stretch`.
fn expected_output_seconds(config: &Config, source_seconds: f64, source_fps: f64) -> f64 {
    kept_source_seconds(config, source_seconds, source_fps) * config.film_stretch()
}

/// Seconds of the source the export keeps.
fn kept_source_seconds(config: &Config, source_seconds: f64, source_fps: f64) -> f64 {
    if config.trim_ranges.is_empty() {
        return source_seconds;
    }
    // Trim ranges count SOURCE frames, so they convert at the source rate.
    // Using the output rate here made a double-rate export expect half the
    // running time it actually produces.
    if source_fps <= 0.0 {
        return source_seconds;
    }
    let frames: i64 = config
        .trim_ranges
        .iter()
        .map(|(a, b)| (b - a + 1).max(0))
        .sum();
    frames as f64 / source_fps
}

/// The kept ranges as (start, end) seconds of source time.
fn trim_seconds(config: &Config, source_fps: f64) -> Vec<(f64, f64)> {
    if source_fps <= 0.0 {
        return Vec::new();
    }
    config
        .trim_ranges
        .iter()
        .map(|(a, b)| {
            // Inclusive frame range, so the end is the frame after the last.
            (*a as f64 / source_fps, (*b + 1) as f64 / source_fps)
        })
        .collect()
}



/// Build the FFmpeg command for the encode.
///
/// The video arrives on stdin from vspipe. Audio, when there is any, is added
/// as the **last** input so a logo watermark's `[1:v]` reference keeps working.
fn encode_command(
    ffmpeg: &Path,
    config: &Config,
    output: &Path,
    audio: Option<&Path>,
    progress_to_stdout: bool,
) -> (Command, bool) {
    let wm = watermark_args(config, crate::watermark_font().as_deref());
    let logo_inputs = wm.extra_inputs.clone();
    let has_logo = !logo_inputs.is_empty();
    let mut filter: VideoFilter = merge_video_filter(wm.filter, output_cs_tag(config));
    let keep_audio = audio.is_some();

    // A logo watermark uses -filter_complex, whose graph ends in an unlabeled
    // output. FFmpeg maps that automatically — but only while nothing else is
    // mapped explicitly, and naming an audio input forces explicit maps for
    // everything. Labelling the output here makes the logo and the sound
    // coexist; without it, a logo watermark silently produced a video with no
    // sound at all, because the audio was deferred to a mux that was never
    // written.
    //
    // Appended here rather than in watermark_args because that graph is
    // compared against the Python byte for byte.
    if has_logo && keep_audio {
        filter.graph.push_str("[vout]");
    }
    let encoder = output_args(config);

    let mut cmd = crate::run::command(ffmpeg);
    cmd.arg("-hide_banner");
    if progress_to_stdout {
        // stdout is free because the result goes to a file path.
        cmd.args(["-nostats", "-progress", "pipe:1"]);
    }
    cmd.args(["-i", "pipe:0"]);
    for arg in &logo_inputs {
        cmd.arg(arg);
    }

    let single_pass_audio = keep_audio;
    if let Some(path) = audio {
        cmd.args(["-i", &path.display().to_string()]);
    }

    for arg in filter.to_args() {
        cmd.arg(arg);
    }
    for arg in &encoder.args {
        cmd.arg(arg);
    }
    cmd.args(["-r", &output_frame_rate(config)]);

    if single_pass_audio {
        // Explicit maps, because naming an audio input disables FFmpeg's
        // automatic stream selection for video too. With a logo the video
        // comes from the filter graph and the audio is input 2, the logo
        // having taken input 1.
        if has_logo {
            cmd.args(["-map", "[vout]", "-map", "2:a:0"]);
        } else {
            cmd.args(["-map", "0:v:0", "-map", "1:a:0"]);
        }
        // Keep the sound lossless in a master. The audio is extracted as
        // 16-bit PCM, but with no -c:a the container default takes over and
        // .mov re-encoded it to 129 kbps AAC — lossy audio inside a ProRes
        // master nobody asked to be lossy. H.264 is the sharing format and
        // names its own AAC in the encoder args, so it is left alone.
        //
        // Set here rather than in the parity-verified output_args: the Python
        // has this bug and that gate exists to prove the port reproduces it,
        // not to grade it.
        if config.output_format != OutputFormat::H264 {
            cmd.args(["-c:a", "pcm_s16le"]);
        }
        // Make the picture authoritative and let the sound follow it.
        //
        // A container is as long as its longest stream, so a mismatch between
        // the two has to be resolved here or the file is silently padded.
        // `-shortest` alone resolved it the wrong way round: it ends the
        // output when the *audio* runs out, and a capture's audio is commonly
        // a fraction of a second shorter than its video — one AVI here is
        // 58.6586s of picture against 58.5000s of sound, and the export threw
        // away the last ten frames to match.
        //
        // `apad` makes the audio stream endless, so `-shortest` now ends the
        // output at the end of the video instead. Every captured frame is
        // kept and any shortfall in the sound becomes silence, which is both
        // the lesser loss and the one nobody has to notice.
        //
        // It also removes a failure that looked nothing like its cause: with
        // the audio ending first, FFmpeg exited while VapourSynth still had
        // frames to write, and the broken pipe that followed hung the export
        // outright until `pipe_with_progress` learned to let go of its end.
        // Longer audio than video is handled by the same pair — `apad` does
        // not truncate, and `-shortest` still stops at the video.
        //
        // Safe beside `-c:a pcm_s16le` above but not beside `-c:a copy`: a
        // filter cannot be inserted into a stream that is not being decoded.
        // Nothing here copies audio.
        cmd.args(["-apad", "-shortest"]);
    }

    // What was done to this file, written into the file.
    //
    // A restored tape outlives the machine that restored it and usually the
    // memory of how, and the one question anybody asks of it a year later is
    // what was done. A sidecar answers that only until someone moves the
    // file; a tag travels with it, survives copying and re-muxing, and reads
    // in MediaInfo or ffprobe without this program.
    //
    // `comment` and `description` because between them they cover what the
    // three containers here actually surface: MOV and MP4 show description,
    // MKV shows comment. Writing both is two short strings against the
    // certainty of neither being visible.
    let note = vcg_core::provenance::summary(config, vcg_core::VERSION);
    cmd.args(["-metadata", &format!("comment={note}")]);
    cmd.args(["-metadata", &format!("description={note}")]);
    cmd.args([
        "-metadata",
        &format!("encoder=VCG Deinterlacer {}", vcg_core::VERSION),
    ]);

    // What the recording *is*, if the user said. Separate tags from the
    // provenance note above: that one says what this program did, these say
    // what somebody pointed a camera at, and a search for "Christmas 1998"
    // should not have to wade through deinterlacer settings to find it.
    //
    // `title` rather than overwriting `description`, so both survive.
    if !config.meta_description.trim().is_empty() {
        cmd.args([
            "-metadata",
            &format!("title={}", config.meta_description.trim()),
        ]);
    }
    // `creation_time` is the tag players and file browsers actually show as
    // the recording date, and ffmpeg wants it as ISO 8601. A bare year or
    // year-month is padded rather than rejected: somebody who knows it was
    // the summer of 1998 and no more than that should not be forced to
    // invent a day.
    if let Some(iso) = iso_date(&config.meta_date) {
        cmd.args(["-metadata", &format!("creation_time={iso}")]);
        cmd.args(["-metadata", &format!("date={}", config.meta_date.trim())]);
    }
    if !config.meta_location.trim().is_empty() {
        cmd.args([
            "-metadata",
            &format!("location={}", config.meta_location.trim()),
        ]);
        // MP4 and MOV read the Apple-style key; MKV reads the plain one.
        cmd.args([
            "-metadata",
            &format!("com.apple.quicktime.location.ISO6709={}", config.meta_location.trim()),
        ]);
    }

    cmd.args(["-y", &output.display().to_string()]);
    (cmd, single_pass_audio)
}

/// Everything one export needs.
///
/// A struct rather than eight positional parameters: two of them are an f64
/// duration and an f64 frame rate, which is exactly the pair a caller would
/// eventually transpose.
pub struct Request<'a> {
    pub tools: &'a ToolPaths,
    pub deps: &'a DepsPaths,
    pub config: &'a Config,
    pub source: &'a Path,
    /// Running time of the source, in seconds.
    pub source_seconds: f64,
    /// Frame rate of the source. Trim ranges count source frames.
    pub source_fps: f64,
    pub output: &'a Path,
    /// Also write a short before/after clip beside the finished video.
    pub comparison: bool,
    /// Set from the interface to stop the render.
    ///
    /// Checked between progress blocks, which arrive about twice a second,
    /// so a stop takes effect in well under a second on a job that may have
    /// hours left to run. Shared rather than passed by value because the
    /// thread doing the work is not the one that can see the button.
    pub cancel: Arc<AtomicBool>,
}

/// Run the export.
pub fn run(request: Request<'_>, progress: Option<Progress<'_>>) -> Result<Outcome, String> {
    let Request {
        tools,
        deps,
        config,
        source,
        source_seconds,
        source_fps,
        output,
        comparison,
        cancel,
    } = request;
    let mut discard = |_: Phase| {};
    let report: &mut dyn FnMut(Phase) = match progress {
        Some(p) => p,
        None => &mut discard,
    };

    let vspipe = tools
        .vspipe
        .as_ref()
        .ok_or("VapourSynth was not found")?
        .path
        .clone();
    let ffmpeg = tools
        .ffmpeg_path()
        .ok_or("ffmpeg was not found")?
        .to_path_buf();

    // ── Audio ───────────────────────────────────────────────────────────────
    report(Phase::ExtractingAudio);
    let audio_path = std::env::temp_dir().join(format!("vcg-audio-{}.wav", std::process::id()));
    let ranges = trim_seconds(config, source_fps);
    let had_audio = match extract_audio(tools, config, source, &audio_path, &ranges, &cancel) {
        Ok(found) => found,
        Err(why) => {
            let _ = std::fs::remove_file(&audio_path);
            return Err(why);
        }
    };

    // ── Script ──────────────────────────────────────────────────────────────
    let mut config = config.clone();
    // The note written into the file says what was done, not what was asked
    // for: with no sound there was no channel to repair.
    config.fix_one_sided_audio &= had_audio;
    // The script reads its source from the config, so it is set from the file
    // being exported rather than trusted to already match.
    config.input_path = source.display().to_string();
    let script = generate_vpy(&config, deps);

    let dir = std::env::temp_dir().join("vcg-export");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let script_path = dir.join(format!("export-{}.vpy", std::process::id()));
    std::fs::write(&script_path, script.as_bytes()).map_err(|e| e.to_string())?;

    // ── Encode ──────────────────────────────────────────────────────────────
    let total_seconds = expected_output_seconds(&config, source_seconds, source_fps);

    let mut producer = crate::run::command(&vspipe);
    producer.args([
        "-c".to_string(),
        "y4m".to_string(),
        script_path.display().to_string(),
        "-".to_string(),
    ]);
    for key in tools.vspipe_env_removals() {
        producer.env_remove(key);
    }
    for (key, value) in tools.vspipe_env() {
        producer.env(key, value);
    }
    // Windows resolves a plugin's own dependent DLLs relative to the working
    // directory, so the VapourSynth folder is the right place to start from.
    if let Some(vs) = &tools.vs_deps_dir {
        producer.current_dir(vs);
    }

    let (consumer, audio_in_pass) = encode_command(
        &ffmpeg,
        &config,
        output,
        if had_audio { Some(&audio_path) } else { None },
        true,
    );

    if config.rebuilds_film() {
        report(Phase::FindingFilmFrames);
    }
    let result = pipe_with_progress(producer, consumer, total_seconds, report, &cancel);
    let _ = std::fs::remove_file(&script_path);

    let stderr = match result {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&audio_path);
            // A stopped render leaves a short, playable file behind. It
            // looks like a finished export and it is not one, so it goes
            // with the job rather than being left for the user to find.
            if cancel.load(Ordering::Relaxed) {
                let _ = std::fs::remove_file(output);
            }
            return Err(e);
        }
    };

    // A frame VapourSynth could not make ends the video there, and nothing
    // after this notices: ffmpeg is handed a stream that simply stops, writes
    // what it got, and exits cleanly; the sound is padded to full length, so
    // the file's duration is right too. An export that failed on frame 0
    // passed every check below as a minute of black with sound.
    if let Some(why) = frame_failure(&stderr.producer) {
        let _ = std::fs::remove_file(&audio_path);
        return Err(format!(
            "VapourSynth stopped part way: {why}\nThe file that was written is incomplete.\n{}",
            stderr.report()
        ));
    }

    if !output.exists() {
        let _ = std::fs::remove_file(&audio_path);
        return Err(format!(
            "no output file was produced.\n{}",
            stderr.report()
        ));
    }

    // ── Verify ──────────────────────────────────────────────────────────────
    report(Phase::Verifying);
    let duration = probe_duration(tools, output).ok_or_else(|| {
        format!(
            "the output could not be read back — it is probably truncated.\n{}",
            stderr.report()
        )
    })?;

    // A render that stops early leaves a playable but short file, which is the
    // worst kind of failure: it looks like success.
    if total_seconds > 0.0 {
        let tolerance = (total_seconds * 0.02).max(0.5);
        if (duration - total_seconds).abs() > tolerance {
            let _ = std::fs::remove_file(&audio_path);
            return Err(format!(
                "the output is {duration:.1}s but should be {total_seconds:.1}s — \
                 the render did not finish.\n{}",
                stderr.report()
            ));
        }
    }

    let _ = std::fs::remove_file(&audio_path);

    // ── The optional side-by-side clip ──────────────────────────────────────
    //
    // Last, and never fatal. The video is written and verified by this
    // point, and losing an hour's render because a bonus file would not
    // encode would be absurd. A failure is appended to the tool output,
    // where the log and the details panel will show it.
    let mut stderr = stderr;
    let comparison_file = if comparison && !cancel.load(Ordering::Relaxed) {
        report(Phase::Comparing);
        match crate::comparison::render(tools, &config, source, output, source_fps) {
            Ok(path) => Some(path),
            Err(why) => {
                // Onto the consumer's half: this is an ffmpeg render of its
                // own, with no VapourSynth in it.
                stderr
                    .consumer
                    .push_str(&format!("\ncomparison clip: {why}\n"));
                None
            }
        }
    } else {
        None
    };

    Ok(Outcome {
        output: output.to_path_buf(),
        duration,
        // Audio only made it in if it was muxed during the pass.
        had_audio: had_audio && audio_in_pass,
        comparison: comparison_file,
        tool_output: tail(&stderr.all(), 20_000),
    })
}

/// Read the finished file's duration.
fn probe_duration(tools: &ToolPaths, file: &Path) -> Option<f64> {
    let ffprobe = tools.ffprobe_path()?;
    let out = run_tool(
        ffprobe,
        [
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            &file.display().to_string(),
        ],
    )
    .ok()?;
    out.stdout.trim().parse().ok()
}

/// vspipe's report of a frame it could not make, if it made one.
///
/// vspipe exits on the first such frame and prints this line; the frames
/// before it have already gone down the pipe.
fn frame_failure(producer_stderr: &str) -> Option<String> {
    producer_stderr
        .lines()
        .map(str::trim)
        .find(|l| l.contains("Failed to retrieve frame"))
        .map(|l| l.trim_start_matches("Error:").trim().to_string())
}

#[cfg(test)]
mod frame_failure_tests {
    use super::frame_failure;

    /// The line from the export that came out black: an odd-width resize in
    /// the dust filter failed on the first frame.
    #[test]
    fn a_frame_vapoursynth_could_not_make_is_a_failure() {
        let stderr = "Warning: Plugin x.dll is using API3\n\n\
            Error: Failed to retrieve frame 0 with error: Resize error 1027: \
            image dimensions must be divisible by subsampling factor\n\
            Output 28 frames in 1.69 seconds (16.54 fps)\n";
        let why = frame_failure(stderr).expect("a failure");
        assert!(why.starts_with("Failed to retrieve frame 0"), "{why}");
        assert!(why.contains("divisible by subsampling factor"), "{why}");
    }

    #[test]
    fn a_clean_render_is_not() {
        let stderr = "Warning: Plugin x.dll is using API3\n\
            Critical: Explicitly instantiated a Cache with make_linear set.\n\
            Output 1108 frames in 65.90 seconds (16.81 fps)\n";
        assert_eq!(frame_failure(stderr), None);
    }
}

/// What each half of the pipe said.
///
/// Kept apart rather than concatenated. They used to be joined into one
/// string, producer first, and every error message then trimmed it to its
/// last 1200 characters — so ffmpeg's "Invalid data found when processing
/// input" survived and the VapourSynth error that *caused* it was the half
/// discarded. The user was shown the symptom and never the reason.
#[derive(Debug, Default, Clone)]
pub struct PipeStderr {
    /// vspipe's stderr. VapourSynth reports script errors here, so this is
    /// the half that explains a failed export.
    pub producer: String,
    /// ffmpeg's stderr.
    pub consumer: String,
}

impl PipeStderr {
    /// Both halves, labelled and separately trimmed, for an error message.
    ///
    /// The producer is named first and given the larger share, because when
    /// the pipe carries nothing ffmpeg's complaint is a consequence rather
    /// than a reason. Each is trimmed from its own end: a VapourSynth failure
    /// finishes with the `vapoursynth.Error` line, which is what a traceback
    /// is read backwards for.
    fn report(&self) -> String {
        let mut out = String::new();
        let producer = tail(&self.producer, 1600);
        if !producer.is_empty() {
            out.push_str("VapourSynth said:\n");
            out.push_str(&producer);
        }
        let consumer = tail(&self.consumer, 800);
        if !consumer.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str("FFmpeg said:\n");
            out.push_str(&consumer);
        }
        if out.is_empty() {
            out.push_str("Neither program reported anything.");
        }
        out
    }

    /// Everything both said, for the log, where there is room.
    fn all(&self) -> String {
        format!(
            "--- vspipe ---\n{}\n\n--- ffmpeg ---\n{}",
            self.producer.trim(),
            self.consumer.trim()
        )
    }
}

/// Wait for a child, killing it if it has not gone within `grace`.
///
/// `Child::wait` has no deadline, and a bare one at the end of an export is
/// not a safe default: the pair being waited on are two processes joined by a
/// pipe, and either can be left blocked on the other. Nothing reads the
/// cancel flag once the progress loop has ended, so a wait that cannot be
/// bounded is a hang the user cannot get out of.
fn wait_or_kill(child: &mut std::process::Child, grace: Duration) {
    let deadline = Instant::now() + grace;
    loop {
        // An unwaitable child counts as gone rather than as a reason to spin.
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Spawn the pair and stream FFmpeg's progress until it finishes.
fn pipe_with_progress(
    mut producer: Command,
    mut consumer: Command,
    total_seconds: f64,
    report: &mut dyn FnMut(Phase),
    cancel: &AtomicBool,
) -> Result<PipeStderr, String> {
    producer
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut prod = producer
        .spawn()
        .map_err(|e| format!("could not start VapourSynth: {e}"))?;
    let prod_out = prod.stdout.take().expect("stdout is piped");
    let prod_err = crate::run::drain_to_string(prod.stderr.take().expect("stderr is piped"));

    consumer
        .stdin(Stdio::from(prod_out))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut cons = match consumer.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = prod.kill();
            let _ = prod.wait();
            return Err(format!("could not start ffmpeg: {e}"));
        }
    };
    // Let go of the pipe.
    //
    // `consumer` still holds the read end that was handed to it as stdin —
    // `spawn` duplicates the handle for the child and leaves the original in
    // the `Command`. While this function owns that `Command`, *this* process
    // is a second reader of the pipe, so when ffmpeg exits vspipe does not
    // see a broken pipe: it blocks in a write that can never complete, and
    // the `prod.wait()` below never returns. The export then hangs after the
    // video is finished and correct, with no log written and nothing left
    // checking the cancel flag, so "force end" has nothing to act on either.
    //
    // `-apad` beside `-shortest` in `encode_command` means ffmpeg no longer
    // ends on the audio, so it should now read its input to the end. That
    // makes this a belt-and-braces measure rather than the live fix it was,
    // and it stays either way: ffmpeg stopping early for any other reason —
    // an encoder failing, a full disk, a kill from outside — puts the pair
    // back in exactly this position.
    drop(consumer);

    let cons_err = crate::run::drain_to_string(cons.stderr.take().expect("stderr is piped"));
    let stdout = cons.stdout.take().expect("stdout is piped");

    // `-progress pipe:1` emits key=value lines, ending each block with
    // progress=continue or progress=end.
    let mut out_seconds = 0.0f64;
    let mut fps = 0.0f64;
    let mut speed = 0.0f64;
    let mut stopped = false;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        // Checked per line rather than per progress block: the blocks are
        // half a second apart and the lines inside one are instant, so this
        // is the most responsive place that still never blocks.
        //
        // Both children are killed, and in this order. Killing ffmpeg first
        // would leave vspipe writing into a closed pipe, where on Windows it
        // can sit in a blocked write rather than noticing; killing the
        // producer first closes the pipe ffmpeg is reading and it ends on
        // its own, with the kill below only as a backstop.
        if cancel.load(Ordering::Relaxed) {
            stopped = true;
            let _ = prod.kill();
            let _ = cons.kill();
            break;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "out_time_us" | "out_time_ms" => {
                if let Ok(v) = value.trim().parse::<f64>() {
                    // out_time_ms is misnamed upstream: it is microseconds.
                    out_seconds = v / 1_000_000.0;
                }
            }
            "fps" => fps = value.trim().parse().unwrap_or(fps),
            "speed" => {
                speed = value
                    .trim()
                    .trim_end_matches('x')
                    .parse()
                    .unwrap_or(speed);
            }
            "progress" => {
                report(Phase::Encoding {
                    out_seconds,
                    total_seconds,
                    fps,
                    speed,
                });
            }
            _ => {}
        }
    }

    // ffmpeg's stdout is at EOF, so it has written its trailer and is on its
    // way out; the grace is for a large file's final flush.
    wait_or_kill(&mut cons, Duration::from_secs(30));
    // vspipe has no reason to outlive the program reading it, and by here the
    // output file is already written. `drop(consumer)` above is what lets it
    // notice; this is the backstop, because a bare `wait()` here is a hang
    // with no way out — the loop above is the only place the cancel flag is
    // read, and it has ended.
    wait_or_kill(&mut prod, Duration::from_secs(10));

    let producer_stderr = prod_err.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let consumer_stderr = cons_err.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = NO_DEADLINE;

    if stopped {
        // An `Err` with wording the page can show as it stands. Stopping is
        // not a fault and the message must not read like one, but it is not
        // a success either: there is no finished file to point at.
        return Err(STOPPED.to_string());
    }

    Ok(PipeStderr {
        producer: producer_stderr,
        consumer: consumer_stderr,
    })
}

/// The last `n` characters, for an error message.
fn tail(text: &str, n: usize) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= n {
        return trimmed.to_string();
    }
    let start = trimmed.len() - n;
    // Do not split a multi-byte character.
    let start = trimmed
        .char_indices()
        .map(|(i, _)| i)
        .find(|i| *i >= start)
        .unwrap_or(0);
    format!("…{}", &trimmed[start..])
}

/// A user-typed date as ffmpeg's `creation_time` wants it.
///
/// Accepts `YYYY`, `YYYY-MM` and `YYYY-MM-DD`, padding the missing parts,
/// and returns `None` for anything else rather than guessing. A tape found
/// in a loft often has a year written on it and nothing more, and refusing
/// that would mean the field goes unused by exactly the people it is for.
///
/// Deliberately not a date library: this validates four digits and two
/// optional two-digit groups, which is the whole of the problem.
fn iso_date(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    let parts: Vec<&str> = t.split('-').collect();
    if parts.len() > 3 || !parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let year: u32 = parts.first()?.parse().ok()?;
    if parts[0].len() != 4 || !(1900..=2200).contains(&year) {
        return None;
    }
    let month: u32 = parts.get(1).map_or(Some(1), |p| p.parse().ok())?;
    let day: u32 = parts.get(2).map_or(Some(1), |p| p.parse().ok())?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}T00:00:00Z"))
}

#[cfg(test)]
mod meta_tests {
    use super::iso_date;

    #[test]
    fn a_bare_year_is_enough() {
        assert_eq!(iso_date("1998").as_deref(), Some("1998-01-01T00:00:00Z"));
        assert_eq!(iso_date("1998-08").as_deref(), Some("1998-08-01T00:00:00Z"));
        assert_eq!(
            iso_date("1998-08-09").as_deref(),
            Some("1998-08-09T00:00:00Z")
        );
    }

    #[test]
    fn nonsense_is_left_out_rather_than_guessed_at() {
        for bad in ["", "   ", "soon", "98", "1998/08/09", "1998-13-01", "1998-08-32", "20255"] {
            assert_eq!(iso_date(bad), None, "{bad:?} should not produce a date");
        }
    }

    #[test]
    fn whitespace_around_a_good_date_is_fine() {
        assert_eq!(iso_date("  2001-05-04 ").as_deref(), Some("2001-05-04T00:00:00Z"));
    }
}

#[cfg(test)]
mod film_clock_tests {
    use super::{audio_args, expected_output_seconds};
    use std::path::Path;
    use vcg_core::config::{Config, FilmOutput, FilmRate};

    fn film(output: FilmOutput) -> Config {
        Config {
            home_movie: true,
            home_movie_rate: FilmRate::Fps18,
            home_movie_fps: Some(18.61),
            home_movie_output: output,
            ..Config::default()
        }
    }

    fn filter(config: &Config, ranges: &[(f64, f64)]) -> String {
        let args = audio_args(config, Path::new("in.avi"), Path::new("out.wav"), ranges);
        let flag = if ranges.is_empty() { "-af" } else { "-filter_complex" };
        args.iter()
            .position(|a| a == flag)
            .map(|i| args[i + 1].clone())
            .unwrap_or_default()
    }

    /// Two of the three clocks leave the running time alone, and the export
    /// must not expect otherwise or touch the sound.
    #[test]
    fn the_measured_and_video_clocks_keep_the_tapes_running_time() {
        for output in [FilmOutput::Measured, FilmOutput::Video] {
            let c = film(output);
            assert_eq!(expected_output_seconds(&c, 689.0, 29.97), 689.0, "{output:?}");
            assert!(!filter(&c, &[]).contains("atempo"), "{output:?}");
        }
    }

    /// A reel projected at 18.61 and written at 18 runs 18.61/18 as long.
    /// The verification that follows an export rejects anything 2% off its
    /// expected length, so without this a conformed export fails as
    /// "did not finish" — and the sound has to be slowed by the same amount
    /// or it ends 23 seconds before the picture does.
    #[test]
    fn conforming_stretches_the_expected_length_and_the_sound_together() {
        let c = film(FilmOutput::Conform);
        let expected = expected_output_seconds(&c, 689.0, 29.97);
        assert!((expected - 689.0 * 18.61 / 18.0).abs() < 1e-6, "{expected}");
        assert!(filter(&c, &[]).contains("atempo=0.967222"), "{}", filter(&c, &[]));

        // With a trim, after the pieces are joined; with the channel repair,
        // in the same chain rather than a second -af that ffmpeg would refuse.
        let trimmed = filter(&c, &[(10.0, 20.0)]);
        assert!(trimmed.contains("concat=n=1:v=0:a=1,atempo=0.967222[out]"), "{trimmed}");
        let both = Config { fix_one_sided_audio: true, ..c.clone() };
        assert!(filter(&both, &[]).contains("pan=stereo|c0=c0|c1=c0,atempo=0.967222"));
        let kept = expected_output_seconds(
            &Config { trim_ranges: vec![(0, 2996)], ..c },
            689.0,
            29.97,
        );
        assert!((kept - 100.0 * 18.61 / 18.0).abs() < 1e-6, "{kept}");
    }

    #[test]
    fn nothing_but_a_home_movie_is_ever_stretched() {
        let video = Config { home_movie: false, ..film(FilmOutput::Conform) };
        assert_eq!(video.film_stretch(), 1.0);
        assert!(!filter(&video, &[]).contains("atempo"));
        // A reel that ran at its nominal speed has nothing to conform.
        let exact = Config { home_movie_fps: None, ..film(FilmOutput::Conform) };
        assert_eq!(exact.film_stretch(), 1.0);
        assert!(!filter(&exact, &[]).contains("atempo"));
    }
}

#[cfg(test)]
mod audio_tests {
    use super::audio_result;
    use crate::run::Output;

    fn out(status: Option<i32>, stderr: &str, killed: bool) -> Output {
        Output {
            status,
            stdout: String::new(),
            stderr: stderr.to_string(),
            timed_out: killed,
        }
    }

    #[test]
    fn a_wav_that_was_written_is_sound() {
        assert_eq!(audio_result(Some(true), &out(Some(0), "", false), 180_000_000), Ok(true));
        assert_eq!(audio_result(None, &out(Some(0), "", false), 180_000_000), Ok(true));
    }

    #[test]
    fn a_bare_header_is_an_empty_track_not_a_failure() {
        assert_eq!(audio_result(Some(true), &out(Some(0), "", false), 78), Ok(false));
    }

    /// The one that shipped: the extraction was killed part-way through a
    /// tape with sound, and the export carried on as if the tape were silent.
    #[test]
    fn a_killed_extraction_of_a_tape_with_sound_is_an_error() {
        let killed = out(None, "size=  60000KiB time=00:05:20.00", true);
        assert!(audio_result(Some(true), &killed, 61_000_000).is_err());
        // Not knowing whether there was a track is no reason to assume not.
        assert!(audio_result(None, &killed, 61_000_000).is_err());
    }

    #[test]
    fn a_failed_extraction_of_a_tape_with_sound_is_an_error_that_quotes_ffmpeg() {
        let failed = out(Some(1), "Error opening output file: Permission denied", false);
        let why = audio_result(Some(true), &failed, 0).unwrap_err();
        assert!(why.contains("Permission denied"), "{why}");
    }

    #[test]
    fn a_source_with_no_sound_track_is_still_not_a_failure() {
        let none = out(Some(1), "Output file does not contain any stream", false);
        assert_eq!(audio_result(Some(false), &none, 0), Ok(false));
        // ffprobe could not say, so ffmpeg's own word is taken for it…
        assert_eq!(audio_result(None, &none, 0), Ok(false));
        let trimmed = out(Some(1), "Stream specifier ':a' in filtergraph matches no streams.", false);
        assert_eq!(audio_result(None, &trimmed, 0), Ok(false));
        // …but not against ffprobe, which counted a track.
        assert!(audio_result(Some(true), &none, 0).is_err());
    }
}

#[cfg(test)]
mod pipe_tests {
    use super::*;

    /// A shell, because these tests need two real processes joined by a pipe
    /// and the system shell is the only program guaranteed to be present.
    #[cfg(windows)]
    fn shell(script: &str) -> Command {
        let mut c = crate::run::command("cmd");
        c.args(["/c", script]);
        c
    }

    #[cfg(not(windows))]
    fn shell(script: &str) -> Command {
        let mut c = crate::run::command("sh");
        c.args(["-c", script]);
        c
    }

    /// A consumer that stops reading must not be able to hang the export.
    /// It is how this bug first arrived — `-shortest` ended ffmpeg as soon as
    /// the audio ran out, which on a capture whose sound is a fraction of a
    /// second short of its picture is every time — and `-apad` has since
    /// removed that particular cause. Any other early exit (a failing
    /// encoder, a full disk, a kill from outside) leaves the producer with
    /// frames still in hand in the same way, so the reaping is what is tested
    /// here rather than the reason for it.
    ///
    /// Without the `drop(consumer)` in `pipe_with_progress` this never
    /// returns: the read end of the pipe stays open in this process, so the
    /// producer's write never fails and the wait never ends.
    #[test]
    fn a_producer_left_writing_into_a_dead_consumer_is_reaped() {
        // Far more than a pipe buffer holds, so the producer is certain to
        // still be writing when the consumer goes.
        #[cfg(windows)]
        let feed = "for /L %i in (1,1,20000) do @echo \
                    xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
        #[cfg(not(windows))]
        let feed = "for i in $(seq 1 20000); do echo \
                    xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; done";

        let started = Instant::now();
        let mut seen = 0usize;
        let mut report = |_: Phase| seen += 1;
        let result = pipe_with_progress(
            shell(feed),
            // Exits at once, having read nothing.
            shell("exit 0"),
            1.0,
            &mut report,
            &AtomicBool::new(false),
        );
        let took = started.elapsed();

        assert!(result.is_ok(), "{result:?}");
        assert!(
            took < Duration::from_secs(20),
            "took {took:?} — the producer was left blocked on a write"
        );
    }

    /// The other half of the same promise: a producer that ends on its own
    /// still returns promptly, and its stderr comes back.
    #[test]
    fn a_normal_end_returns_what_both_halves_said() {
        let mut report = |_: Phase| {};
        let stderr = pipe_with_progress(
            shell("echo to-stderr 1>&2"),
            shell("exit 0"),
            1.0,
            &mut report,
            &AtomicBool::new(false),
        )
        .expect("the pair ran");
        assert!(
            stderr.producer.contains("to-stderr"),
            "producer stderr was {:?}",
            stderr.producer
        );
    }
}
