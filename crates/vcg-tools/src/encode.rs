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
use std::time::Duration;

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
            Phase::ExtractingAudio => 0.0,
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

/// Extract the source audio to a temporary WAV.
///
/// Decoded rather than copied, because the source codec may not be legal in
/// the output container and because a tape capture's audio is small next to
/// its video.
fn extract_audio(
    tools: &ToolPaths,
    config: &Config,
    file: &Path,
    dest: &Path,
    ranges: &[(f64, f64)],
) -> bool {
    let Some(ffmpeg) = tools.ffmpeg_path() else {
        return false;
    };

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

    let out = run_tool(ffmpeg, &args);
    // A source with no audio track is normal, not a failure.
    matches!(out, Ok(o) if o.success())
        && std::fs::metadata(dest).map(|m| m.len() > 1000).unwrap_or(false)
}

/// Seconds of output the export should produce.
///
/// Measured in output time rather than frames, because the frame count changes
/// with the mode — double-rate doubles it, inverse telecine reduces it — while
/// the running time does not.
fn expected_output_seconds(config: &Config, source_seconds: f64, source_fps: f64) -> f64 {
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
    cmd.args(["-r", output_frame_rate(config)]);

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
        // A container is as long as its longest stream, so any residual
        // mismatch would silently pad the file rather than fail.
        cmd.arg("-shortest");
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
    let had_audio = extract_audio(tools, config, source, &audio_path, &ranges);

    // ── Script ──────────────────────────────────────────────────────────────
    let mut config = config.clone();
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

    if !output.exists() {
        let _ = std::fs::remove_file(&audio_path);
        return Err(format!(
            "no output file was produced.\n{}",
            tail(&stderr, 1200)
        ));
    }

    // ── Verify ──────────────────────────────────────────────────────────────
    report(Phase::Verifying);
    let duration = probe_duration(tools, output).ok_or_else(|| {
        format!(
            "the output could not be read back — it is probably truncated.\n{}",
            tail(&stderr, 1200)
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
                tail(&stderr, 1200)
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
                stderr.push_str(&format!("\ncomparison clip: {why}\n"));
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
        tool_output: tail(&stderr, 20_000),
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

/// Spawn the pair and stream FFmpeg's progress until it finishes.
///
/// Returns the producer's stderr, which is where VapourSynth reports script
/// errors — the ones worth showing when an export fails.
fn pipe_with_progress(
    mut producer: Command,
    mut consumer: Command,
    total_seconds: f64,
    report: &mut dyn FnMut(Phase),
    cancel: &AtomicBool,
) -> Result<String, String> {
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

    let _ = cons.wait();
    let _ = prod.wait();

    let producer_stderr = prod_err.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let consumer_stderr = cons_err.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = NO_DEADLINE;

    if stopped {
        // An `Err` with wording the page can show as it stands. Stopping is
        // not a fault and the message must not read like one, but it is not
        // a success either: there is no finished file to point at.
        return Err(STOPPED.to_string());
    }

    Ok(format!("{producer_stderr}\n{consumer_stderr}"))
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
