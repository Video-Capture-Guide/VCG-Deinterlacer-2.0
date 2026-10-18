//! Running the probes off the UI thread.
//!
//! Analysis takes seconds to a minute, so it cannot run inside `ui()` — egui
//! redraws continuously and a blocking call there freezes the window. The
//! Python uses `threading.Thread` plus `self.after()` to marshal results back;
//! here a worker thread sends messages down a channel that `ui()` drains, which
//! makes the data flow explicit and means a stale result can never land on the
//! wrong page.

use std::path::PathBuf;
use std::sync::mpsc;

use vcg_tools::paths::ToolPaths;
use vcg_tools::probe::{probe_file, Options, Report};

/// A message from a running probe job.
pub enum Msg {
    Progress {
        name: String,
        done: u32,
        total: u32,
    },
    Done(Box<Report>),
}

/// A probe running in the background.
pub struct Job {
    rx: mpsc::Receiver<Msg>,
    /// The file this job is for, so a result arriving after the user has
    /// changed files can be discarded instead of shown against the wrong one.
    pub file: PathBuf,
    pub progress: Option<(String, u32, u32)>,
    pub finished: bool,
}

impl Job {
    /// Start probing `file`.
    pub fn start(tools: ToolPaths, file: PathBuf, opts: Options) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread_file = file.clone();

        std::thread::spawn(move || {
            let mut send_progress = |name: &str, done: u32, total: u32| {
                // A failed send means the UI dropped the job; the probe then
                // finishes harmlessly and its result is discarded.
                let _ = tx.send(Msg::Progress {
                    name: name.to_string(),
                    done,
                    total,
                });
            };
            let report = probe_file(&tools, &thread_file, opts, Some(&mut send_progress));
            let _ = tx.send(Msg::Done(Box::new(report)));
        });

        Self {
            rx,
            file,
            progress: None,
            finished: false,
        }
    }

    /// Drain pending messages. Returns the report once it arrives.
    ///
    /// Called every frame, so it must never block.
    pub fn poll(&mut self) -> Option<Box<Report>> {
        let mut done = None;
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Progress { name, done: d, total }) => {
                    self.progress = Some((name, d, total));
                }
                Ok(Msg::Done(report)) => {
                    self.finished = true;
                    done = Some(report);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // The thread ended. If it ended without sending a report
                    // it panicked, which `finished` records so the UI can say
                    // so rather than spinning forever.
                    self.finished = true;
                    break;
                }
            }
        }
        done
    }

    /// Fraction complete, for a progress bar.
    pub fn fraction(&self) -> f32 {
        match &self.progress {
            Some((_, done, total)) if *total > 0 => *done as f32 / *total as f32,
            _ => 0.0,
        }
    }

    /// What the probe is currently doing.
    pub fn label(&self) -> String {
        match &self.progress {
            Some((name, done, total)) => format!("{name} — {done} of {total}"),
            None => "starting…".to_string(),
        }
    }
}

/// The quick container read plus a thumbnail, off the UI thread.
///
/// One ffprobe and one frame grab sounds instant and is not: seeking a
/// quarter of the way into a two-hour capture takes real time, and
/// `run::command` allows every child forty-five seconds before it gives up.
/// This used to run inside `ui()`, which froze the window for as long as it
/// took — nothing repainted, drops were ignored and the close button did not
/// answer. That is indistinguishable from a crash, and it was reported as
/// one.
pub struct ContainerJob {
    rx: mpsc::Receiver<(Box<Report>, Option<vcg_tools::probe::Thumbnail>)>,
    /// The file this read is for, so a result arriving after the user has
    /// changed files is discarded rather than shown against the wrong one.
    pub file: PathBuf,
    pub finished: bool,
}

impl ContainerJob {
    /// Read `file`'s container and grab one frame from it.
    pub fn start(tools: ToolPaths, file: PathBuf, thumb_width: usize) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread_file = file.clone();

        std::thread::spawn(move || {
            let report = probe_file(&tools, &thread_file, Options::container_only(), None);
            // A frame a little way in, because tape heads and leaders often
            // make the first second black.
            let thumb = report.info.and_then(|info| {
                let at = (info.duration * 0.15).clamp(0.0, info.duration.max(0.0));
                vcg_tools::probe::thumbnail(&tools, &thread_file, &info, thumb_width, at)
            });
            let _ = tx.send((Box::new(report), thumb));
        });

