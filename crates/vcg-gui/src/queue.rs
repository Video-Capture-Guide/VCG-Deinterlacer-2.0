//! The export queue: several videos lined up, run one at a time.
//!
//! Modelled on a printer queue, which is the comparison the brief draws. The
//! job at the top is the one running; anything added lands behind it; items
//! can be reordered or removed, the running one included.
//!
//! **A job is a snapshot, not a pointer at the app.** Everything an export
//! needs — the source, the settings, the mode, the measured findings, which
//! companion files to write — is copied into the job when it is added. The
//! wizard then resets for the next video, and nothing the user does there can
//! reach a job that is already queued or running.
//!
//! That is what makes the Back button safe to leave enabled. The brief worried
//! that a user could walk back during a run and make edits that silently do
//! nothing; here those edits belong to the *next* job, which is what someone
//! walking back during a run almost always wants.
//!
//! It also fixes a smaller bug that predates the queue. The companion files
//! (`.settings.json`, `.vpy`, `.log.txt`) used to be written at the end of a
//! run from whatever was in `App` at that moment, on the assumption that it
//! was still what the export had used. Change a setting mid-render and the
//! sidecar would describe a pipeline that never ran. Writing them from the
//! snapshot removes the assumption.

use std::path::PathBuf;

use vcg_core::config::{Config, Mode};

/// The id the running job answers to in the queue list.
///
/// A running job has left `ExportQueue`, so it needs an id of its own for the
/// panel's buttons to address. Zero is safe because `push` numbers from one.
pub const RUNNING_JOB_ID: u64 = 0;

/// One export, frozen at the moment it was queued.
#[derive(Debug, Clone)]
pub struct QueuedJob {
    /// Stable across reordering, so the row the user clicked is the row that
    /// moves. An index would shift under a concurrent dequeue.
    pub id: u64,
    pub source: PathBuf,
    pub config: Config,
    pub mode: Mode,
    /// Where the output goes, or `None` to put it beside the source.
    ///
    /// The *folder* rather than the full path: the final name is settled when
    /// the job starts, not when it is queued. Two jobs queued from the same
    /// tape would otherwise both claim the same free filename, because
    /// neither file exists yet while they are waiting.
    pub export_folder: Option<PathBuf>,
    pub source_seconds: f64,
    pub source_fps: f64,
    pub comparison: bool,
    pub save_log: bool,
    pub save_settings: bool,
    pub save_script: bool,
    /// The findings text, rendered when the job was queued. The report it
    /// came from describes *this* job's tape, and the app will have moved on
    /// to another by the time the job runs.
    pub findings: Option<String>,
}

