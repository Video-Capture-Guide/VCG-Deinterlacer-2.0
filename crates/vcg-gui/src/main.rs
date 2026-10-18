//! VCG Deinterlacer — the Rust interface.
//!
//! The shell plus Welcome, Choose video and Check video. The engine underneath
//! is the ported, parity-verified one in `vcg-core`, driven through
//! `vcg-tools`.

// No console window in release. A debug build keeps one so panics are visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod check;
mod crop;
mod dialog;
mod export;
mod findings;
mod flags;
mod nav;
mod pages;
mod settings;
mod trim;
mod watermark;
mod theme;
mod worker;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use eframe::egui;

use findings::Check;
use nav::Step;
use vcg_core::config::{Config, Mode};
use vcg_deps::manifest::CURRENT as DEPS_MANIFEST;
use vcg_deps::state::state as deps_state;
use vcg_tools::apply::{apply_report, options_for};
use vcg_tools::paths::ToolPaths;
use vcg_tools::probe::Report;
use worker::{
    ContainerJob, ExportJob, FrameJob, Job, PreviewJob, PreviewKind, ScrubJob, SettingPreviewJob,
};

use dialog::Dialog;

pub use vcg_core::VERSION;

/// Key for the stored technical-details preference.
const KEY_SHOW_TECHNICAL: &str = "show_technical";
/// Key for the stored progress-bar flag. Empty means the plain purple bar.
const KEY_FLAG: &str = "progress_flag";
/// Light / dark / match Windows.
const KEY_THEME: &str = "theme_choice";
/// How large the interface is drawn.
const KEY_TEXT_SIZE: &str = "text_size";

/// The front page's workflow diagram.
const FRONT_GRAPHIC: &[u8] = include_bytes!("../assets/front-graphic.png");

/// The window and taskbar icon, as raw RGBA.
///
/// Stored decoded rather than as a PNG so no image-decoding dependency is
/// needed for one picture that never changes. Made from the project's logo
/// with `ffmpeg -vf scale=256:256:flags=lanczos -pix_fmt rgba`, which is also
/// how to remake it if the logo is ever redrawn.
const ICON_RGBA: &[u8] = include_bytes!("../assets/icon-256.rgba");
const ICON_SIDE: u32 = 256;

/// Where a crash or a failed start is written down.
///
/// Beside the stored settings rather than beside the exe: the app may live in
/// Program Files or in a read-only folder, and a log nobody can write is worse
/// than no log because it fails silently in its turn.
fn crash_log_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("vcg-deinterlacer").join("last-error.txt"))
}

/// Write `text` where it can be found again, and return where that was.
fn write_crash_log(text: &str) -> Option<PathBuf> {
    let path = crash_log_path()?;
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(&path, text).ok()?;
    Some(path)
}

/// Tell the user, in the only way a windowless process can.
///
/// A release build is `windows_subsystem = "windows"`, so it has no console:
/// anything printed to stderr goes nowhere, and returning an error from `main`
/// exits with a code nobody sees. On a machine that cannot start the app at
/// all — no OpenGL 3.3, a missing runtime — that produced a process that
/// launched and vanished with no explanation, which is what this exists to
/// prevent. rfd is already a dependency for the file dialogs.
fn report_fatal(what: &str, detail: &str) {
    let logged = write_crash_log(&format!(
        "VCG Deinterlacer {}\n{}\n\n{}\n{}\n",
        vcg_core::VERSION_STRING,
        chrono_free_timestamp(),
        what,
        detail
    ));

    let mut body = format!("{what}\n\n{detail}");
    if let Some(path) = logged {
        body.push_str(&format!("\n\nWritten to:\n{}", path.display()));
    }

    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("VCG Deinterlacer could not start")
        .set_description(body)
        .show();
}

/// A timestamp without pulling in a date library for one line.
///
/// Seconds since the epoch is not friendly, but it orders correctly and is
/// unambiguous, which is all a crash log needs.
fn chrono_free_timestamp() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => format!("unix time {}", d.as_secs()),
        Err(_) => "unknown time".to_string(),
    }
}

fn main() -> eframe::Result {
    // `panic = "abort"` skips unwinding but still runs the hook first, so this
    // is the last chance to say anything at all about a panic in a build with
    // no console.
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        report_fatal(
            "The app stopped unexpectedly.",
            &format!("{info}\n\nat {location}"),
        );
    }));

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1180.0, 820.0])
        .with_min_inner_size([900.0, 600.0])
        .with_title("VCG Deinterlacer");

    // A wrong-sized buffer would be a cropped or garbled icon rather than an
    // error, so it is checked rather than trusted.
    if ICON_RGBA.len() == (ICON_SIDE * ICON_SIDE * 4) as usize {
        viewport = viewport.with_icon(std::sync::Arc::new(egui::IconData {
            rgba: ICON_RGBA.to_vec(),
            width: ICON_SIDE,
            height: ICON_SIDE,
        }));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let started = eframe::run_native(
        "vcg-deinterlacer",
        options,
        Box::new(|cc| {
            // Fonts are installed once; the palette is re-applied every frame
            // so the window follows Windows switching between light and dark
            // without a restart.
            theme::install_fonts(&cc.egui_ctx);
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(App::new(cc)))
        }),
    );

    // The failure this is really for is "no window appeared and nothing was
    // said". eframe draws through OpenGL 3.3, which a machine with only the
    // Microsoft Basic Display Adapter driver, a very old integrated GPU, or a
    // Remote Desktop session cannot provide — and the error for that arrives
    // here, on a process with nowhere to print it.
    if let Err(e) = &started {
        report_fatal(
            "The window could not be created.",
            &format!(
                "{e}\n\nThis is usually the graphics driver: the interface is drawn \
                 with OpenGL 3.3, which is not available over Remote Desktop or on a \
                 PC running Windows' basic display driver. Installing the graphics \
                 driver for your display adapter normally fixes it."
            ),
        );
    }

    started
}

/// Changes a page requests while it is being drawn.
///
/// Drawing takes `&App` in places — a card is rendered from immutable state —
/// so a click inside one is queued here and applied after the frame rather
/// than fought for with the borrow checker.
#[derive(Default)]
pub struct Pending {
    open: Vec<(String, bool)>,
    goto: Option<Step>,
    /// A native dialog to open once the frame is finished. Only the shell
    /// has the window handle a dialog needs as its owner — see `dialog`.
    dialog: Option<Dialog>,
}

pub struct App {
    step: Step,
    /// Steps the user has seen, for the sidebar ticks.
    visited: Vec<Step>,

    /// Which flag fills the progress bars, by name. Empty is the plain
    /// purple bar. Stored by name rather than by index so that inserting a
    /// flag into the middle of the list cannot silently change a saved
    /// choice to its neighbour's.
    pub flag: String,
    /// Decoded flag images, filled in as they are first drawn.
    pub flag_textures: flags::Textures,

    /// The app's own logo, for the front page. Decoded from the same raw
    /// RGBA the window icon uses, so there is still no image decoder in the
    /// build for one picture that never changes.
    logo: Option<egui::TextureHandle>,

    pub file: Option<PathBuf>,
    /// The quick container read, available before the full analysis runs.
    pub file_info: Option<Report>,
    pub thumbnail: Option<egui::TextureHandle>,
    /// Set when a newly chosen file still needs its container read.
    needs_file_probe: bool,
    /// That read, running in the background.
    container_job: Option<ContainerJob>,

