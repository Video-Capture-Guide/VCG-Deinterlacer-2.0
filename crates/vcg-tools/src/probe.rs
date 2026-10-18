//! Running the analysis probes against a real file.
//!
//! The thresholds and parsing live in `vcg_core::analysis`, verified against
//! the Python. This module only decides *which* ffmpeg commands to run and
//! hands their output to those parsers.
//!
//! Three places do less work than the Python, with no change in result:
//!
//! * **One ffprobe call, not four.** `get_video_info`, `detect_par_format` and
//!   `classify_source` each spawn their own ffprobe, and three of the probes
//!   spawn a fourth just for the duration. `-select_streams v:0 -show_format
//!   -show_streams` satisfies all of them at once.
//! * **Levels and color share their samples.** Both parse the same
//!   signalstats text from the same timestamps, so the Python's two sets of
//!   ten identical ffmpeg runs become one set of ten.
//! * **The halo probe writes no files.** The Python saves a PNG per sample and
//!   opens it with Pillow; asking ffmpeg for a raw gray plane on stdout skips
//!   the disk round-trip and the dependency. See `halo` for the one caveat.

use std::path::Path;
use std::time::Duration;

use vcg_core::analysis::classify::{
    bleeding, color_data, levels, noise, BleedingAnalysis, ColorData, Levels, NoiseAnalysis,
};
use vcg_core::analysis::halo::{self, EdgeTally, HaloAnalysis, LumaPlane};
use vcg_core::analysis::idet::{self, IdetCounts, Telecine};
use vcg_core::analysis::probe::{self, Classification, ParFormat, VideoInfo};
use vcg_core::analysis::signalstats::{interior_sample_times, mid_sample_offsets, SignalStatsRun};
use vcg_core::analysis::audio::{self, AudioAnalysis};
use vcg_core::analysis::edges::Detection;
use vcg_core::analysis::histogram::LumaHistogram;
use vcg_core::config::{SourceClass, VideoFormat};

use crate::paths::ToolPaths;
use crate::run::{run_tool, run_tool_binary, DEFAULT_TIMEOUT};

/// Sample counts, matching the Python's per-probe defaults.
const LEVELS_COLOR_SAMPLES: u32 = 10;
const NOISE_SAMPLES: u32 = 15;
const BLEEDING_SAMPLES: u32 = 10;
const HALO_SAMPLES: u32 = 8;

/// A telecine scan can read several 240-frame windows, so it gets longer.
const IDET_TIMEOUT: Duration = Duration::from_secs(90);

/// Which probes to run.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Telecine detection reads 240-frame windows and is the slowest probe.
    pub telecine: bool,
    pub halo: bool,
    pub noise: bool,
    pub bleeding: bool,
    pub levels_color: bool,
    /// Channel count and per-channel levels, for the one-sided-sound check.
    pub audio: bool,
    /// Where the picture actually starts at each frame edge.
    ///
    /// Runs in every mode, unlike the enhancement probes: a black bar left in
    /// the output is a defect, not a missing improvement, and cropping is on
    /// in every mode for exactly that reason.
    pub edges: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            telecine: true,
            halo: true,
            noise: true,
            bleeding: true,
            levels_color: true,
            edges: true,
            audio: true,
        }
    }
}

impl Options {
    /// The container read only: no frames are decoded.
    ///
    /// Used the moment a file is chosen, so its size, duration, resolution and
    /// scan type can be shown before the real analysis runs.
    pub fn container_only() -> Self {
        Self {
            telecine: false,
            halo: false,
            noise: false,
            bleeding: false,
            levels_color: false,
            edges: false,
            audio: false,
        }
    }

    /// Just the cheap probes — one ffprobe call and a handful of single frames.
    pub fn quick() -> Self {
        Self {
            telecine: false,
            halo: false,
            noise: false,
            bleeding: true,
            levels_color: true,
            edges: true,
            audio: true,
        }
    }
}