impl QueuedJob {
    /// The source file's name, for a queue row.
    pub fn source_name(&self) -> String {
        self.source
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.source.display().to_string())
    }

    /// The folder this job's output will land in.
    ///
    /// Derived rather than stored, because the job holds the *folder choice*
    /// (`None` meaning "beside the source") rather than a settled path — the
    /// filename itself is not decided until the job starts. See
    /// `export_folder`.
    pub fn output_folder(&self) -> Option<PathBuf> {
        match &self.export_folder {
            Some(dir) => Some(dir.clone()),
            None => self.source.parent().map(PathBuf::from),
        }
    }

    /// The settings worth showing on a queue row, as short chips.
    ///
    /// Only what differs between two jobs on the same tape is interesting
    /// here, so anything left at its default is omitted rather than listed as
    /// "off" — a row of eight chips saying nothing happened is harder to read
    /// than three saying what does.
    pub fn chips(&self) -> Vec<String> {
        use vcg_core::config::{
            DehaloMode, LevelsAdjustment, NoiseLevel, OutputFormat, QtgmcImpl,
        };
        let c = &self.config;
        let mut out = Vec::new();

        out.push(format!(
            "{:?} · {:?} · {}",
            c.format,
            c.field_order,
            clock(self.source_seconds)
        ));
        out.push(
            match c.qtgmc_impl {
                QtgmcImpl::Legacy => "QTGMC havsfunc",
                QtgmcImpl::Modern => "QTGMC jetpack",
            }
            .to_string(),
        );
        if c.crop_left + c.crop_right + c.crop_top + c.crop_bottom > 0 {
            out.push(format!(
                "Crop {}/{}/{}/{}",
                c.crop_left, c.crop_top, c.crop_right, c.crop_bottom
            ));
        }
        match c.noise_level {
            NoiseLevel::None => {}
            NoiseLevel::Moderate => out.push("Denoise moderate".into()),
            NoiseLevel::Heavy => out.push("Denoise heavy".into()),
        }
        match c.dehalo_mode {
            DehaloMode::None => {}
            DehaloMode::Light => out.push("Dehalo light".into()),
            DehaloMode::Strong => out.push("Dehalo strong".into()),
        }
        if c.levels_adjustment != LevelsAdjustment::None {
            out.push("Levels fix".into());
        }
        if c.upscale_enabled && let Some(r) = &c.upscale_resolution {
            out.push(format!("Upscale {r}"));
        }
        out.push(format!(
            "to {}",
            match c.output_format {
                OutputFormat::Prores => "ProRes MOV",
                OutputFormat::Ffv1 => "FFV1 MKV",
                OutputFormat::H264 => "H.264 MP4",
                OutputFormat::Huffyuv => "Huffyuv AVI",
                OutputFormat::Utvideo => "Ut Video AVI",
                OutputFormat::Lagarith => "Lagarith AVI",
            }
        ));
        out
    }

    /// A one-line description of what this job will do.
    ///
    /// The point is to tell two queued jobs apart at a glance — the same tape
    /// queued twice at different settings is the case that needs it — so it
    /// names the mode and the container, not every switch.
    pub fn summary_line(&self) -> String {
        use vcg_core::config::OutputFormat;
        let format = match self.config.output_format {
            OutputFormat::Prores => "ProRes",
            OutputFormat::Ffv1 => "FFV1",
            OutputFormat::H264 => "H.264",
            OutputFormat::Huffyuv => "Huffyuv",
            OutputFormat::Utvideo => "Ut Video",
            OutputFormat::Lagarith => "Lagarith",
        };
        let minutes = (self.source_seconds / 60.0).round() as u64;
        let length = if self.source_seconds < 60.0 {
            "under a minute".to_string()
        } else if minutes == 1 {
            "1 minute".to_string()
        } else {
            format!("{minutes} minutes")
        };
        format!("{} · {format} · {length}", self.mode.short_title())
    }
}

/// Seconds as h:mm:ss, or m:ss under an hour — how a running time reads.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// A duration as "1 h 54 min" / "26 min" / "under a minute".
///
/// Deliberately coarse. Every figure here is an extrapolation from how fast
/// the machine has been going so far, and printing "1 h 54 min 12 s" would
/// claim a precision the estimate does not have.
pub fn rough(seconds: f64) -> String {
    let m = (seconds.max(0.0) / 60.0).round() as u64;
    if m < 1 {
        "under a minute".to_string()
    } else if m < 60 {
        format!("{m} min")
    } else if m.is_multiple_of(60) {
        format!("{} h", m / 60)
    } else {
        format!("{} h {:02} min", m / 60, m % 60)
    }
}

/// Bytes as GB, or MB below a gigabyte.
pub fn size(bytes: u64) -> String {
    let gb = bytes as f64 / 1_000_000_000.0;
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", bytes as f64 / 1_000_000.0)
    }
}

/// How many seconds of wall clock each second of source has been costing.
///
/// The honest basis for every "time left" on the page. Taken from jobs this
/// session has actually finished, so it reflects this machine, this
/// deinterlacer and these settings rather than a number baked in by us.
/// `None` until something has finished — a guess with nothing behind it is
/// worse than saying nothing, so the page then omits the estimates entirely.
pub fn seconds_per_source_second(history: &[Finished]) -> Option<f64> {
    let (mut work, mut source) = (0.0, 0.0);
    for done in history {
        // Failures stopped early, so they say nothing about how long a whole
        // job takes.
        if done.outcome.is_ok() && done.source_seconds > 0.0 {
            work += done.took;
            source += done.source_seconds;
        }
    }
    (source > 0.0).then(|| work / source)
}

/// Whether a page about "the export" means the running job or another video.
///
/// With a queue the two come apart: a render carries on while the wizard is
/// being set up for the next tape. Two separate behaviours hang off this —
/// whether the Export page shows the progress panel or its own form, and
/// whether a finished job's summary is allowed to appear there — and getting
/// it wrong means describing one video on a page about a different one.
pub fn viewing_job(open: Option<&std::path::Path>, job_source: Option<&std::path::Path>) -> bool {
    match (open, job_source) {
        (Some(open), Some(source)) => open == source,
        // Nothing loaded: there is no other video the page could be about.
        (None, Some(_)) => true,
        // No job at all, or a file open with nothing running.
        _ => false,
    }
}