    pub report: Option<Report>,
    pub job: Option<Job>,

    pub mode: Mode,
    pub config: Config,

    /// One checkbox at the bottom of Check video, rather than a segmented
    /// control competing with the heading for a minor option.
    pub show_technical: bool,
    /// The before/after pair, once rendered.
    pub preview: Option<(egui::TextureHandle, egui::TextureHandle)>,
    pub preview_job: Option<PreviewJob>,
    /// One whole frame at native size, for the crop overlay.
    pub crop_frame: Option<egui::TextureHandle>,
    pub crop_frame_job: Option<FrameJob>,

    pub export_job: Option<ExportJob>,
    /// The finished export, kept so the page can describe it after the job
    /// has gone. `Err` holds the reason, for the details panel.
    pub export_done: Option<Result<export::ExportSummary, String>>,
    /// The side crops the user actually asked for, before the aspect
    /// compensation is folded into them.
    ///
    /// Kept here rather than in `Config` because it is bookkeeping for one
    /// screen, not a setting the engine reads — and `Config` is the surface
    /// the parity harness compares, which should not grow a field the Python
    /// has no equivalent for.
    pub crop_sides_base: Option<(u32, u32)>,
    /// A folder the user picked. `None` means alongside the source.
    export_folder: Option<PathBuf>,
    /// Write a log of the run beside the finished video.
    pub save_log: bool,
    /// Write the settings as JSON beside the finished video.
    pub save_settings: bool,
    /// Write a short side-by-side comparison clip beside the finished video.
    pub save_comparison: bool,
    /// Write the VapourSynth script beside the finished video.
    ///
    /// Asked for alongside the metadata description: the sidecar JSON says
    /// what was chosen, and this says what was actually run. They are not the
    /// same document — the script is the only artifact that can be opened in
    /// another tool, compared against a Hybrid script, or edited and run by
    /// hand.
    pub save_script: bool,
    /// Whether the Stop link on the export page has been pressed, which
    /// reveals the confirmation rather than stopping anything.
    pub export_stop_armed: bool,
    /// Whether the flag picker on the export page has been unfolded. It
    /// starts closed: the bar is the thing being watched, and a row of
    /// controls under it is noise until somebody goes looking for it.
    pub flag_picker_open: bool,
    /// What the last settings-file load said, so the page can confirm it or
    /// explain why nothing changed.
    pub settings_loaded: Option<Result<String, String>>,

    /// Where the Trim scrubber sits, as a fraction of the running time.
    pub trim_scrub: f32,
    /// Which timeline handle the pointer took hold of, for the length of the
    /// drag. See `trim::TimelineDrag`.
    pub timeline_drag: trim::TimelineDrag,
    pub trim_frame: Option<egui::TextureHandle>,
    trim_job: Option<ScrubJob>,
    /// The moment a frame has been asked for, so a late arrival for a
    /// position the user has already left can be dropped.
    trim_wanted: Option<f64>,

    /// Settings-page previews, keyed by what they are of.
    ///
    /// A cache rather than one slot because a before/after pair needs both
    /// halves at once, and the untouched half does not change when a
    /// strength does — re-rendering it on every click would double the wait
    /// for no new information. Bounded, and cleared with the file.
    previews: HashMap<String, egui::TextureHandle>,
    /// The pixels behind those previews, kept so a scope can be computed
    /// from the very frame on screen rather than from one fetched
    /// separately — which on a cut would be a different shot entirely.
    preview_pixels: HashMap<String, vcg_tools::preview::Image>,
    /// Drawn scopes, cached because they are rebuilt from whole frames and
    /// nothing about one changes until its frame does.
    scopes: HashMap<String, egui::TextureHandle>,
    /// Which scope the Color tint page is showing.
    pub scope_choice: crate::settings::Scope,
    /// How far the difference view is amplified.
    ///
    /// Separate from the vectorscope's gain: they answer different questions
    /// and a reader setting one does not mean the other.
    pub diff_gain: f32,
    /// How far the vectorscope is magnified about its center.
    ///
    /// One field, not one per half, because a before and an after drawn at
    /// different gains cannot be compared — which is the only reason the
    /// page shows two.
    pub scope_gain: f64,
    /// True while the moment slider is held.
    scrubbing: bool,
    /// Set when the step changes, so the next frame starts at the top.
    scroll_to_top: bool,
    setting_preview_job: Option<SettingPreviewJob>,
    /// Which moment every settings preview is taken from, as a fraction of
    /// the running time. One setting for all of them, so moving it compares
    /// the same moment on every page.
    pub preview_at: f32,
    /// Magnification of the comparison panes, 1 to 8.
    ///
    /// Several of these differences — dither banding most of all — are
    /// invisible at a quarter of the frame width, so the picture has to be
    /// enlargeable or the comparison cannot answer its own question.
    pub zoom: f32,
    /// Where the zoom is centered, in 0..1 of the frame.
    pub zoom_at: egui::Vec2,
    /// Which disclosures the user has opened by hand.
    open: HashSet<String>,
    pub pending: RefCell<Pending>,

    tools: ToolPaths,
    pub deps_ready: bool,
    pub deps_summary: String,
    pub tool_lines: Vec<String>,