/// Everything the probes found. Each field is `None` if that probe was
/// skipped or could not run.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub info: Option<VideoInfo>,
    pub classification: Option<Classification>,
    pub par: Option<ParFormat>,
    pub levels: Option<Levels>,
    pub color: Option<ColorData>,
    pub noise: Option<NoiseAnalysis>,
    pub bleeding: Option<BleedingAnalysis>,
    pub halo: Option<HaloAnalysis>,
    pub telecine: Option<Telecine>,
    /// Where the picture starts at each edge. `Inconclusive` is a real answer
    /// here, not a failure: it means leave the chosen preset alone.
    pub edges: Option<Detection>,
    /// What the sound track is, and whether a side of it is missing.
    pub audio: Option<AudioAnalysis>,
    /// Luma distribution, from the same frames the edge scan decoded.
    pub histogram: Option<LumaHistogram>,
    /// How many ffmpeg/ffprobe processes were spawned.
    pub spawns: u32,
    /// Probes that failed, with why, rather than silently reporting defaults.
    pub warnings: Vec<String>,
}

/// Progress callback: (probe name, completed, total).
pub type Progress<'a> = &'a mut dyn FnMut(&str, u32, u32);

struct Ctx<'a> {
    tools: &'a ToolPaths,
    file: &'a Path,
    report: Report,
}

impl Ctx<'_> {
    fn warn(&mut self, msg: impl Into<String>) {
        self.report.warnings.push(msg.into());
    }

    /// Run ffmpeg and return its stderr, where signalstats and idet write.
    fn ffmpeg_stderr(&mut self, args: &[String], timeout: Duration) -> Option<String> {
        let ffmpeg = self.tools.ffmpeg_path()?.to_path_buf();
        self.report.spawns += 1;
        let mut cmd = crate::run::command(&ffmpeg);
        cmd.args(args);
        match crate::run::run(cmd, timeout) {
            Ok(out) => {
                if out.timed_out {
                    self.warn(format!("ffmpeg timed out after {}s", timeout.as_secs()));
                }
                Some(out.stderr)
            }
            Err(e) => {
                self.warn(format!("ffmpeg could not be started: {e}"));
                None
            }
        }
    }
}

/// Build the shared signalstats argument list for a single frame at `t`.
fn signalstats_frame_args(file: &Path, t: f64) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-ss".into(),
        format!("{t}"),
        "-i".into(),
        file.display().to_string(),
        "-vframes".into(),
        "1".into(),
        "-vf".into(),
        "signalstats=stat=tout+vrep+brng,metadata=mode=print".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ]
}