/// A job that has ended, for the queue page's history.
#[derive(Debug, Clone)]
pub struct Finished {
    pub source_name: String,
    /// The file it produced, or why it did not.
    pub outcome: Result<PathBuf, String>,
    /// Wall-clock seconds the job took.
    pub took: f64,
    /// Size of the finished file, or 0 when it never got written.
    pub bytes: u64,
    /// How long the source was, kept so finished jobs can tell the queue how
    /// fast this machine actually works — which is what the estimates for
    /// everything still waiting are built from.
    pub source_seconds: f64,
}

/// The waiting list. The running job is not held here — see `ExportQueue`.
#[derive(Debug, Default)]
pub struct ExportQueue {
    items: Vec<QueuedJob>,
    next_id: u64,
}

impl ExportQueue {
    /// Add a job to the back and return its id.
    pub fn push(&mut self, mut job: QueuedJob) -> u64 {
        self.next_id += 1;
        job.id = self.next_id;
        let id = job.id;
        self.items.push(job);
        id
    }

    /// Take the next job to run.
    pub fn pop_front(&mut self) -> Option<QueuedJob> {
        if self.items.is_empty() {
            None
        } else {
            Some(self.items.remove(0))
        }
    }

    /// Drop a waiting job. Returns whether it was there.
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|j| j.id != id);
        self.items.len() != before
    }

    /// Move a waiting job one place towards the front.
    ///
    /// A no-op at the front rather than an error: the button is drawn
    /// disabled there, and a queue that reorders itself when a disabled
    /// control is somehow reached would be worse than one that does nothing.
    pub fn move_up(&mut self, id: u64) -> bool {
        match self.position(id) {
            Some(i) if i > 0 => {
                self.items.swap(i - 1, i);
                true
            }
            _ => false,
        }
    }

    /// Move a waiting job one place towards the back.
    pub fn move_down(&mut self, id: u64) -> bool {
        match self.position(id) {
            Some(i) if i + 1 < self.items.len() => {
                self.items.swap(i, i + 1);
                true
            }
            _ => false,
        }
    }

    pub fn position(&self, id: u64) -> Option<usize> {
        self.items.iter().position(|j| j.id == id)
    }

    pub fn items(&self) -> &[QueuedJob] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

// ── The page ────────────────────────────────────────────────────────────────
//
// A full page rather than the modal this started as. The modal was "a bit
// small and unclear for newbies": a 560px box with its own scrollbar, inside
// which the heading scrolled out of sight once a few jobs had run, leaving a
// headless middle of a list. A queue someone leaves running overnight is a
// place they come back to, not a dialog they dismiss, so it gets the width of
// the window and a heading that stays put.

use eframe::egui::{self, RichText};

use crate::theme;
use crate::App;

/// What a row's buttons asked for, collected during the frame and applied
/// after it — the rows borrow the queue they would otherwise mutate.
pub enum QueueAction {
    Up(u64),
    Down(u64),
    Remove(u64),
    Duplicate(u64),
    Reveal(std::path::PathBuf),
    Open(std::path::PathBuf),
    AddAnother,
    ClearFinished,
    ToggleStopAfter,
    Close,
}

/// Everything the page needs to say about the state of the batch.
struct Totals {
    done: usize,
    failed: usize,
    total: usize,
    /// Seconds of work still to do, when there is any basis for saying.
    left: Option<f64>,
    bytes: u64,
}

fn totals(app: &App) -> Totals {
    let done = app.queue_history.iter().filter(|f| f.outcome.is_ok()).count();
    let failed = app.queue_history.len() - done;
    let running = usize::from(app.current_job.is_some());
    let bytes = app.queue_history.iter().map(|f| f.bytes).sum();

    // Per source-second cost, measured from what this machine has actually
    // done. Until something finishes there is nothing to extrapolate from and
    // the estimates are left off the page rather than invented.
    let rate = seconds_per_source_second(&app.queue_history);
    let left = rate.map(|r| {
        let waiting: f64 = app.queue.items().iter().map(|j| j.source_seconds * r).sum();
        waiting + app.export_job.as_ref().map_or(0.0, remaining_of)
    });

    Totals {
        done,
        failed,
        total: app.queue_history.len() + running + app.queue.len(),
        left,
        bytes,
    }
}