    /// Whether the About panel is open. Not a `Step`: it is not part of the
    /// walk from a file to an export, and putting it in the sidebar's
    /// numbering would misdescribe the shape of the job.
    /// Light, dark, or whatever Windows is doing.
    pub theme_choice: theme::ThemeChoice,
    /// How large the whole interface is drawn.
    pub text_size: theme::TextSize,
    /// The front page's workflow diagram, decoded on first use.
    front_graphic: Option<Option<egui::TextureHandle>>,
    pub show_about: bool,
    pub update_state: about::UpdateState,
    /// The release check, running off the UI thread.
    update_rx: Option<std::sync::mpsc::Receiver<vcg_deps::update::Outcome>>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let tools = ToolPaths::discover(None);
        let deps_dir = tools
            .deps_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("_deps"));
        let state = deps_state(&deps_dir, &DEPS_MANIFEST);

        // Which engine is in use, and where it was found. This machine has
        // three VapourSynth installations and the Python app picks among them
        // silently, which makes a bug report unanswerable.
        let mut tool_lines = Vec::new();
        for (label, tool) in [
            ("ffmpeg", &tools.ffmpeg),
            ("ffprobe", &tools.ffprobe),
            ("VapourSynth", &tools.vspipe),
        ] {
            match tool {
                Some(t) => tool_lines.push(format!("{label}: {} ({})", short(&t.path), t.origin)),
                None => tool_lines.push(format!("{label}: not found")),
            }
        }

        let flag = cc
            .storage
            .and_then(|s| s.get_string(KEY_FLAG))
            .filter(|name| flags::find(name).is_some())
            .unwrap_or_default();

        let show_technical = cc
            .storage
            .and_then(|s| s.get_string(KEY_SHOW_TECHNICAL))
            .map(|v| v == "true")
            .unwrap_or(false);

        let theme_choice = cc
            .storage
            .and_then(|s| s.get_string(KEY_THEME))
            .map(|v| theme::ThemeChoice::from_str(&v))
            .unwrap_or_default();

        let text_size = cc
            .storage
            .and_then(|s| s.get_string(KEY_TEXT_SIZE))
            .map(|v| theme::TextSize::from_str(&v))
            .unwrap_or_default();

        let logo = (ICON_RGBA.len() == (ICON_SIDE * ICON_SIDE * 4) as usize).then(|| {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [ICON_SIDE as usize, ICON_SIDE as usize],
                ICON_RGBA,
            );
            cc.egui_ctx
                .load_texture("logo", image, egui::TextureOptions::LINEAR)
        });

        Self {
            front_graphic: None,
            theme_choice,
            text_size,
            show_about: false,
            update_state: about::UpdateState::default(),
            update_rx: None,
            step: Step::Welcome,
            visited: vec![Step::Welcome],
            flag,
            flag_textures: flags::Textures::default(),
            logo,
            file: None,
            file_info: None,
            thumbnail: None,
            needs_file_probe: false,
            container_job: None,
            report: None,
            job: None,
            // Deinterlacing is the tool's job; enhancement is opt-in.
            mode: Mode::EnhanceAutomatically,
            config: Config::default(),
            show_technical,
            preview: None,
            preview_job: None,
            crop_frame: None,
            crop_frame_job: None,
            export_job: None,
            export_done: None,
            crop_sides_base: None,
            export_folder: None,
            // Off by default. A folder of videos with two extra files beside
            // each one is a cost, and most people want the video.
            save_log: false,
            save_settings: false,
            save_comparison: false,
            save_script: false,
            export_stop_armed: false,
            flag_picker_open: false,
            settings_loaded: None,
            trim_scrub: 0.0,
            timeline_drag: trim::TimelineDrag::None,
            trim_frame: None,
            trim_job: None,
            trim_wanted: None,
            previews: HashMap::new(),
            preview_pixels: HashMap::new(),
            scopes: HashMap::new(),
            scope_choice: crate::settings::Scope::Vectorscope,
            // Eight, because the changes worth hunting for are the ones a
            // few levels wide; at unity they are black.
            diff_gain: 8.0,
            // Two, because a tape's cast is small: at unity the cloud is a
            // smudge on the center of an empty circle.
            scope_gain: 2.0,
            scrubbing: false,
            scroll_to_top: false,
            setting_preview_job: None,
            preview_at: 0.45,
            zoom: 1.0,
            zoom_at: egui::vec2(0.5, 0.5),
            open: HashSet::new(),
            pending: RefCell::new(Pending::default()),
            deps_ready: state.is_ready() && tools.can_probe(),
            deps_summary: state.describe(&DEPS_MANIFEST),
            tool_lines,
            tools,
        }
    }

    // ── Disclosure state ────────────────────────────────────────────────────

    /// Whether a disclosure is open. Detailed view opens them all.
    pub fn is_open(&self, key: &str) -> bool {
        self.open.contains(key)
    }

    pub fn set_open(&mut self, key: &str, open: bool) {
        if open {
            self.open.insert(key.to_string());
        } else {
            self.open.remove(key);
        }
    }

    /// Queue a disclosure change from a context that only has `&self`.
    pub fn queue_open(&self, key: &str, open: bool) {
        self.pending
            .borrow_mut()
            .open
            .push((key.to_string(), open));
    }

    pub fn queue_goto(&self, step: Step) {
        self.pending.borrow_mut().goto = Some(step);
    }

    /// Ask the shell to open a native dialog after this frame.
    ///
    /// Queued rather than opened here because a dialog needs an owner window
    /// to come up in front of the app, and the handle for that is on
    /// `eframe::Frame`, which pages are not given.
    pub fn queue_dialog(&self, which: Dialog) {
        self.pending.borrow_mut().dialog = Some(which);
    }

    /// The front-page logo, if it decoded.
    pub fn logo(&self) -> Option<&egui::TextureHandle> {
        self.logo.as_ref()
    }

    /// The workflow diagram on the front page, decoded on first use.
    ///
    /// Lazy like the flags: it is one picture on one screen, and decoding a
    /// quarter-megabyte PNG at startup to show it a second later is a cost
    /// paid by every launch including the ones that go straight past it.
    pub fn front_graphic(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if self.front_graphic.is_none() {
            self.front_graphic = Some(crate::flags::decode_png(
                ctx,
                "front-graphic",
                FRONT_GRAPHIC,
            ));
        }
        self.front_graphic.clone().flatten()
    }

    /// A progress bar filled with whichever flag is chosen.
    ///
    /// One function for every bar in the app, so the analysis bar and the
    /// export bar can never disagree about what the setting means.
    pub fn progress_bar(&mut self, ui: &mut egui::Ui, fraction: f32, width: f32, height: f32) {
        let flag = flags::find(&self.flag);
        flags::progress_bar(ui, &mut self.flag_textures, flag, fraction, width, height);
    }

    pub fn set_show_technical(&mut self, show: bool) {
        self.show_technical = show;
    }

    // ── File ────────────────────────────────────────────────────────────────

    pub fn set_file(&mut self, path: PathBuf) {
        if self.file.as_deref() == Some(path.as_path()) {
            return;
        }
        // The generated script reads its source from the config, so this has
        // to be kept in step with the chosen file.
        self.config.input_path = path.display().to_string();
        self.file = Some(path);
        // Nothing measured about the old file may be shown against the new one.
        self.file_info = None;
        self.thumbnail = None;
        self.container_job = None;
        self.preview = None;
        self.preview_job = None;
        self.crop_frame = None;
        self.crop_frame_job = None;
        self.export_done = None;
        self.export_folder = None;
        self.trim_scrub = 0.0;
        self.trim_frame = None;
        self.trim_job = None;
        self.trim_wanted = None;
        self.previews.clear();
        self.preview_pixels.clear();
        self.scopes.clear();
        self.setting_preview_job = None;
        self.report = None;
        self.job = None;
        let input_path = self.config.input_path.clone();
        self.config = Config {
            input_path,
            ..Config::default()
        };
        self.needs_file_probe = true;
    }

    pub fn clear_file(&mut self) {
        self.file = None;
        self.file_info = None;
        self.thumbnail = None;
        self.container_job = None;
        self.preview = None;
        self.preview_job = None;
        self.crop_frame = None;
        self.crop_frame_job = None;
        self.export_done = None;
        self.export_folder = None;
        self.trim_scrub = 0.0;
        self.trim_frame = None;
        self.trim_job = None;
        self.trim_wanted = None;
        self.previews.clear();
        self.preview_pixels.clear();
        self.scopes.clear();
        self.setting_preview_job = None;
        self.report = None;
        self.job = None;
        self.config = Config::default();
        self.needs_file_probe = false;
    }

    /// One ffprobe plus one frame grab, so the file card can show real
    /// metadata the moment a file is chosen.
    ///
    /// Started, not run: both are child processes and neither belongs on the
    /// thread that is drawing. See `worker::ContainerJob`.
    fn start_container_probe(&mut self) {
        self.needs_file_probe = false;
        let Some(file) = self.file.clone() else { return };
        if !self.tools.can_probe() {
            return;
        }
        self.container_job = Some(ContainerJob::start(self.tools.clone(), file, 160));
    }

    // ── Mode and analysis ───────────────────────────────────────────────────

    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        if let Some(report) = &self.report {
            apply_report(&mut self.config, report, mode);
        } else if mode == Mode::JustDeinterlace {
            self.config.clear_enhancements();
        }
        if !self.step.visible_in(mode) {
            self.step = Step::CheckVideo;
        }
    }

    /// Start analyzing, reusing a cached result for the same file and mode.
    fn start_probe(&mut self) {
        let Some(file) = self.file.clone() else { return };
        if !self.tools.can_probe() {
            return;
        }
        self.report = None;
        self.job = Some(Job::start(self.tools.clone(), file, options_for(self.mode)));
    }

    /// Analyze unless an up-to-date result is already in hand.
    fn ensure_analyzed(&mut self) {
        if self.report.is_some() || self.job.is_some() {
            return;
        }
        self.start_probe();
    }

    /// Render the before/after comparison in the background.
    ///
    /// Needs the portable VapourSynth, so it is skipped when only a system
    /// install is available — the plugin paths baked into the script point at
    /// the bundled payload.
    fn start_preview(&mut self) {
        if self.preview.is_some() || self.preview_job.is_some() {
            return;
        }
        let Some(file) = self.file.clone() else { return };
        let Some(info) = self.report.as_ref().and_then(|r| r.info) else {
            return;
        };
        let Some(vs) = self.tools.vs_deps_dir.clone() else {
            return;
        };
        self.preview_job = Some(PreviewJob::start(
            self.tools.clone(),
            vcg_core::vpy::DepsPaths::new(vs),
            self.config.clone(),
            file,
            info,
            // A region big enough to read at half the content column.
            (360, 240),
        ));
    }

    /// Fetch one whole frame for the Crop page overlay.
    ///
    /// Needs only ffmpeg, unlike the before/after comparison, so it runs even
    /// when the bundled VapourSynth is unavailable.
    fn start_crop_frame(&mut self) {
        if self.crop_frame.is_some() || self.crop_frame_job.is_some() {
            return;
        }
        let Some(file) = self.file.clone() else { return };
        let Some(info) = self
            .report
            .as_ref()
            .and_then(|r| r.info)
            .or_else(|| self.file_info.as_ref().and_then(|r| r.info))
        else {
            return;
        };
        if !self.tools.can_probe() {
            return;
        }
        self.crop_frame_job = Some(FrameJob::start(
            self.tools.clone(),
            self.config.clone(),
            file,
            info,
        ));
    }

    fn apply_fix(&mut self, check: Check, on: bool) {
        if let Some(report) = self.report.clone() {
            check.set(&mut self.config, &report, on);
        }
    }

    // ── Trim ────────────────────────────────────────────────────────────────

    /// The kept range in source frames. See `trim::bounds`.
    pub fn trim_bounds(&self, total_frames: i64) -> (i64, i64) {
        trim::bounds(&self.config.trim_ranges, total_frames)
    }

    /// The frame the scrubber is sitting on.
    pub fn trim_at_frame(&self, total_frames: i64) -> i64 {
        trim::frame_at(self.trim_scrub, total_frames)
    }

    /// Set the kept range, from a handle, a box or a button.
    ///
    /// One route in, so every way of changing the trim is ordered and
    /// clamped identically — a handle dragged past its partner and a number
    /// typed past it have to behave the same, and they did not when each
    /// control did its own arithmetic.
    ///
    /// A range covering the whole file is stored as no ranges at all, which
    /// is what "do not trim" means to the generator: writing an explicit
    /// 0..last would make every untrimmed export run the trim path for
    /// nothing.
    pub fn set_trim_range(&mut self, start: i64, end: i64, total_frames: i64) {
        let last = (total_frames - 1).max(0);
        let start = start.clamp(0, last);
        let end = end.clamp(start, last);
        self.config.trim_ranges = trim::ranges_for(start, end, total_frames);
    }

    /// Set the kept range, collapsing to "keep everything" when it is.
    /// Ask for the frame under the scrubber.
    pub fn request_scrub_frame(&mut self) {
        let Some(file) = self.file.clone() else { return };
        let Some(info) = self
            .report
            .as_ref()
            .or(self.file_info.as_ref())
            .and_then(|r| r.info)
        else {
            return;
        };
        if !self.tools.can_probe() {
            return;
        }
        let at = self.trim_scrub as f64 * info.duration.max(0.0);
        // Already showing it, or already fetching it.
        if self.trim_wanted == Some(at) {
            return;
        }
        self.trim_wanted = Some(at);
        self.trim_job = Some(ScrubJob::start(
            self.tools.clone(),
            self.config.clone(),
            file,
            info,
            at,
            crate::trim::SCRUB_WIDTH,
        ));
    }

    // ── Settings previews ───────────────────────────────────────────────────

    /// The frame size the finished file will have, for rendering a preview
    /// at the size the setting will actually apply at.
    fn preview_size(&self) -> (usize, usize) {
        let (w, h) = self.source_size().unwrap_or((720, 480));
        let (ow, oh) = crate::crop::output_size(&self.config, w, h);
        (ow.max(16) as usize, oh.max(16) as usize)
    }

    /// Ask for a settings preview, unless the same one is already in hand.
    pub fn request_setting_preview(&mut self, tag: String, kind: PreviewKind) {
        if self.previews.contains_key(&tag) || self.scrubbing {
            return;
        }
        if self
            .setting_preview_job
            .as_ref()
            .is_some_and(|j| j.tag == tag)
        {
            return;
        }
        let Some(file) = self.file.clone() else { return };
        let Some(info) = self
            .report
            .as_ref()
            .or(self.file_info.as_ref())
            .and_then(|r| r.info)
        else {
            return;
        };
        if !self.tools.can_probe() {
            return;
        }
        let at = info.duration * self.preview_at.clamp(0.0, 1.0) as f64;
        let out = self.preview_size();
        self.setting_preview_job = Some(SettingPreviewJob::start(
            self.tools.clone(),
            self.config.clone(),
            file,
            at,
            out,
            kind,
            tag,
        ));
    }

    /// Move every settings preview to a different moment.
    ///
    /// Clears the cache, because every picture in it is of the old moment —
    /// keeping them would leave one page showing a frame another page had
    /// moved on from.
    pub fn set_preview_at(&mut self, at: f32) {
        self.preview_at = at.clamp(0.0, 1.0);
    }

    /// Whether the moment slider is being dragged right now.
    ///
    /// Requests are held off while it is, because every intermediate value
    /// would otherwise start its own pair of VapourSynth renders for a frame
    /// nobody will look at.
    pub fn set_scrubbing(&mut self, scrubbing: bool) {
        self.scrubbing = scrubbing;
    }

    /// The moment, as a key fragment.
    ///
    /// Every cached preview and scope carries this, so moving the slider
    /// cannot show a frame from where the slider used to be. The previous
    /// version cleared the caches instead and had a hole in it: during a
    /// drag the value was written straight to the field, so by the time the
    /// clear ran on release the value already matched and the clear returned
    /// early — leaving every page showing the old moment forever, and the
    /// two halves of a comparison showing different scenes.
    pub fn moment_key(&self) -> String {
        format!("@{:.4}", self.preview_at)
    }

    /// The preview for `tag`, if it has been rendered.
    pub fn setting_preview_for(&self, tag: &str) -> Option<&egui::TextureHandle> {
        self.previews.get(tag)
    }

    /// The pixels behind a preview, for building a scope from them.
    pub fn preview_rgb_for(&self, tag: &str) -> Option<&vcg_tools::preview::Image> {
        self.preview_pixels.get(tag)
    }

    /// What to say under a pane that is still rendering, if this is the one.
    ///
    /// `None` once the picture is there, or when a different preview is the
    /// one in flight — only the pane actually waiting should be captioned.
    ///
    /// The wording changes with elapsed time rather than ticking a bar,
    /// because the slow part is not the frame: `LWLibavSource` indexes the
    /// whole file before it can seek, which is why a two-hour tape takes so
    /// much longer than a two-minute one. Saying so is more use than a bar
    /// with nothing behind it, and it stops the first long wait reading as a
    /// hang.
    pub fn preview_progress_for(&self, tag: &str) -> Option<String> {
        if self.previews.contains_key(tag) {
            return None;
        }
        let job = self.setting_preview_job.as_ref()?;
        if job.tag != tag || job.finished {
            return None;
        }
        let secs = job.started.elapsed().as_secs();
        let long = self
            .report
            .as_ref()
            .or(self.file_info.as_ref())
            .and_then(|r| r.info)
            .is_some_and(|i| i.duration > 600.0);

        Some(match secs {
            0..=3 => "Rendering this frame…".to_string(),
            4..=14 if long => format!(
                "Reading the video… {secs}s. A long recording has to be indexed \
                 before any moment in it can be reached."
            ),
            4..=14 => format!("Rendering this frame… {secs}s."),
            _ => format!(
                "Still working… {}m {:02}s. Deinterlacing one frame means looking \
                 at the frames either side of it, and the first one also has to \
                 index the file.",
                secs / 60,
                secs % 60
            ),
        })
    }

    pub fn scope_texture(&self, key: &str) -> Option<&egui::TextureHandle> {
        self.scopes.get(key)
    }

    /// Upload a drawn scope and keep it until its frame changes.
    pub fn store_scope(
        &mut self,
        key: &str,
        img: vcg_core::analysis::scopes::ScopeImage,
        ctx: &egui::Context,
    ) {
        if self.scopes.len() > 12 {
            self.scopes.clear();
        }
        let image = egui::ColorImage::from_rgb([img.width, img.height], &img.rgb);
        // Nearest: a scope is a density plot, and smoothing it invents
        // samples that were never measured.
        self.scopes.insert(
            key.to_string(),
            ctx.load_texture(key, image, egui::TextureOptions::NEAREST),
        );
    }

    /// Ask for a before/after pair of one filter stage.
    ///
    /// Returns the two tags so the page can draw whichever halves exist.
    /// Only one render runs at a time, and the untouched half is asked for
    /// first: it is shared by every strength, so it is the one worth having
    /// cached before the user starts clicking through them.
    pub fn request_stage_pair(
        &mut self,
        stage: vcg_core::vpy::PreviewStage,
        strength: &str,
    ) -> (String, String) {
        let m = self.moment_key();
        // One untouched half for every page: with no stage applied the
        // script is the same whichever stage asked for it, so rendering it
        // per page would be the same picture computed five times.
        let before = format!("stage-base{m}");
        let after = format!("stage-{}-on-{strength}{m}", stage.id());
        if self.scrubbing {
            return (before, after);
        }

        let Some(info) = self
            .report
            .as_ref()
            .or(self.file_info.as_ref())
            .and_then(|r| r.info)
        else {
            return (before, after);
        };
        let Some(vs) = self.tools.vs_deps_dir.clone() else {
            return (before, after);
        };
        let deps = vcg_core::vpy::DepsPaths::new(vs);

        let (tag, apply) = if self.previews.contains_key(&before) {
            (after.clone(), true)
        } else {
            (before.clone(), false)
        };
        self.request_setting_preview(
            tag,
            PreviewKind::Stage {
                stage,
                apply,
                deps,
                info,
            },
        );
        (before, after)
    }

    // ── Export ──────────────────────────────────────────────────────────────

    pub fn source_size(&self) -> Option<(u32, u32)> {
        self.report
            .as_ref()
            .or(self.file_info.as_ref())
            .and_then(|r| r.info)
            .map(|i| (i.width, i.height))
    }

    /// Where the finished file will be written.
    ///
    /// Write the log and the settings sidecar, if either was asked for.
    ///
    /// Both land beside the finished video with its own stem, so a folder of
    /// exports stays sorted together rather than interleaving three
    /// alphabetical runs. Returns what was actually written, so the finished
    /// page can say so rather than claim it.
    ///
    /// A failure here is reported and never fatal: the video is the thing
    /// that took an hour, and losing the run over an unwritable sidecar
    /// would be absurd.
    fn write_companions(
        &self,
        outcome: Option<&vcg_tools::encode::Outcome>,
        took: f64,
    ) -> Vec<PathBuf> {
        let mut written = Vec::new();
        let Some(o) = outcome else { return written };
        if !self.save_log && !self.save_settings && !self.save_script {
            return written;
        }

        let stem = o
            .output
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "export".to_string());
        let dir = o.output.parent().map(PathBuf::from).unwrap_or_default();
        let source_name = self
            .file
            .as_ref()
            .and_then(|f| f.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let findings = self.report.as_ref().map(findings_text);

        if self.save_settings {
            let json = vcg_core::provenance::sidecar_json(
                &self.config,
                VERSION,
                &source_name,
                findings.as_deref(),
            );
            let path = dir.join(format!("{stem}.settings.json"));
            if std::fs::write(&path, json).is_ok() {
                written.push(path);
            }
        }

        // The script that was run, verbatim. Generated from the same config
        // the export used and by the same function, so it cannot describe a
        // pipeline other than the one that produced the file beside it.
        //
        // Needs the deps root for the plugin-loading block — without one the
        // script would be written with that block missing, which is a script
        // that does not run. Better to write nothing and say so.
        if self.save_script
            && let Some(vs) = self.tools.vs_deps_dir.clone()
        {
            let script =
                vcg_core::vpy::generate_vpy(&self.config, &vcg_core::vpy::DepsPaths::new(vs));
            let path = dir.join(format!("{stem}.vpy"));
            if std::fs::write(&path, script).is_ok() {
                written.push(path);
            }
        }

        if self.save_log {
            let text = vcg_core::provenance::log_text(vcg_core::provenance::LogEntry {
                config: &self.config,
                version: VERSION,
                source_name: &source_name,
                output_name: &o.output.file_name().unwrap_or_default().to_string_lossy(),
                started: &now_stamp(),
                seconds: took,
                findings: findings.as_deref(),
                tool_output: &o.tool_output,
                mode: Some(self.mode),
            });
            let path = dir.join(format!("{stem}.log.txt"));
            if std::fs::write(&path, text).is_ok() {
                written.push(path);
            }
        }
        written
    }

    /// Read a settings file written by an earlier export and adopt it whole.
    ///
    /// The point of the feature is the next tape off the same deck: one
    /// click instead of walking sixteen pages again. So it replaces the
    /// settings outright rather than merging — a half-applied preset is
    /// worse than none, because nothing on screen would say which half.
    ///
    /// Two things survive it. The input path, because the script reads its
    /// source from there and it must stay the file the user has open. And
    /// the facts measured from *this* file — field order above all — because
    /// those are not settings, they are observations of the tape in hand.
    /// See `apply::restore_source_facts`.
    fn load_settings_file(&mut self, path: &std::path::Path) {
        let read = std::fs::read_to_string(path)
            .map_err(|e| format!("that file could not be read ({e})"))
            .and_then(|text| vcg_core::provenance::load_sidecar(&text));

        match read {
            Ok(loaded) => {
                let input_path = self.config.input_path.clone();
                self.config = loaded.config;
                self.config.input_path = input_path;
                if let Some(report) = &self.report {
                    vcg_tools::apply::restore_source_facts(&mut self.config, report);
                }
                // Every rendered preview was made with the old settings.
                self.previews.clear();
                self.preview_pixels.clear();
                self.scopes.clear();
                self.setting_preview_job = None;
                self.crop_sides_base = None;
                self.settings_loaded = Some(Ok(match loaded.source {
                    Some(name) => format!("Settings loaded from {name}."),
                    None => "Settings loaded.".to_string(),
                }));
            }
            Err(why) => self.settings_loaded = Some(Err(why)),
        }
    }

    /// Versioned against the destination folder, not the source folder — the
    /// name has to be free where the file actually lands.
    pub fn output_path(&self) -> Option<PathBuf> {
        let source = self.file.as_ref()?;
        let dir = match &self.export_folder {
            Some(dir) => dir.clone(),
            None => source.parent().map(PathBuf::from).unwrap_or_default(),
        };
        Some(export::output_in(&dir, source, &self.config))
    }

    /// Re-derive the destination after the format changed, so the extension
    /// never contradicts the container.
    pub fn retarget_output(&mut self) {}

    /// Where a dialog should start, so the picker opens somewhere useful
    /// rather than wherever it was left last time.
    fn dialog_start(&self, which: Dialog) -> Option<PathBuf> {
        match which {
            Dialog::OutputFolder | Dialog::LoadSettings => self
                .output_path()
                .and_then(|p| p.parent().map(std::path::Path::to_path_buf)),
            Dialog::Source => self
                .file
                .as_ref()
                .and_then(|p| p.parent().map(std::path::Path::to_path_buf)),
            Dialog::Logo => None,
        }
    }

    /// Open a queued dialog and act on what came back.
    ///
    /// Runs after the frame is drawn, from the one place that holds the
    /// window handle the dialog needs as its owner.
    fn run_dialog(&mut self, which: Dialog, frame: &eframe::Frame) {
        let start = self.dialog_start(which);
        let Some(path) = dialog::open(which, frame, start) else {
            return;
        };
        match which {
            Dialog::Source => self.set_file(path),
            Dialog::OutputFolder => self.export_folder = Some(path),
            Dialog::Logo => {
                if let vcg_core::config::Watermark::Logo { logo_path, .. } =
                    &mut self.config.watermark
                {
                    *logo_path = path.display().to_string();
                }
            }
            Dialog::LoadSettings => self.load_settings_file(&path),
        }
    }

    /// Whether the export can start at all.
    pub fn can_export(&self) -> bool {
        self.export_blocker().is_empty()
    }

    /// Why it cannot, in one sentence, or empty when it can.
    ///
    /// Checked before the button rather than discovered by a failed render:
    /// the user would otherwise wait through a long encode to be told the
    /// engine was never there.
    pub fn export_blocker(&self) -> &'static str {
        if self.file.is_none() {
            return "Choose a video first.";
        }
        if self.report.is_none() {
            return "Run the check on the Check video page first.";
        }
        if self.tools.vspipe.is_none() {
            return "VapourSynth was not found, so the video cannot be rebuilt.";
        }
        if self.tools.ffmpeg.is_none() {
            return "FFmpeg was not found.";
        }
        if self.tools.vs_deps_dir.is_none() {
            return "The bundled VapourSynth files were not found.";
        }
        ""
    }

    fn start_export(&mut self) {
        if self.export_job.is_some() || !self.can_export() {
            return;
        }
        let (Some(file), Some(output)) = (self.file.clone(), self.output_path()) else {
            return;
        };
        let Some(info) = self.report.as_ref().and_then(|r| r.info) else {
            return;
        };
        let Some(vs) = self.tools.vs_deps_dir.clone() else {
            return;
        };
        self.export_done = None;
        self.export_job = Some(ExportJob::start(
            self.tools.clone(),
            vcg_core::vpy::DepsPaths::new(vs),
            self.config.clone(),
            file,
            output,
            info.duration,
            info.fps,
            self.save_comparison,
        ));
    }

    /// Back to the start, keeping the tool paths and the mode.
    pub fn reset_for_another(&mut self) {
        self.export_done = None;
        self.clear_file();
        self.step = Step::ChooseVideo;
    }

    /// Ask GitHub about newer releases, off the UI thread.
    ///
    /// A check already running is left alone rather than restarted: the
    /// button is the only way in, and double-clicking it should not open two
    /// connections.
    pub fn start_update_check(&mut self) {
        if self.update_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // The receiver is dropped if the window closes mid-check, which
            // makes this send fail. That is the normal way this thread ends
            // and is not worth reporting.
            let _ = tx.send(vcg_deps::update::check(vcg_core::VERSION));
        });
        self.update_rx = Some(rx);
        self.update_state = about::UpdateState::Checking;
    }

    /// Collect the check's answer, if it has arrived.
    fn poll_update_check(&mut self) {
        let Some(rx) = &self.update_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(outcome) => {
                self.update_state = about::UpdateState::Done(outcome);
                self.update_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // The worker died without sending, which should not happen;
                // say so rather than spinning forever.
                self.update_state = about::UpdateState::Done(
                    vcg_deps::update::Outcome::Failed("the check did not finish".into()),
                );
                self.update_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    fn visit(&mut self, step: Step) {
        if !self.visited.contains(&step) {
            self.visited.push(step);
        }
        if self.step != step {
            self.scroll_to_top = true;
        }
        self.step = step;
        // Moving onto Check video triggers the analysis, so Next is the only
        // control needed rather than a separate Analyze button.
        if step == Step::CheckVideo {
            self.ensure_analyzed();
        }
        if step == Step::Crop {
            self.start_crop_frame();
        }
        if step == Step::Trim && self.trim_frame.is_none() {
            self.request_scrub_frame();
        }
    }

    /// Accept files dropped anywhere on the window.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(path) = dropped.into_iter().find(|p| p.is_file()) {
            self.set_file(path);
            self.visit(Step::ChooseVideo);
        }
    }

    /// Whether the forward action is available on this step.
    fn can_advance(&self) -> bool {
        match self.step {
            Step::Welcome => self.deps_ready,
            // Analysis runs on Next, so a file is all that is required.
            Step::ChooseVideo => self.file.is_some(),
            Step::CheckVideo => self.report.is_some(),
            _ => self.file.is_some(),
        }
    }
}