        Self {
            rx,
            file,
            finished: false,
        }
    }

    #[allow(clippy::type_complexity)]
    pub fn poll(&mut self) -> Option<(Box<Report>, Option<vcg_tools::probe::Thumbnail>)> {
        match self.rx.try_recv() {
            Ok(pair) => {
                self.finished = true;
                Some(pair)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                // The thread ended without sending, so it panicked. The card
                // then shows the name and size it can read itself, which is
                // better than waiting forever for chips that are not coming.
                self.finished = true;
                None
            }
        }
    }
}

/// A before/after preview being rendered in the background.
///
/// Scoring nine frames then running QTGMC takes a couple of seconds, so the
/// analysis results are shown first and the picture appears when it is ready.
/// The brief is explicit that this must never block, and that a failure shows
/// nothing rather than a placeholder.
pub struct PreviewJob {
    rx: mpsc::Receiver<Result<Box<vcg_tools::preview::Comparison>, String>>,
    /// The file this render is for, so a late result cannot be shown against a
    /// file the user has since changed.
    pub file: PathBuf,
    pub finished: bool,
    /// Why it failed, if it did. Kept for the technical-details view rather
    /// than shown to everyone.
    pub error: Option<String>,
}

impl PreviewJob {
    pub fn start(
        tools: ToolPaths,
        deps: vcg_core::vpy::DepsPaths,
        config: vcg_core::config::Config,
        file: PathBuf,
        info: vcg_core::analysis::probe::VideoInfo,
        region: (usize, usize),
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread_file = file.clone();

        std::thread::spawn(move || {
            let result = vcg_tools::preview::comparison(
                &tools,
                &deps,
                &config,
                &thread_file,
                &info,
                region.0,
                region.1,
            )
            .map(Box::new);
            let _ = tx.send(result);
        });

        Self {
            rx,
            file,
            finished: false,
            error: None,
        }
    }

    /// Drain the channel. Returns the comparison once it arrives.
    pub fn poll(&mut self) -> Option<Box<vcg_tools::preview::Comparison>> {
        match self.rx.try_recv() {
            Ok(Ok(comparison)) => {
                self.finished = true;
                Some(comparison)
            }
            Ok(Err(why)) => {
                self.finished = true;
                self.error = Some(why);
                None
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                // The thread ended without sending, so it panicked.
                self.finished = true;
                if self.error.is_none() {
                    self.error = Some("the preview render stopped unexpectedly".to_string());
                }
                None
            }
        }
    }
}

/// One whole frame being fetched for the Crop page.
///
/// Separate from `PreviewJob` because it needs no VapourSynth: it is a single
/// ffmpeg frame grab, so it works even when only a system ffmpeg is available.
pub struct FrameJob {
    rx: mpsc::Receiver<Result<Box<vcg_tools::preview::Image>, String>>,
    /// The file this frame is for, so a late result cannot be drawn with
    /// another file's crop values over it.
    pub file: PathBuf,
    pub finished: bool,
    pub error: Option<String>,
}

impl FrameJob {
    pub fn start(
        tools: ToolPaths,
        config: vcg_core::config::Config,
        file: PathBuf,
        info: vcg_core::analysis::probe::VideoInfo,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread_file = file.clone();

        std::thread::spawn(move || {
            let result =
                vcg_tools::preview::whole_frame(&tools, &config, &thread_file, &info).map(Box::new);
            let _ = tx.send(result);
        });

        Self {
            rx,
            file,
            finished: false,
            error: None,
        }
    }

    pub fn poll(&mut self) -> Option<Box<vcg_tools::preview::Image>> {
        match self.rx.try_recv() {
            Ok(Ok(image)) => {
                self.finished = true;
                Some(image)
            }
            Ok(Err(why)) => {
                self.finished = true;
                self.error = Some(why);
                None
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.finished = true;
                if self.error.is_none() {
                    self.error = Some("the frame grab stopped unexpectedly".to_string());
                }
                None
            }
        }
    }
}