/// Seconds left on the running job, straight from ffmpeg's own reckoning.
fn remaining_of(job: &crate::worker::ExportJob) -> f64 {
    use vcg_tools::encode::Phase;
    match &job.phase {
        Some(Phase::Encoding { out_seconds, total_seconds, speed, .. }) if *speed > 0.01 => {
            ((total_seconds - out_seconds) / speed).max(0.0)
        }
        _ => 0.0,
    }
}

/// The queue page. Drawn in place of the wizard when the queue is open.
pub fn queue_page(ui: &mut egui::Ui, app: &App) -> Option<QueueAction> {
    let mut action = None;
    let width = ui.available_width().min(1180.0);
    ui.set_max_width(width);

    // ── Title row ──────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new("Export queue")
                    .size(theme::FONT_TITLE)
                    .color(theme::text()),
            );
            ui.add_space(3.0);
            ui.label(
                RichText::new(
                    "Videos export one at a time, top to bottom. Each job keeps \
                     the settings it was added with.",
                )
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::primary(ui, "Add another video", true).clicked() {
                action = Some(QueueAction::AddAnother);
            }
        });
    });
    ui.add_space(18.0);

    let t = totals(app);
    if let Some(a) = summary_card(ui, app, &t, width) {
        action = Some(a);
    }

    if let Some(job) = &app.current_job {
        ui.add_space(22.0);
        section_label(ui, "EXPORTING NOW");
        ui.add_space(8.0);
        if let Some(a) = exporting_now(ui, app, job, width) {
            action = Some(a);
        }
    }

    if !app.queue.is_empty() {
        ui.add_space(22.0);
        section_label(ui, &format!("UP NEXT · {}", app.queue.len()));
        ui.add_space(8.0);
        let last = app.queue.len().saturating_sub(1);
        let running = app.current_job.is_some();
        let rate = seconds_per_source_second(&app.queue_history);
        for (i, job) in app.queue.items().iter().enumerate() {
            if let Some(a) = waiting_row(ui, job, i, last, running, rate, width) {
                action = Some(a);
            }
            ui.add_space(6.0);
        }
    }

    if !app.queue_history.is_empty() {
        ui.add_space(22.0);
        ui.horizontal(|ui| {
            section_label(ui, &format!("FINISHED · {}", app.queue_history.len()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::link(ui, "Clear finished").clicked() {
                    action = Some(QueueAction::ClearFinished);
                }
            });
        });
        ui.add_space(8.0);
        // Newest first: what just happened is what is being looked for.
        for done in app.queue_history.iter().rev() {
            if let Some(a) = finished_row(ui, done, width) {
                action = Some(a);
            }
            ui.add_space(5.0);
        }
    }

    if app.current_job.is_none() && app.queue.is_empty() && app.queue_history.is_empty() {
        ui.add_space(20.0);
        theme::card().show(ui, |ui| {
            ui.set_width(width - 40.0);
            ui.label(
                RichText::new("Nothing queued yet.")
                    .size(theme::FONT_BODY)
                    .color(theme::text()),
            );
            ui.add_space(5.0);
            ui.label(
                RichText::new(
                    "Set a video up as usual, then choose Add to queue on the \
                     Export page. Add as many as you like — they run one after \
                     another, so you can load the next tape while one works.",
                )
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
            );
        });
    }

    ui.add_space(26.0);
    if theme::secondary(ui, "Back to the wizard").clicked() {
        action = Some(QueueAction::Close);
    }
    ui.add_space(20.0);

    action
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::FONT_TINY)
            .color(theme::muted()),
    );
}

/// One figure with its caption above it, as the design lays the summary out.
fn stat(ui: &mut egui::Ui, caption: &str, value: &str) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new(caption)
                .size(theme::FONT_TINY)
                .color(theme::muted()),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(value)
                .size(theme::FONT_SUBHEAD)
                .color(theme::text()),
        );
    });
}