/// Probe a file.
pub fn probe_file(
    tools: &ToolPaths,
    file: &Path,
    opts: Options,
    mut progress: Option<Progress<'_>>,
) -> Report {
    let mut ctx = Ctx { tools, file, report: Report::default() };

    let mut tick = |name: &str, done: u32, total: u32| {
        if let Some(p) = progress.as_deref_mut() {
            p(name, done, total);
        }
    };

    // ── One ffprobe call feeds info, classification and PAR detection ──────
    tick("probing container", 0, 1);
    let streams_json = match tools.ffprobe_path() {
        None => {
            ctx.warn("ffprobe was not found; nothing can be probed".to_string());
            return ctx.report;
        }
        Some(ffprobe) => {
            ctx.report.spawns += 1;
            match run_tool(
                ffprobe,
                [
                    "-v",
                    "quiet",
                    "-print_format",
                    "json",
                    "-select_streams",
                    "v:0",
                    "-show_format",
                    "-show_streams",
                    &file.display().to_string(),
                ],
            ) {
                Ok(out) if out.success() => out.stdout,
                Ok(out) => {
                    ctx.warn(format!(
                        "ffprobe exited with {:?}: {}",
                        out.status,
                        out.stderr.trim()
                    ));
                    out.stdout
                }
                Err(e) => {
                    ctx.warn(format!("ffprobe could not be started: {e}"));
                    return ctx.report;
                }
            }
        }
    };

    ctx.report.info = probe::video_info(&streams_json);
    ctx.report.classification = Some(probe::classify(&streams_json));
    ctx.report.par = Some(probe::par_format(&streams_json));
    tick("probing container", 1, 1);

    let Some(info) = ctx.report.info else {
        ctx.warn("no video stream was found; the remaining probes need one".to_string());
        return ctx.report;
    };
    // A duration of zero would collapse every sample onto timestamp 0.
    let duration = if info.duration > 0.0 {
        info.duration
    } else {
        ctx.warn("the container reports no duration; sampling from 60s assumed".to_string());
        60.0
    };

    // ── Frame edges: how much of the border is not picture ─────────────────
    //
    // Skipped for HD, where there is no analog blanking to find and the crop
    // is forced off anyway — on a 1440x1080 source the scan costs 7.6s for a
    // guaranteed answer of zero.
    if opts.edges {
        let hd = ctx.report.classification.as_ref().is_some_and(|c| {
            matches!(c.source_class, SourceClass::Avchd | SourceClass::Hdv)
        });
        if hd {
            ctx.report.edges = Some(Detection::Inconclusive(
                "an HD source has no analog blanking to crop",
            ));
        } else {
            tick("frame edges", 0, 1);
            ctx.report.spawns += crate::edges::FRAMES as u32;
            let (detection, histogram) =
                crate::edges::detect_crop_and_levels(tools, file, &info);
            ctx.report.edges = Some(detection);
            if !histogram.is_empty() {
                ctx.report.histogram = Some(histogram);
            }
            tick("frame edges", 1, 1);
        }
    }

    // ── Audio: channel count, then per-channel level ───────────────────────
    //
    // A separate ffprobe rather than widening the shared one: that call is
    // -select_streams v:0, and the parsers reading it are compared against
    // the Python byte for byte. One extra spawn is cheaper than disturbing
    // them.
    if opts.audio {
        tick("sound", 0, 1);
        let duration = ctx.report.info.as_ref().map(|i| i.duration).unwrap_or(0.0);
        ctx.report.audio = probe_audio(tools, file, duration, &mut ctx.report.spawns);
        tick("sound", 1, 1);
    }

    // ── Levels and color, from one shared set of samples ──────────────────
    if opts.levels_color {
        let times = interior_sample_times(duration, LEVELS_COLOR_SAMPLES);
        let mut runs = Vec::new();
        for (i, t) in times.iter().enumerate() {
            tick("levels / color", i as u32 + 1, LEVELS_COLOR_SAMPLES);
            let args = signalstats_frame_args(file, *t);
            if let Some(stderr) = ctx.ffmpeg_stderr(&args, DEFAULT_TIMEOUT) {
                runs.push(SignalStatsRun::parse(&stderr));
            }
        }
        if runs.is_empty() {
            ctx.warn("no frame could be measured for levels or color".to_string());
        } else {
            ctx.report.levels = Some(levels(&runs));
            ctx.report.color = color_data(&runs);
        }
    }

    // ── Noise: one second of video per sample ──────────────────────────────
    if opts.noise {
        let mut runs = Vec::new();
        for (i, offset) in mid_sample_offsets(NOISE_SAMPLES).iter().enumerate() {
            tick("noise", i as u32 + 1, NOISE_SAMPLES);
            let t = duration * offset;
            // +discardcorrupt stops an H.264 seek decode error from aborting
            // the run before signalstats measures anything.
            let args = vec![
                "-hide_banner".into(),
                "-fflags".into(),
                "+discardcorrupt".into(),
                "-ss".into(),
                format!("{t}"),
                "-i".into(),
                file.display().to_string(),
                "-t".into(),
                "1".into(),
                "-vf".into(),
                "signalstats=stat=tout+vrep+brng,metadata=mode=print".into(),
                "-f".into(),
                "null".into(),
                "-".into(),
            ];
            if let Some(stderr) = ctx.ffmpeg_stderr(&args, DEFAULT_TIMEOUT) {
                runs.push(SignalStatsRun::parse(&stderr));
            }
        }
        if !runs.is_empty() {
            ctx.report.noise = Some(noise(&runs));
        }
    }

    // ── Color bleeding ────────────────────────────────────────────────────
    if opts.bleeding {
        let mut runs = Vec::new();
        for (i, offset) in mid_sample_offsets(BLEEDING_SAMPLES).iter().enumerate() {
            tick("color bleeding", i as u32 + 1, BLEEDING_SAMPLES);
            let args = signalstats_frame_args(file, duration * offset);
            if let Some(stderr) = ctx.ffmpeg_stderr(&args, DEFAULT_TIMEOUT) {
                runs.push(SignalStatsRun::parse(&stderr));
            }
        }
        if !runs.is_empty() {
            ctx.report.bleeding = Some(bleeding(&runs));
        }
    }

    // ── Halo ───────────────────────────────────────────────────────────────
    if opts.halo {
        ctx.report.halo = Some(probe_halo(&mut ctx, duration, info, &mut tick));
    }

    // ── Telecine ───────────────────────────────────────────────────────────
    if opts.telecine {
        let format = ctx
            .report
            .par
            .as_ref()
            .and_then(|p| p.format)
            // Height is the fallback the PAR detection itself uses.
            .unwrap_or(if info.height == 576 {
                VideoFormat::Pal
            } else {
                VideoFormat::Ntsc
            });

        let plan = idet::sample_plan(duration, info.fps);
        let total = plan.len() as u32;
        let mut windows = Vec::new();
        for (i, w) in plan.iter().enumerate() {
            tick("telecine", i as u32 + 1, total);
            let mut args: Vec<String> = vec!["-hide_banner".into()];
            // Input-side seek is the fast one: keyframe-accurate rather than
            // exact, which is all a statistical sample needs.
            if w.start_seconds > 0.5 {
                args.push("-ss".into());
                args.push(format!("{:.2}", w.start_seconds));
            }
            args.extend([
                "-i".into(),
                file.display().to_string(),
                "-vf".into(),
                "idet".into(),
                "-frames:v".into(),
                w.frames.to_string(),
                "-an".into(),
                "-f".into(),
                "null".into(),
                "-".into(),
            ]);
            if let Some(stderr) = ctx.ffmpeg_stderr(&args, IDET_TIMEOUT)
                && let Some(counts) = IdetCounts::parse(&stderr) {
                    windows.push(counts);
                }
        }
        if windows.is_empty() {
            ctx.warn("no window could be analyzed for telecine".to_string());
        } else {
            ctx.report.telecine = Some(idet::verdict(&windows, format));
        }
    }

    ctx.report
}