/// Shorten a path for display, keeping the last two components.
fn short(path: &std::path::Path) -> String {
    let parts: Vec<_> = path.components().collect();
    if parts.len() <= 2 {
        return path.display().to_string();
    }
    let tail: PathBuf = parts[parts.len() - 2..].iter().collect();
    format!("…{}{}", std::path::MAIN_SEPARATOR, tail.display())
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(
            KEY_SHOW_TECHNICAL,
            if self.show_technical { "true" } else { "false" }.to_string(),
        );
        storage.set_string(KEY_FLAG, self.flag.clone());
        storage.set_string(KEY_THEME, self.theme_choice.as_str().to_string());
        storage.set_string(KEY_TEXT_SIZE, self.text_size.as_str().to_string());
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Cheap, and the only way the window follows the system theme while
        // it is open rather than only at startup.
        theme::apply_with(&ctx, self.theme_choice, self.text_size);
        self.take_dropped_files(&ctx);

        if self.needs_file_probe {
            self.start_container_probe();
        }

        // Collect the container read and its thumbnail.
        if let Some(job) = &mut self.container_job {
            let wrong_file = self.file.as_deref() != Some(job.file.as_path());
            if let Some((report, thumb)) = job.poll() {
                if !wrong_file {
                    if let Some(t) = thumb {
                        let image = egui::ColorImage::from_rgb([t.width, t.height], &t.rgb);
                        self.thumbnail =
                            Some(ctx.load_texture("thumbnail", image, egui::TextureOptions::LINEAR));
                    }
                    self.file_info = Some(*report);
                }
                self.container_job = None;
            } else if !job.finished {
                // Nothing else on this page animates, so without this the
                // chips would appear only when the user next moved the mouse.
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        // Collect a finished analysis before anything draws, so the page
        // renders the result in the frame it arrives.
        if let Some(job) = &mut self.job
            && let Some(report) = job.poll()
        {
            // Guard against a result for a file the user has since changed.
            if self.file.as_deref() == Some(job.file.as_path()) {
                apply_report(&mut self.config, &report, self.mode);
                self.report = Some(*report);
                // The analysis is on screen now; the picture follows.
                self.start_preview();
            }
        }

        // Collect a finished render and upload it.
        if let Some(job) = &mut self.preview_job
            && let Some(comparison) = job.poll()
            && self.file.as_deref() == Some(job.file.as_path()) {
                let to_texture = |name: &str, img: &vcg_tools::preview::Image| {
                    let image =
                        egui::ColorImage::from_rgb([img.width, img.height], &img.rgb);
                    // Nearest-neighbor: smoothing the comb lines away would
                    // defeat the entire purpose of the comparison.
                    ctx.load_texture(name, image, egui::TextureOptions::NEAREST)
                };
                self.preview = Some((
                    to_texture("preview-before", &comparison.before),
                    to_texture("preview-after", &comparison.after),
                ));
            }

        // Collect the crop frame. Nearest-neighbor again: a smoothed edge
        // would blur the very boundary the overlay is drawn against.
        if let Some(job) = &mut self.crop_frame_job
            && let Some(img) = job.poll()
            && self.file.as_deref() == Some(job.file.as_path())
        {
            let image = egui::ColorImage::from_rgb([img.width, img.height], &img.rgb);
            self.crop_frame =
                Some(ctx.load_texture("crop-frame", image, egui::TextureOptions::NEAREST));
        }

        // Collect a finished export. The job is dropped and its result kept,
        // so the page can describe the file long after the thread has gone.
        if let Some(job) = &mut self.export_job
            && let Some(result) = job.poll()
        {
            let took = job.started.elapsed().as_secs_f64();
            // Written now, while the config and the findings are still the
            // ones the export used. Doing it on the button press instead
            // would let a setting changed afterwards be recorded as though it
            // had been applied, which is worse than having no record at all.
            let mut companions = self.write_companions(result.as_ref().ok(), took);
            // The clip is written by the encoder, not here, so it is added
            // to the list rather than produced by it — but it is listed the
            // same way, because to the reader it is one more file that
            // appeared beside the video.
            if let Ok(o) = &result
                && let Some(clip) = &o.comparison
            {
                companions.push(clip.clone());
            }
            self.export_done = Some(result.map(|o| {
                let bytes = std::fs::metadata(&o.output).map(|m| m.len()).unwrap_or(0);
                export::ExportSummary {
                    output: o.output,
                    duration: o.duration,
                    had_audio: o.had_audio,
                    took,
                    bytes,
                    companions,
                }
            }));
            self.export_job = None;
            self.export_stop_armed = false;
        }

        // Collect a scrub frame, discarding one for a position already left.
        if let Some(job) = &mut self.trim_job {
            let stale = self.trim_wanted != Some(job.at)
                || self.file.as_deref() != Some(job.file.as_path());
            if let Some(img) = job.poll() {
                if !stale {
                    let image =
                        egui::ColorImage::from_rgb([img.width, img.height], &img.rgb);
                    self.trim_frame = Some(ctx.load_texture(
                        "trim-frame",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
                self.trim_job = None;
            }
        }

        // Collect a settings preview.
        if let Some(job) = &mut self.setting_preview_job {
            let wrong_file = self.file.as_deref() != Some(job.file.as_path());
            if let Some(img) = job.poll() {
                if !wrong_file {
                    let image =
                        egui::ColorImage::from_rgb([img.width, img.height], &img.rgb);
                    // Bounded: a slider dragged through its range would
                    // otherwise hold a texture for every value it passed.
                    if self.previews.len() > 16 {
                        self.previews.clear();
                    }
                    self.previews.insert(
                        job.tag.clone(),
                        ctx.load_texture(&job.tag, image, egui::TextureOptions::LINEAR),
                    );
                    if self.preview_pixels.len() > 16 {
                        self.preview_pixels.clear();
                    }
                    self.preview_pixels.insert(job.tag.clone(), *img);
                }
                self.setting_preview_job = None;
            }
        }

        let has_file = self.file.is_some();

        // The release check runs on its own thread; pick up its answer and
        // keep the frame loop alive while it is still out.
        self.poll_update_check();
        if self.update_rx.is_some() {
            ui.ctx().request_repaint();
        }

        egui::Panel::left("nav")
            .exact_size(205.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(theme::sidebar()))
            .show(ui, |ui| {
                match nav::sidebar(ui, self.step, &self.visited, has_file, self.mode) {
                    nav::SidebarAction::Goto(step) => self.visit(step),
                    nav::SidebarAction::OpenAbout => self.show_about = true,
                    nav::SidebarAction::None => {}
                }
            });

        let mut action = pages::PageAction::default();

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme::bg()))
            .show(ui, |ui| {
                // The footer is laid out first, so the body takes the rest of
                // the height and scrolls inside it.
                egui::Panel::bottom("footer")
                    .frame(
                        egui::Frame::NONE
                            .fill(theme::bg())
                            .inner_margin(egui::Margin {
                                left: 32,
                                right: 32,
                                top: 14,
                                bottom: 14,
                            }),
                    )
                    .show_separator_line(true)
                    .show(ui, |ui| {
                        if let Some(step) = self.footer(ui) {
                            action.go_to = Some(step);
                        }
                    });

                // One scroll area for every page. Without it a page taller
                // than the window hides its lower half with no way to reach it.
                // A new page starts at its top. Carrying the previous page's
                // scroll position over drops the reader into the middle of
                // something they have not seen the start of — and the pages
                // with scopes on them are tall enough that the heading can
                // be off screen entirely.
                let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
                if self.scroll_to_top {
                    area = area.vertical_scroll_offset(0.0);
                    self.scroll_to_top = false;
                }
                let page = area
                    .show(ui, |ui| {
                        egui::Frame::NONE
                            .inner_margin(egui::Margin {
                                left: 32,
                                right: 32,
                                top: 26,
                                bottom: 26,
                            })
                            .show(ui, |ui| {
                                // Long lines are hard to read; cap the column.
                                ui.set_max_width(980.0);
                                match self.step {
                                    Step::Welcome => pages::welcome(ui, self),
                                    Step::ChooseVideo => pages::choose_video(ui, self),
                                    Step::CheckVideo => check::check_video(ui, self),
                                    Step::Crop => crop::crop_page(ui, self),
                                    Step::Export => export::export_page(ui, self),
                                    Step::Trim => trim::trim_page(ui, self),
                                    Step::Watermark => watermark::watermark_page(ui, self),
                                    Step::Audio => settings::audio_page(ui, self),
                                    Step::ColorBleeding => settings::bleeding_page(ui, self),
                                    other => match settings::settings_page(ui, self, other) {
                                        Some(a) => a,
                                        None => pages::placeholder(ui, other),
                                    },
                                }
                            })
                            .inner
                    })
                    .inner;

                if page.go_to.is_some() {
                    action.go_to = page.go_to;
                }
                action.start_probe |= page.start_probe;
                action.start_export |= page.start_export;
                if page.set_mode.is_some() {
                    action.set_mode = page.set_mode;
                }
                if page.set_fix.is_some() {
                    action.set_fix = page.set_fix;
                }
            });

        // Drawn last so it sits over the sidebar and the footer as well as
        // the page; a modal that only covers the body is not modal.
        about::about_modal(ui.ctx(), self);

        // ── Apply everything the frame asked for ────────────────────────────
        let pending = std::mem::take(&mut *self.pending.borrow_mut());
        for (key, open) in pending.open {
            self.set_open(&key, open);
        }

        // Mode first: it decides which probes a following analysis runs.
        if let Some(mode) = action.set_mode {
            self.set_mode(mode);
        }
        if let Some((check, on)) = action.set_fix {
            self.apply_fix(check, on);
        }
        if let Some(step) = action.go_to.or(pending.goto) {
            self.visit(step);
        }
        if action.start_probe {
            self.start_probe();
        }
        if action.start_export {
            self.start_export();
        }
        // Last, after everything else this frame asked for. A dialog blocks
        // until it is answered, so opening one earlier would stall the rest.
        if let Some(which) = pending.dialog {
            self.run_dialog(which, frame);
        }
    }
}

impl App {
    /// Back, a ghost action, the step counter, and the one forward button.
    fn footer(&mut self, ui: &mut egui::Ui) -> Option<Step> {
        let mut go = None;
        let step = self.step;
        let mode = self.mode;

        ui.horizontal(|ui| {
            if let Some(prev) = step.prev_in(mode) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    let response = theme::secondary(ui, "   Back");
                    // The arrow is drawn into the button's own rect, because
                    // the bundled fonts have no arrow glyph.
                    let r = egui::Rect::from_center_size(
                        egui::pos2(response.rect.left() + 20.0, response.rect.center().y),
                        egui::vec2(14.0, 14.0),
                    );
                    if ui.is_rect_visible(r) {
                        theme::icon::arrow(ui.painter(), r, theme::text(), theme::icon::Dir::Left);
                    }
                    if response.clicked() {
                        go = Some(prev);
                    }
                });
            }

            // Re-checking belongs here, not as a second primary on the page.
            if step == Step::CheckVideo && self.report.is_some() {
                let response = theme::ghost(ui, "      Scan again");
                let r = egui::Rect::from_center_size(
                    egui::pos2(response.rect.left() + 24.0, response.rect.center().y),
                    egui::vec2(15.0, 15.0),
                );
                if ui.is_rect_visible(r) {
                    theme::icon::refresh(ui.painter(), r, theme::muted());
                }
                if response.clicked() {
                    self.start_probe();
                }
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(next) = step.next_in(mode) {
                    let label = format!("Next: {}    ", next.title());
                    let enabled = self.can_advance();
                    let response = theme::primary(ui, &label, enabled);
                    let r = egui::Rect::from_center_size(
                        egui::pos2(response.rect.right() - 22.0, response.rect.center().y),
                        egui::vec2(15.0, 15.0),
                    );
                    if ui.is_rect_visible(r) {
                        theme::icon::arrow(
                            ui.painter(),
                            r,
                            if enabled {
                                theme::text()
                            } else {
                                theme::faint()
                            },
                            theme::icon::Dir::Right,
                        );
                    }
                    if response.clicked() {
                        go = Some(next);
                    }
                    if !enabled && step == Step::ChooseVideo {
                        response.on_hover_text("Choose a video first");
                    }
                }
                ui.add_space(10.0);
                let counter = step.counter_label(mode);
                if !counter.is_empty() {
                    ui.label(
                        egui::RichText::new(counter)
                            .size(theme::FONT_SMALL)
                            .color(theme::faint()),
                    );
                }
            });
        });

        go
    }
}