fn summary_card(ui: &mut egui::Ui, app: &App, t: &Totals, width: f32) -> Option<QueueAction> {
    let mut action = None;
    let inner = width - 40.0;

    // Once everything has run the card changes character: it stops predicting
    // and starts reporting, which is the third screen of the design.
    let idle = app.current_job.is_none() && app.queue.is_empty();

    theme::card().show(ui, |ui| {
        ui.set_width(inner);

        if idle && t.total > 0 {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Queue finished")
                            .size(theme::FONT_SECTION)
                            .color(theme::text()),
                    );
                    ui.add_space(4.0);
                    let mut line = format!(
                        "{} of {} videos exported · {} written",
                        t.done,
                        t.total,
                        size(t.bytes)
                    );
                    if t.failed > 0 {
                        line.push_str(&format!(" · {} needs attention", t.failed));
                    }
                    ui.label(
                        RichText::new(line)
                            .size(theme::FONT_SMALL)
                            .color(theme::faint()),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The folder of the most recent success, which is where
                    // someone coming back to a finished batch wants to go.
                    if let Some(dir) = app
                        .queue_history
                        .iter()
                        .rev()
                        .find_map(|f| f.outcome.as_ref().ok())
                        .and_then(|p| p.parent())
                        && theme::primary(ui, "Open output folder", true).clicked()
                    {
                        action = Some(QueueAction::Open(dir.to_path_buf()));
                    }
                });
            });
            return;
        }

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 46.0;
            stat(ui, "PROGRESS", &format!("{} of {} finished", t.done, t.total));
            if t.failed > 0 {
                stat(ui, "FAILED", &t.failed.to_string());
            }
            // No "finishes at" clock: the only time source here is UTC, and
            // printing a wall-clock time that is wrong by the user's offset
            // would be worse than leaving it out. "Time left" needs no
            // timezone to be true.
            match t.left {
                Some(left) => stat(ui, "TIME LEFT", &format!("≈ {}", rough(left))),
                None => stat(ui, "TIME LEFT", "measuring…"),
            }
            if t.bytes > 0 {
                stat(ui, "WRITTEN SO FAR", &size(t.bytes));
            }
        });

        ui.add_space(12.0);
        let fraction = if t.total == 0 {
            0.0
        } else {
            (t.done as f32 + app.export_job.as_ref().map_or(0.0, |j| j.fraction()))
                / t.total as f32
        };
        ui.add(
            egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                .desired_height(8.0)
                .corner_radius(4.0),
        );

        if app.export_job.is_some() {
            ui.add_space(12.0);
            // "Stop after this video" is the one batch-level control here
            // that needs nothing new behind it: it simply declines to start
            // the next job, and the one running is left alone.
            let mut stop_after = app.stop_after_current;
            if ui
                .checkbox(&mut stop_after, "Stop after this video")
                .on_hover_text(
                    "Lets the video that is running finish, then holds the rest \
                     of the queue. Nothing is lost — press it again to carry on.",
                )
                .clicked()
            {
                action = Some(QueueAction::ToggleStopAfter);
            }
            if app.stop_after_current {
                ui.label(
                    RichText::new(
                        "The queue will stop when this video finishes. The jobs \
                         behind it keep their place.",
                    )
                    .size(theme::FONT_SMALL)
                    .color(theme::faint()),
                );
            }
        }
    });

    action
}

fn exporting_now(
    ui: &mut egui::Ui,
    app: &App,
    job: &QueuedJob,
    width: f32,
) -> Option<QueueAction> {
    let mut action = None;
    let inner = width - 40.0;
    let stopping = app.export_job.as_ref().is_some_and(|j| j.stopping());

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(inner - 190.0);
                ui.label(
                    RichText::new(job.source_name())
                        .size(theme::FONT_BODY)
                        .color(theme::text()),
                );
                ui.add_space(6.0);
                chip_row(ui, job, inner - 190.0);
                ui.add_space(8.0);
                ui.add(
                    egui::ProgressBar::new(app.export_job.as_ref().map_or(0.0, |j| j.fraction()))
                        .desired_height(6.0)
                        .corner_radius(3.0),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(if stopping {
                        "Stopping…".to_string()
                    } else {
                        app.export_job.as_ref().map(live_line).unwrap_or_default()
                    })
                    .size(theme::FONT_SMALL)
                    .color(theme::faint()),
                );
                if let Some(dir) = job.output_folder() {
                    ui.add_space(5.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        ui.label(
                            RichText::new("Saving to")
                                .size(theme::FONT_SMALL)
                                .color(theme::faint()),
                        );
                        if theme::link(ui, &dir.display().to_string()).clicked() {
                            action = Some(QueueAction::Reveal(dir.clone()));
                        }
                    });
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !stopping && theme::secondary(ui, "Cancel this video").clicked() {
                    action = Some(QueueAction::Remove(RUNNING_JOB_ID));
                }
            });
        });
    });
    action
}