/// The export, running in the background.
///
/// Unlike the probe and preview jobs this one cannot be discarded when the
/// user changes file: it is writing to disk. It still records which file it
/// was started for, so the result is never shown against another.
pub struct ExportJob {
    rx: mpsc::Receiver<ExportMsg>,
    pub file: PathBuf,
    pub output: PathBuf,
    pub phase: Option<vcg_tools::encode::Phase>,
    pub finished: bool,
    pub started: std::time::Instant,
    /// Set when the user confirms a stop. The worker checks it between
    /// progress blocks and kills both children.
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

enum ExportMsg {
    Progress(vcg_tools::encode::Phase),
    Done(Box<Result<vcg_tools::encode::Outcome, String>>),
}

impl ExportJob {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        tools: ToolPaths,
        deps: vcg_core::vpy::DepsPaths,
        config: vcg_core::config::Config,
        file: PathBuf,
        output: PathBuf,
        source_seconds: f64,
        source_fps: f64,
        comparison: bool,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread_file = file.clone();
        let thread_output = output.clone();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_cancel = cancel.clone();

        std::thread::spawn(move || {
            let mut report = |phase: vcg_tools::encode::Phase| {
                let _ = tx.send(ExportMsg::Progress(phase));
            };
            let result = vcg_tools::encode::run(
                vcg_tools::encode::Request {
                    tools: &tools,
                    deps: &deps,
                    config: &config,
                    source: &thread_file,
                    source_seconds,
                    source_fps,
                    output: &thread_output,
                    comparison,
                    cancel: thread_cancel,
                },
                Some(&mut report),
            );
            let _ = tx.send(ExportMsg::Done(Box::new(result)));
        });

        Self {
            rx,
            file,
            output,
            phase: None,
            finished: false,
            started: std::time::Instant::now(),
            cancel,
        }
    }