/// The findings as plain lines, for the log and the settings sidecar.
///
/// Built from the same probe report the wizard showed, so a log cannot
/// disagree with what was on screen when the choices were made.
fn findings_text(report: &Report) -> String {
    let mut lines = Vec::new();
    if let Some(l) = &report.levels {
        lines.push(format!(
            "levels: Y {} to {}{}",
            l.min_y.unwrap_or(0),
            l.max_y.unwrap_or(255),
            if l.needs_adjustment {
                " (outside the legal 16-235)"
            } else {
                " (inside the legal 16-235)"
            }
        ));
    }
    if let Some(c) = &report.color {
        lines.push(match c.color_cast {
            Some(cast) => format!(
                "color: {cast:?} cast, U {:+.1} V {:+.1}",
                c.u_correction, c.v_correction
            ),
            None => "color: no consistent cast".to_string(),
        });
    }
    if let Some(n) = &report.noise {
        lines.push(format!(
            "noise: {:?}, {:.2}% of pixels flickering, {} samples",
            n.noise_level,
            n.avg_variance * 100.0,
            n.samples_analyzed
        ));
    }
    if let Some(h) = &report.halo {
        lines.push(format!(
            "halos: {:?}, {:.0}% of strong edges overshoot",
            h.halo_level,
            h.halo_ratio * 100.0
        ));
    }
    if let Some(b) = &report.bleeding {
        lines.push(format!(
            "color bleeding: {:?}, score {:.0}",
            b.bleed_level, b.bleeding_score
        ));
    }
    if let Some(t) = &report.telecine {
        lines.push(if t.progressive_source {
            "scan: progressive already".to_string()
        } else if t.pattern.is_some() {
            "scan: film, pulldown removed".to_string()
        } else {
            format!(
                "scan: interlaced, {} TFF / {} BFF frames measured",
                t.tff_frames, t.bff_frames
            )
        });
    }
    if let Some(a) = &report.audio {
        lines.push(format!("sound: {:?}", a.verdict));
    }
    for w in &report.warnings {
        lines.push(format!("warning: {w}"));
    }
    lines.join("\n")
}

/// The current local time, for the log's header.
///
/// Formatted by hand from the system clock rather than pulling in a date
/// library for one line. UTC, and labeled as such, because an unlabeled
/// local timestamp in a log that may be emailed is worse than no timestamp.
fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let (h, m, s) = (secs % 86_400 / 3600, secs % 3600 / 60, secs % 60);

    // Civil-from-days, the standard algorithm; correct across leap years and
    // centuries, which a naive 365.25 is not.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mth <= 2 { y + 1 } else { y };

    format!("{year:04}-{mth:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
}