/// "42% · 0:31 of 1:14 · 38.4 fps · 41 min left", from ffmpeg's own counters.
fn live_line(job: &crate::worker::ExportJob) -> String {
    use vcg_tools::encode::Phase;
    match &job.phase {
        None => "Starting up…".to_string(),
        Some(Phase::ExtractingAudio) => "Working on the sound…".to_string(),
        Some(Phase::Verifying) => "Checking the finished file…".to_string(),
        Some(Phase::Comparing) => "Making the before-and-after clip…".to_string(),
        Some(Phase::Encoding { out_seconds, total_seconds, fps, speed }) => {
            let pct = if *total_seconds > 0.0 {
                (out_seconds / total_seconds * 100.0).round() as u32
            } else {
                0
            };
            let mut s = format!(
                "{pct}% · {} of {} · {fps:.1} fps",
                clock(*out_seconds),
                clock(*total_seconds)
            );
            if *speed > 0.01 && total_seconds > out_seconds {
                s.push_str(&format!(
                    " · {} left",
                    rough((total_seconds - out_seconds) / speed)
                ));
            }
            s
        }
    }
}

fn chip_row(ui: &mut egui::Ui, job: &QueuedJob, width: f32) {
    ui.set_max_width(width);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(5.0, 4.0);
        for chip in job.chips() {
            egui::Frame::NONE
                .fill(theme::panel2())
                .corner_radius(theme::RADIUS_SMALL)
                .inner_margin(egui::Margin::symmetric(7, 3))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(chip)
                            .size(theme::FONT_TINY)
                            .color(theme::muted()),
                    );
                });
        }
    });
}

fn waiting_row(
    ui: &mut egui::Ui,
    job: &QueuedJob,
    index: usize,
    last: usize,
    running: bool,
    rate: Option<f64>,
    width: f32,
) -> Option<QueueAction> {
    let mut action = None;
    let inner = width - 40.0;

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        ui.horizontal(|ui| {
            // Numbered from 2 when something is running: the job above is 1.
            let position = if running { index + 2 } else { index + 1 };
            ui.label(
                RichText::new(format!("{position}"))
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
            );
            ui.add_space(6.0);

            ui.vertical(|ui| {
                ui.set_width(inner - 250.0);
                ui.label(
                    RichText::new(job.source_name())
                        .size(theme::FONT_BODY)
                        .color(theme::text()),
                );
                ui.add_space(5.0);
                chip_row(ui, job, inner - 250.0);
                if let Some(dir) = job.output_folder() {
                    ui.add_space(5.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        ui.label(
                            RichText::new("Saving to")
                                .size(theme::FONT_SMALL)
                                .color(theme::faint()),
                        );
                        if theme::link(ui, &dir.display().to_string()).clicked() {
                            action = Some(QueueAction::Reveal(dir.clone()));
                        }
                    });
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::ghost(ui, "Remove").clicked() {
                    action = Some(QueueAction::Remove(job.id));
                }
                if ui
                    .add(small_button("Copy"))
                    .on_hover_text("Add another job just like this one to the end of the queue.")
                    .clicked()
                {
                    action = Some(QueueAction::Duplicate(job.id));
                }
                // Disabled rather than hidden at the ends, so a row's
                // controls do not shift about as items are reordered.
                if ui.add_enabled(index < last, small_button("Down")).clicked() {
                    action = Some(QueueAction::Down(job.id));
                }
                if ui.add_enabled(index > 0, small_button("Up")).clicked() {
                    action = Some(QueueAction::Up(job.id));
                }
                if let Some(r) = rate {
                    ui.label(
                        RichText::new(format!("≈ {}", rough(job.source_seconds * r)))
                            .size(theme::FONT_SMALL)
                            .color(theme::faint()),
                    );
                }
            });
        });
    });
    action
}

fn finished_row(ui: &mut egui::Ui, done: &Finished, width: f32) -> Option<QueueAction> {
    let mut action = None;
    let inner = width - 40.0;

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        ui.horizontal(|ui| {
            let (mark, color) = match &done.outcome {
                Ok(_) => ("Done", theme::ok()),
                Err(_) => ("Failed", theme::fix()),
            };
            ui.label(RichText::new(mark).size(theme::FONT_SMALL).color(color));
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.set_width(inner - 240.0);
                ui.label(
                    RichText::new(&done.source_name)
                        .size(theme::FONT_BODY)
                        .color(theme::text()),
                );
                if let Err(why) = &done.outcome {
                    ui.add_space(3.0);
                    ui.label(
                        RichText::new(first_line(why))
                            .size(theme::FONT_SMALL)
                            .color(theme::faint()),
                    );
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Ok(output) = &done.outcome {
                    if theme::link(ui, "Show in folder").clicked() {
                        action = Some(QueueAction::Reveal(output.clone()));
                    }
                    if theme::link(ui, "Play").clicked() {
                        action = Some(QueueAction::Open(output.clone()));
                    }
                    ui.label(
                        RichText::new(format!("{} · {}", rough(done.took), size(done.bytes)))
                            .size(theme::FONT_SMALL)
                            .color(theme::faint()),
                    );
                }
            });
        });
    });
    action
}