    /// Ask the render to stop.
    ///
    /// Does not wait: the worker notices within about half a second and
    /// comes back through the same channel as any other ending, so there is
    /// exactly one path by which the page learns the job is over. Blocking
    /// the interface on a process kill would be the bug this whole module
    /// exists to avoid.
    pub fn stop(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether a stop has been asked for and not yet taken effect.
    pub fn stopping(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Drain pending messages. Returns the result once it arrives.
    pub fn poll(&mut self) -> Option<Result<vcg_tools::encode::Outcome, String>> {
        let mut done = None;
        loop {
            match self.rx.try_recv() {
                Ok(ExportMsg::Progress(p)) => self.phase = Some(p),
                Ok(ExportMsg::Done(r)) => {
                    self.finished = true;
                    done = Some(*r);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finished = true;
                    break;
                }
            }
        }
        done
    }

    pub fn fraction(&self) -> f32 {
        self.phase.as_ref().map_or(0.0, |p| p.fraction())
    }

    /// What is happening, in words a newcomer can read.
    pub fn label(&self) -> String {
        use vcg_tools::encode::Phase;
        match &self.phase {
            None => "Starting up…".to_string(),
            Some(Phase::ExtractingAudio) => "Working on the sound…".to_string(),
            Some(Phase::Encoding {
                out_seconds,
                total_seconds,
                speed,
                ..
            }) => {
                let done = format_clock(*out_seconds);
                let total = format_clock(*total_seconds);
                // Below about a tenth of real time an estimate swings wildly
                // for the first few seconds, so it is left out until the
                // measurement settles.
                let left = if *speed > 0.01 && *total_seconds > *out_seconds {
                    let remaining = (total_seconds - out_seconds) / speed;
                    format!(" · about {} left", format_rough(remaining))
                } else {
                    String::new()
                };
                format!("Enhancing your video — {done} of {total}{left}")
            }
            Some(Phase::Verifying) => "Checking the finished file…".to_string(),
            Some(Phase::Comparing) => "Making the before-and-after clip…".to_string(),
        }
    }
}

/// Seconds as m:ss, which is how a running time reads.
pub fn format_clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A deliberately vague estimate. Claiming "3:47 left" from a figure that
/// moves every second reads as precision the number does not have.
pub fn format_rough(seconds: f64) -> String {
    let s = seconds.max(0.0);
    if s < 60.0 {
        "under a minute".to_string()
    } else if s < 150.0 {
        "a couple of minutes".to_string()
    } else {
        format!("{} minutes", (s / 60.0).round() as u64)
    }
}

/// One frame being fetched for the Trim timeline.
///
/// Separate from `FrameJob` because of the staleness rule: dragging a slider
/// asks for a new frame constantly, and the answers come back out of order.
/// Each job carries the moment it was asked for, so a late arrival for a
/// position the user has already left is dropped rather than flashed up.
pub struct ScrubJob {
    rx: mpsc::Receiver<Result<Box<vcg_tools::preview::Image>, String>>,
    pub file: PathBuf,
    /// The moment this job was asked for, in seconds.
    pub at: f64,
    pub finished: bool,
}

impl ScrubJob {
    pub fn start(
        tools: ToolPaths,
        config: vcg_core::config::Config,
        file: PathBuf,
        info: vcg_core::analysis::probe::VideoInfo,
        at: f64,
        width: usize,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread_file = file.clone();

        std::thread::spawn(move || {
            let result =
                vcg_tools::preview::frame_at(&tools, &config, &thread_file, &info, at, width)
                    .map(Box::new);
            let _ = tx.send(result);
        });

        Self {
            rx,
            file,
            at,
            finished: false,
        }
    }

    pub fn poll(&mut self) -> Option<Box<vcg_tools::preview::Image>> {
        match self.rx.try_recv() {
            Ok(Ok(image)) => {
                self.finished = true;
                Some(image)
            }
            Ok(Err(_)) => {
                self.finished = true;
                None
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.finished = true;
                None
            }
        }
    }
}

/// A settings-page preview being rendered.
///
/// Carries the exact request it was started for — the filter string or the
/// watermark state — so a late result for a setting the user has already
/// changed is dropped rather than shown as if it were current. A preview
/// that lags behind the control is worse than none: it says the change did
/// something other than it did.
pub struct SettingPreviewJob {
    rx: mpsc::Receiver<Result<Box<vcg_tools::preview::Image>, String>>,
    pub file: PathBuf,
    /// What this render was asked for.
    pub tag: String,
    pub finished: bool,
    /// When it started, so the page can say how long it has been going.
    ///
    /// There is no frame count to report — one frame is being rendered — and
    /// the wait is not where a newcomer expects it: most of it is the source
    /// indexer reading the whole file before it can seek, which is why a
    /// two-hour tape takes so much longer than a two-minute one. Elapsed time
    /// with that explanation is honest; a progress bar with nothing behind it
    /// would not be.
    pub started: std::time::Instant,
}

/// What to render.
pub enum PreviewKind {
    /// An extra ffmpeg filter chain, empty for the untouched frame.
    Filter(String),
    /// The configured watermark, burnt in.
    Watermark,
    /// One filter stage, applied or not, through VapourSynth.
    Stage {
        stage: vcg_core::vpy::PreviewStage,
        apply: bool,
        deps: vcg_core::vpy::DepsPaths,
        info: vcg_core::analysis::probe::VideoInfo,
    },
}

impl SettingPreviewJob {
    pub fn start(
        tools: ToolPaths,
        config: vcg_core::config::Config,
        file: PathBuf,
        at: f64,
        out: (usize, usize),
        kind: PreviewKind,
        tag: String,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread_file = file.clone();
        let started = std::time::Instant::now();

        std::thread::spawn(move || {
            let result = match kind {
                PreviewKind::Filter(extra) => vcg_tools::preview::filtered_frame(
                    &tools,
                    &config,
                    &thread_file,
                    at,
                    out.0,
                    out.1,
                    &extra,
                ),
                PreviewKind::Watermark => vcg_tools::preview::watermark_frame(
                    &tools,
                    &config,
                    &thread_file,
                    at,
                    out.0,
                    out.1,
                ),
                PreviewKind::Stage {
                    stage,
                    apply,
                    deps,
                    info,
                } => vcg_tools::preview::stage_frame(vcg_tools::preview::StageRequest {
                    tools: &tools,
                    deps: &deps,
                    config: &config,
                    file: &thread_file,
                    info: &info,
                    stage,
                    apply,
                    at,
                    out,
                }),
            }
            .map(Box::new);
            let _ = tx.send(result);
        });

        Self {
            rx,
            file,
            tag,
            finished: false,
            started,
        }
    }

    pub fn poll(&mut self) -> Option<Box<vcg_tools::preview::Image>> {
        match self.rx.try_recv() {
            Ok(Ok(image)) => {
                self.finished = true;
                Some(image)
            }
            Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                self.finished = true;
                None
            }
            Err(mpsc::TryRecvError::Empty) => None,
        }
    }
}