/// Scan sampled frames for edge halos.
///
/// **Caveat worth knowing:** the Python extracts each frame to a PNG and takes
/// Pillow's `convert('L')`, which computes luma from RGB. This asks ffmpeg for
/// the gray plane directly, which is the source Y plane. The two are close but
/// not identical, so absolute halo ratios can differ slightly from the Python's
/// for the same file. The *scan* is parity-verified on identical planes; only
/// the extraction differs — and this way needs no temp files and no image
/// library.
fn probe_halo(
    ctx: &mut Ctx<'_>,
    duration: f64,
    info: VideoInfo,
    tick: &mut impl FnMut(&str, u32, u32),
) -> HaloAnalysis {
    let (w, h) = (info.width as usize, info.height as usize);
    let expected = w * h;
    let mut tally = EdgeTally::default();

    if expected == 0 {
        ctx.warn("frame size is unknown, so halos cannot be scanned".to_string());
        return halo::verdict(tally);
    }

    let ffmpeg = match ctx.tools.ffmpeg_path() {
        Some(p) => p.to_path_buf(),
        None => return halo::verdict(tally),
    };

    for (i, offset) in mid_sample_offsets(HALO_SAMPLES).iter().enumerate() {
        tick("halos", i as u32 + 1, HALO_SAMPLES);
        let t = duration * offset;
        let args: Vec<String> = vec![
            "-hide_banner".into(),
            "-fflags".into(),
            "+discardcorrupt".into(),
            "-ss".into(),
            format!("{t}"),
            "-i".into(),
            ctx.file.display().to_string(),
            "-vframes".into(),
            "1".into(),
            "-vf".into(),
            "format=gray".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "gray".into(),
            "-".into(),
        ];
        ctx.report.spawns += 1;
        match run_tool_binary(&ffmpeg, &args, DEFAULT_TIMEOUT) {
            Ok((_, data)) if data.len() >= expected => {
                // A frame shorter than expected is a truncated read and must
                // not be scanned, or the scan would index past the row it
                // thinks it is on.
                halo::scan_frame(
                    &LumaPlane { data: &data[..expected], width: w, height: h },
                    &mut tally,
                );
            }
            Ok((_, data)) if !data.is_empty() => {
                ctx.warn(format!(
                    "halo sample at {t:.1}s returned {} bytes, expected {expected}",
                    data.len()
                ));
            }
            Ok(_) => {}
            Err(e) => ctx.warn(format!("halo sample at {t:.1}s failed: {e}")),
        }
    }

    halo::verdict(tally)
}

/// A decoded RGB frame, for display.
pub struct Thumbnail {
    pub width: usize,
    pub height: usize,
    /// Row-major RGB, three bytes per pixel.
    pub rgb: Vec<u8>,
}