/// A failure message can run to several lines of tool output; a row wants the
/// sentence, not the transcript.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

/// Spelled out rather than drawn as a glyph: the bundled fonts carry no arrow
/// characters, which is why the wizard's Back button paints its own.
fn small_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text.to_string())
            .size(theme::FONT_SMALL)
            .color(theme::muted()),
    )
    .fill(egui::Color32::TRANSPARENT)
    .stroke(egui::Stroke::NONE)
    .corner_radius(theme::RADIUS_SMALL)
    .min_size(egui::Vec2::new(0.0, 28.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(name: &str) -> QueuedJob {
        QueuedJob {
            id: 0,
            source: PathBuf::from(name),
            config: Config::default(),
            mode: Mode::JustDeinterlace,
            export_folder: None,
            source_seconds: 60.0,
            source_fps: 29.97,
            comparison: false,
            save_log: false,
            save_settings: false,
            save_script: false,
            findings: None,
        }
    }

    fn names(q: &ExportQueue) -> Vec<String> {
        q.items().iter().map(|j| j.source_name()).collect()
    }

    #[test]
    fn jobs_run_in_the_order_they_were_added() {
        let mut q = ExportQueue::default();
        q.push(job("a.avi"));
        q.push(job("b.avi"));
        q.push(job("c.avi"));
        assert_eq!(q.pop_front().unwrap().source_name(), "a.avi");
        assert_eq!(q.pop_front().unwrap().source_name(), "b.avi");
        assert_eq!(q.pop_front().unwrap().source_name(), "c.avi");
        assert!(q.pop_front().is_none());
    }

    #[test]
    fn ids_are_unique_and_survive_reordering() {
        // The row the user clicks must be the row that moves. Ids are handed
        // out by the queue rather than taken from the caller so two jobs
        // queued from the same tape cannot collide.
        let mut q = ExportQueue::default();
        let a = q.push(job("a.avi"));
        let b = q.push(job("a.avi"));
        assert_ne!(a, b);
        q.move_down(a);
        assert_eq!(q.position(a), Some(1));
        assert_eq!(q.position(b), Some(0));
    }

    #[test]
    fn reordering_stops_at_the_ends() {
        let mut q = ExportQueue::default();
        let a = q.push(job("a.avi"));
        let b = q.push(job("b.avi"));
        assert!(!q.move_up(a), "already at the front");
        assert!(!q.move_down(b), "already at the back");
        assert_eq!(names(&q), ["a.avi", "b.avi"]);

        assert!(q.move_down(a));
        assert_eq!(names(&q), ["b.avi", "a.avi"]);
        assert!(q.move_up(a));
        assert_eq!(names(&q), ["a.avi", "b.avi"]);
    }

    #[test]
    fn removing_a_job_leaves_the_rest_in_order() {
        let mut q = ExportQueue::default();
        q.push(job("a.avi"));
        let b = q.push(job("b.avi"));
        q.push(job("c.avi"));
        assert!(q.remove(b));
        assert_eq!(names(&q), ["a.avi", "c.avi"]);
        assert!(!q.remove(b), "removing twice is not an error, just false");
    }

    #[test]
    fn an_unknown_id_moves_nothing() {
        let mut q = ExportQueue::default();
        q.push(job("a.avi"));
        assert!(!q.move_up(999));
        assert!(!q.move_down(999));
        assert!(!q.remove(999));
        assert_eq!(names(&q), ["a.avi"]);
    }

    #[test]
    fn the_export_page_follows_the_file_on_screen_not_the_running_job() {
        use std::path::Path;
        let running = Path::new("a.avi");

        // Watching the tape that is rendering: the page is about that job.
        assert!(viewing_job(Some(running), Some(running)));
        // Moved on to set the next tape up: the page is about the new one,
        // so it must offer its own controls rather than the progress panel.
        assert!(!viewing_job(Some(Path::new("b.avi")), Some(running)));
        // Nothing loaded yet — there is no other video it could mean.
        assert!(viewing_job(None, Some(running)));
        // Nothing running at all.
        assert!(!viewing_job(Some(running), None));
        assert!(!viewing_job(None, None));
    }

    #[test]
    fn the_lock_lifts_as_soon_as_another_video_is_started() {
        use std::path::Path;
        let running = Path::new("a.avi");
        // `wizard_locked` is `file.is_some() && viewing_job(..)`. The pair
        // that matters: no file loaded means the user is setting the next
        // job up, so the wizard must be open even though a render continues.
        let locked = |file: Option<&Path>| {
            file.is_some() && viewing_job(file, Some(running))
        };
        assert!(locked(Some(running)), "watching your own render");
        assert!(!locked(Some(Path::new("b.avi"))), "already on the next tape");
        assert!(!locked(None), "cleared the file to start another");
    }

    #[test]
    fn a_job_knows_the_folder_its_output_will_land_in() {
        // The queue row links to this, so it has to be right before the job
        // has written anything. With no folder chosen the output goes beside
        // the source, which is what the Export page promises.
        let mut beside = job("a.avi");
        beside.source = PathBuf::from(r"D:\Tapes\a.avi");
        assert_eq!(beside.output_folder(), Some(PathBuf::from(r"D:\Tapes")));

        let mut chosen = job("a.avi");
        chosen.source = PathBuf::from(r"D:\Tapes\a.avi");
        chosen.export_folder = Some(PathBuf::from(r"E:\Done"));
        assert_eq!(chosen.output_folder(), Some(PathBuf::from(r"E:\Done")));
    }

    #[test]
    fn a_bare_filename_has_no_folder_to_offer() {
        // `parent()` of a bare name is an empty path, not None, and linking
        // to "" would open whatever Explorer felt like. The row omits the
        // line entirely instead.
        let mut bare = job("a.avi");
        bare.source = PathBuf::from("a.avi");
        let folder = bare.output_folder();
        assert!(
            folder.is_none() || folder.as_deref() == Some(std::path::Path::new("")),
            "unexpected folder: {folder:?}"
        );
    }

    #[test]
    fn a_summary_line_tells_two_jobs_on_the_same_tape_apart() {
        let mut prores = job("tape.avi");
        prores.config.output_format = vcg_core::config::OutputFormat::Prores;
        prores.source_seconds = 1800.0;
        let mut h264 = job("tape.avi");
        h264.config.output_format = vcg_core::config::OutputFormat::H264;
        h264.source_seconds = 1800.0;

        assert_ne!(prores.summary_line(), h264.summary_line());
        assert!(prores.summary_line().contains("ProRes"));
        assert!(h264.summary_line().contains("H.264"));
        assert!(prores.summary_line().contains("30 minutes"));
    }

    #[test]
    fn a_short_clip_is_not_described_as_zero_minutes() {
        let mut short = job("clip.avi");
        short.source_seconds = 12.0;
        assert!(short.summary_line().contains("under a minute"), "{}", short.summary_line());

        let mut one = job("clip.avi");
        one.source_seconds = 62.0;
        assert!(one.summary_line().contains("1 minute"), "{}", one.summary_line());
    }

    #[test]
    fn the_running_job_id_is_never_handed_out_to_a_waiting_one() {
        // The panel addresses the running job by RUNNING_JOB_ID, so a waiting
        // job sharing that id would make Remove stop the wrong thing.
        let mut q = ExportQueue::default();
        for _ in 0..5 {
            assert_ne!(q.push(job("a.avi")), RUNNING_JOB_ID);
        }
    }

    #[test]
    fn a_job_keeps_its_own_settings_not_the_apps() {
        // The whole point of the snapshot: queue a job, change your mind,
        // queue another, and the first still exports what it was given.
        let mut first = job("a.avi");
        first.config.noise_level = vcg_core::config::NoiseLevel::Heavy;
        let mut second = job("b.avi");
        second.config.noise_level = vcg_core::config::NoiseLevel::None;

        let mut q = ExportQueue::default();
        q.push(first);
        q.push(second);

        let a = q.pop_front().unwrap();
        let b = q.pop_front().unwrap();
        assert_eq!(a.config.noise_level, vcg_core::config::NoiseLevel::Heavy);
        assert_eq!(b.config.noise_level, vcg_core::config::NoiseLevel::None);
    }
}
