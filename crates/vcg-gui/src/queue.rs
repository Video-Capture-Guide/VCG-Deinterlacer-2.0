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

/// A job that has ended, for the queue panel's history.
#[derive(Debug, Clone)]
pub struct Finished {
    pub source_name: String,
    /// The file it produced, or why it did not.
    pub outcome: Result<PathBuf, String>,
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

// ── The panel ───────────────────────────────────────────────────────────────

use eframe::egui::{self, RichText};

use crate::theme;
use crate::App;

const WIDTH: f32 = 560.0;

/// What a row's buttons asked for. Collected during the frame and applied
/// after it, because the rows borrow the queue they would otherwise mutate.
enum RowAction {
    Up(u64),
    Down(u64),
    Remove(u64),
}

/// The queue window, opened from the sidebar.
pub fn queue_modal(ctx: &egui::Context, app: &mut App) {
    if !app.show_queue {
        return;
    }
    let max_height = (ctx.content_rect().height() - 140.0).max(280.0);

    let response = egui::Modal::new(egui::Id::new("export-queue"))
        .frame(
            egui::Frame::NONE
                .fill(theme::panel())
                .corner_radius(theme::RADIUS)
                .inner_margin(egui::Margin::symmetric(26, 22))
                .stroke(egui::Stroke::new(1.0, theme::line())),
        )
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .auto_shrink([false, true])
                .show(ui, |ui| body(ui, app));
        });

    if response.should_close() {
        app.show_queue = false;
    }
}

fn body(ui: &mut egui::Ui, app: &mut App) {
    ui.set_width(WIDTH - 14.0);

    ui.label(
        RichText::new("Export queue")
            .size(theme::FONT_SUBHEAD)
            .color(theme::text()),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "Videos are exported one at a time, in this order. Anything you \
             change in the wizard belongs to the next video you add — the \
             jobs below already carry their own settings.",
        )
        .size(theme::FONT_SMALL)
        .color(theme::faint()),
    );
    ui.add_space(16.0);

    let mut action = None;

    // The running job sits at the top, as a printer queue puts the sheet in
    // the machine at the head of the list.
    if let Some(job) = &app.current_job {
        let (label, fraction, stopping) = match &app.export_job {
            Some(j) if j.stopping() => ("Stopping…".to_string(), j.fraction(), true),
            Some(j) => (j.label(), j.fraction(), false),
            None => (String::new(), 0.0, false),
        };
        if let Some(a) = running_row(ui, &job.source_name(), &label, fraction, stopping) {
            action = Some(a);
        }
        ui.add_space(8.0);
    }

    if app.queue.is_empty() && app.current_job.is_none() {
        theme::card().show(ui, |ui| {
            ui.set_width(WIDTH - 60.0);
            ui.label(
                RichText::new("Nothing queued.")
                    .size(theme::FONT_BODY)
                    .color(theme::text()),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Set a video up as usual, then use Add to queue on the \
                     Export page instead of Start export.",
                )
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
            );
        });
    }

    let last = app.queue.len().saturating_sub(1);
    let running = app.current_job.is_some();
    for (i, job) in app.queue.items().iter().enumerate() {
        if let Some(a) = waiting_row(ui, job, i, last, running) {
            action = Some(a);
        }
        ui.add_space(6.0);
    }

    // Applied after the rows, which borrowed the queue immutably.
    match action {
        Some(RowAction::Up(id)) => {
            app.queue.move_up(id);
            app.queue_notice = None;
        }
        Some(RowAction::Down(id)) => {
            app.queue.move_down(id);
            app.queue_notice = None;
        }
        Some(RowAction::Remove(id)) => app.remove_queued(id),
        None => {}
    }

    if let Some(notice) = app.queue_notice.clone() {
        ui.add_space(10.0);
        ui.label(
            RichText::new(notice)
                .size(theme::FONT_SMALL)
                .color(theme::accent()),
        );
    }

    if !app.queue_history.is_empty() {
        ui.add_space(18.0);
        ui.label(
            RichText::new("Finished")
                .size(theme::FONT_SECTION)
                .color(theme::text()),
        );
        ui.add_space(8.0);
        // Newest first: what just happened is what is being looked for.
        for done in app.queue_history.iter().rev() {
            ui.horizontal(|ui| {
                let (mark, color) = match &done.outcome {
                    Ok(_) => ("Done", theme::ok()),
                    Err(_) => ("Failed", theme::fix()),
                };
                ui.label(RichText::new(mark).size(theme::FONT_SMALL).color(color));
                ui.label(
                    RichText::new(&done.source_name)
                        .size(theme::FONT_SMALL)
                        .color(theme::text()),
                );
            });
            if let Err(why) = &done.outcome {
                ui.label(
                    RichText::new(first_line(why))
                        .size(theme::FONT_TINY)
                        .color(theme::faint()),
                );
            }
            ui.add_space(4.0);
        }
    }

    ui.add_space(18.0);
    if theme::secondary(ui, "Close").clicked() {
        app.show_queue = false;
    }
}

/// A failure message can run to several lines of tool output; a row wants the
/// sentence, not the transcript.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

fn running_row(
    ui: &mut egui::Ui,
    name: &str,
    label: &str,
    fraction: f32,
    stopping: bool,
) -> Option<RowAction> {
    let mut action = None;
    theme::card().show(ui, |ui| {
        ui.set_width(WIDTH - 60.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Running")
                    .size(theme::FONT_TINY)
                    .color(theme::accent()),
            );
            ui.label(RichText::new(name).size(theme::FONT_BODY).color(theme::text()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Removing the running job means stopping it, so the button
                // says so: the consequence is not a waiting row's.
                if !stopping && theme::ghost(ui, "Stop").clicked() {
                    action = Some(RowAction::Remove(RUNNING_JOB_ID));
                }
            });
        });
        ui.add_space(6.0);
        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_height(6.0)
                .corner_radius(3.0),
        );
        ui.add_space(5.0);
        ui.label(
            RichText::new(label)
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
        );
    });
    action
}

fn waiting_row(
    ui: &mut egui::Ui,
    job: &QueuedJob,
    index: usize,
    last: usize,
    running: bool,
) -> Option<RowAction> {
    let mut action = None;
    theme::card().show(ui, |ui| {
        ui.set_width(WIDTH - 60.0);
        ui.horizontal(|ui| {
            // Numbered from 2 when something is running, because the running
            // job is number 1 on the screen above it.
            let position = if running { index + 2 } else { index + 1 };
            ui.label(
                RichText::new(format!("{position}."))
                    .size(theme::FONT_SMALL)
                    .color(theme::faint()),
            );
            ui.label(
                RichText::new(job.source_name())
                    .size(theme::FONT_BODY)
                    .color(theme::text()),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::ghost(ui, "Remove").clicked() {
                    action = Some(RowAction::Remove(job.id));
                }
                // Disabled rather than hidden at the ends, so a row's
                // controls do not shift about as items are reordered.
                if ui.add_enabled(index < last, small_button("Down")).clicked() {
                    action = Some(RowAction::Down(job.id));
                }
                if ui.add_enabled(index > 0, small_button("Up")).clicked() {
                    action = Some(RowAction::Up(job.id));
                }
            });
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(job.summary_line())
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
        );
    });
    action
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