/// Grab one frame as raw RGB, scaled to `target_width`.
///
/// Raw `rgb24` on stdout rather than a PNG to a temp file: no image decoder is
/// needed, nothing touches the disk, and the exact byte count is known in
/// advance so a truncated read is detectable.
pub fn thumbnail(
    tools: &ToolPaths,
    file: &Path,
    info: &VideoInfo,
    target_width: usize,
    at_seconds: f64,
) -> Option<Thumbnail> {
    let ffmpeg = tools.ffmpeg_path()?;
    if info.width == 0 || info.height == 0 {
        return None;
    }

    // Keep the aspect and force an even height, which the scaler requires.
    let height =
        ((target_width as f64 * info.height as f64 / info.width as f64).round() as usize).max(2);
    let height = height + (height % 2);
    let expected = target_width * height * 3;

    let args: Vec<String> = vec![
        "-hide_banner".into(),
        "-fflags".into(),
        "+discardcorrupt".into(),
        "-ss".into(),
        format!("{at_seconds}"),
        "-i".into(),
        file.display().to_string(),
        "-vframes".into(),
        "1".into(),
        "-vf".into(),
        format!("scale={target_width}:{height}"),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-f".into(),
        "rawvideo".into(),
        "-".into(),
    ];

    let (_, data) = run_tool_binary(ffmpeg, &args, DEFAULT_TIMEOUT).ok()?;
    if data.len() < expected {
        return None;
    }
    Some(Thumbnail {
        width: target_width,
        height,
        rgb: data[..expected].to_vec(),
    })
}

/// Channel count from ffprobe, then per-channel levels from astats.
///
/// Returns `None` only when ffprobe could not be run at all; a file with no
/// audio track reports zero channels, which is a finding rather than a
/// failure.
fn probe_audio(
    tools: &ToolPaths,
    file: &Path,
    duration: f64,
    spawns: &mut u32,
) -> Option<AudioAnalysis> {
    let ffprobe = tools.ffprobe_path()?;
    *spawns += 1;
    let out = run_tool(
        ffprobe,
        [
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=channels",
            "-of",
            "default=nw=1:nk=1",
            &file.display().to_string(),
        ],
    )
    .ok()?;
    let channels: u32 = out.stdout.trim().parse().unwrap_or(0);
    if channels == 0 {
        return Some(audio::analyze(0, ""));
    }

    // Measure slices rather than the whole track — reading a two-hour tape to
    // judge its channels would dominate the analysis — but slices from
    // several places rather than one.
    //
    // One window at the head is not enough. A tape is one recording session
    // after another: a camcorder's internal mono mic for an hour, then a
    // stereo deck feed from the next tape on, and the first minute decides
    // for all of it. Worse, the head of a capture is often bars, silence or
    // leader, which reads as two dead channels.
    //
    // The windows are merged by keeping the loudest reading per channel, so a
    // channel counts as live if it carried sound anywhere. That errs towards
    // leaving the sound alone, which is the right direction: copying one side
    // over both is lossless on a genuinely one-sided tape and destroys real
    // stereo everywhere else.
    let ffmpeg = tools.ffmpeg_path()?;
    let mut merged: Vec<audio::ChannelLevel> = Vec::new();
    for start in audio_windows(duration) {
        *spawns += 1;
        let args: Vec<String> = vec![
            "-hide_banner".into(),
            "-ss".into(),
            format!("{start:.2}"),
            "-t".into(),
            AUDIO_WINDOW_SECONDS.to_string(),
            "-i".into(),
            file.display().to_string(),
            "-af".into(),
            "astats".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ];
        let mut cmd = crate::run::command(ffmpeg);
        cmd.args(&args);
        let stderr = crate::run::run(cmd, DEFAULT_TIMEOUT)
            .map(|o| o.stderr)
            .unwrap_or_default();
        audio::merge_loudest(&mut merged, &audio::parse_astats(&stderr));
    }
    Some(AudioAnalysis {
        channels,
        verdict: audio::classify(channels, &merged),
        levels: merged,
    })
}

/// How long each audio window is. Long enough to cover a pause in speech,
/// short enough that three of them cost less than the old single minute.
const AUDIO_WINDOW_SECONDS: u32 = 20;

/// Where to listen, in seconds from the start.
///
/// Spread through the file rather than bunched at the head, and never right
/// at either end: a capture usually opens on leader or bars and often ends
/// mid-stop, and neither says anything about the recording.
fn audio_windows(duration: f64) -> Vec<f64> {
    let window = AUDIO_WINDOW_SECONDS as f64;
    if !duration.is_finite() || duration <= window * 2.0 {
        // Too short to spread: one pass from the start covers all of it.
        return vec![0.0];
    }
    [0.1, 0.45, 0.8]
        .iter()
        .map(|f| (duration * f).min(duration - window).max(0.0))
        .collect()
}
