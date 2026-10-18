//! The Trim screen.
//!
//! Cutting the leader and the tail off a capture: the blank tape before the
//! recording starts, the snow after it ends, and the minute of the camera
//! being put down at the end. Almost every tape has some.
//!
//! The deliberate limit here is **one kept range**. The config can hold
//! several and the engine renders them, but a multi-range editor is a
//! different and much larger thing — a timeline with handles, overlapping
//! selections, and a way to see what you have cut. One range with a start and
//! an end covers the leader-and-tail case that nearly everyone has, and it
//! can be read back and understood at a glance, which a list of ranges
//! cannot.
//!
//! Positions are held in **source frames**, not seconds, because that is what
//! the generated script trims on and what the audio is cut to match. Seconds
//! are a display convenience; converting at the edges would accumulate
//! rounding and let the audio drift from the picture.

use eframe::egui::{self, RichText};

use crate::check::COLUMN;
use crate::pages::PageAction;
use crate::theme;
use crate::App;

/// Width of the scrub frame. Small and fast: this is about recognising where
/// you are, not about judging the picture.
pub const SCRUB_WIDTH: usize = 480;

pub fn trim_page(ui: &mut egui::Ui, app: &mut App) -> PageAction {
    let action = PageAction::default();
    ui.set_max_width(COLUMN);

    theme::page_header(
        ui,
        "Trim",
        "This trims video from the start and/or the end. Drag the handles at          either end of the bar, or type a start frame and an end frame into          the boxes.",
    );
    ui.add_space(18.0);

    let Some(info) = app.report.as_ref().or(app.file_info.as_ref()).and_then(|r| r.info)
    else {
        theme::card().show(ui, |ui| {
            ui.set_width(COLUMN - 34.0);
            ui.label(
                RichText::new("Choose a video first.")
                    .size(theme::FONT_BODY)
                    .color(theme::muted()),
            );
        });
        return action;
    };

    let fps = if info.fps > 0.0 { info.fps } else { 25.0 };
    let total_frames = ((info.duration * fps).round() as i64).max(1);
    let inner = COLUMN - 34.0;

    // ── The frame ───────────────────────────────────────────────────────────
    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        scrub_picture(ui, app, inner);
        ui.add_space(12.0);
        scrubber(ui, app, inner, total_frames, fps);
    });

    ui.add_space(18.0);

    // ── What that leaves ────────────────────────────────────────────────────
    //
    // The controls used to live here as three buttons, with the timeline
    // above being a playhead and nothing more. They are now on the timeline
    // itself — draggable ends, and boxes under it for an exact frame — so
    // this card is left saying what the choice amounts to, which is the one
    // thing the controls cannot show while you are using them.
    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        let (start, end) = app.trim_bounds(total_frames);
        let kept = (end - start + 1).max(0);
        let cut = total_frames - kept;
        ui.label(
            RichText::new(format!(
                "Keeping {} to {} — {} of {}.",
                clock(start as f64 / fps),
                clock(end as f64 / fps),
                clock(kept as f64 / fps),
                clock(total_frames as f64 / fps),
            ))
            .size(theme::FONT_BODY)
            .color(theme::text()),
        );
        if cut > 0 {
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "{} will not be in the finished video.",
                    clock(cut as f64 / fps)
                ))
                .size(theme::FONT_SMALL)
                .color(theme::fix()),
            );
        }
    });

    ui.add_space(18.0);
    details(ui, app, total_frames, fps);

    action
}

fn scrub_picture(ui: &mut egui::Ui, app: &App, width: f32) {
    match &app.trim_frame {
        Some(texture) => {
            let size = texture.size_vec2();
            let scale = width / size.x;
            ui.add(
                egui::Image::new(texture)
                    .fit_to_exact_size(size * scale)
                    .corner_radius(theme::RADIUS_SMALL),
            );
        }
        None => {
            // Fetching one frame from the middle of a long capture takes a
            // moment — the decoder has to seek there. An empty grey rectangle
            // reads as "this is broken" rather than "this is working", so it
            // says what it is doing.
            let height = (width * 3.0 / 4.0).floor();
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
            if ui.is_rect_visible(rect) {
                let p = ui.painter();
                p.rect_filled(rect, theme::RADIUS_SMALL, theme::panel2());
                p.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Fetching that frame…",
                    egui::FontId::proportional(theme::FONT_BODY),
                    theme::muted(),
                );
            }
            ui.ctx().request_repaint();
        }
    }
}

/// What the pointer is doing to the timeline.
///
/// Stored on `App` across frames because a drag is not one event: the handle
/// is grabbed on one frame and moved on the next hundred, and without
/// remembering which handle was grabbed, a fast drag past the other one would
/// swap which is being moved mid-gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimelineDrag {
    #[default]
    None,
    Start,
    End,
    Playhead,
}

/// The timeline: cut material dimmed, kept material lit, both ends draggable.
///
/// A bare slider gave no sense of where the cut actually falls — the numbers
/// said one thing and the bar showed nothing. Seeing the two ends grayed out
/// is the whole point of the screen, and being able to take hold of them is
/// the point of this version: the ends used to be set by parking a playhead
/// and pressing a button, which is two gestures for one intention.
///
/// Returns true when a new frame should be fetched, which is on drag release
/// rather than on every pixel — a scrub that renders per pixel renders
/// nothing, because each render is cancelled by the next.
fn timeline(ui: &mut egui::Ui, app: &mut App, width: f32, total_frames: i64) -> bool {
    const HEIGHT: f32 = 34.0;
    /// How close the pointer has to be to a handle to take it rather than the
    /// bar. Generous: these are the two controls on the page that matter, and
    /// a handle you have to hit exactly is a handle you fight.
    const GRAB: f32 = 9.0;

    let (start, end) = app.trim_bounds(total_frames);
    let last = (total_frames - 1).max(1) as f32;

    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, HEIGHT), egui::Sense::click_and_drag());
    let x_for = |frame: i64| -> f32 {
        rect.left() + rect.width() * (frame as f32 / last).clamp(0.0, 1.0)
    };
    let frame_for = |x: f32| -> i64 {
        let t = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        (t * last).round() as i64
    };

    // Decide what is being grabbed, once, when the drag begins.
    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (ds, de) = ((pos.x - x_for(start)).abs(), (pos.x - x_for(end)).abs());
        app.timeline_drag = if ds <= GRAB && ds <= de {
            TimelineDrag::Start
        } else if de <= GRAB {
            TimelineDrag::End
        } else {
            TimelineDrag::Playhead
        };
    }

    let mut moved = false;
    if let Some(pos) = response.interact_pointer_pos()
        && (response.dragged() || response.clicked())
    {
        let drag = if response.clicked() {
            // A plain click moves the playhead; grabbing a handle needs a
            // drag, so a mis-click cannot silently retrim the video.
            TimelineDrag::Playhead
        } else {
            app.timeline_drag
        };
        let at = frame_for(pos.x);
        match drag {
            // The handles cannot cross: a start past the end would be an
            // empty range, and the engine would render nothing at all.
            TimelineDrag::Start => {
                app.set_trim_range(at.min(end - 1).max(0), end, total_frames);
            }
            TimelineDrag::End => {
                app.set_trim_range(start, at.max(start + 1).min(total_frames - 1), total_frames);
            }
            TimelineDrag::Playhead | TimelineDrag::None => {
                let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                if (t - app.trim_scrub).abs() > f32::EPSILON {
                    app.trim_scrub = t;
                    moved = true;
                }
            }
        }
    }
    if response.drag_stopped() {
        app.timeline_drag = TimelineDrag::None;
    }

    if ui.is_rect_visible(rect) {
        let p = ui.painter_at(rect);
        let x_of = |frame: i64| -> f32 {
            rect.left() + rect.width() * (frame as f32 / last).clamp(0.0, 1.0)
        };

        // Cut material first, as the ground.
        p.rect_filled(rect, theme::RADIUS_SMALL, theme::panel2());

        // Kept material, lit.
        let kept = egui::Rect::from_min_max(
            egui::pos2(x_of(start), rect.top()),
            egui::pos2(x_of(end).max(x_of(start) + 2.0), rect.bottom()),
        );
        p.rect_filled(kept, theme::RADIUS_SMALL, theme::accent_soft());
        p.rect_stroke(
            kept,
            theme::RADIUS_SMALL,
            egui::Stroke::new(1.0, theme::accent()),
            egui::StrokeKind::Inside,
        );

        // The two handles. Drawn as grips rather than hairlines, because a
        // line says "here is the edge" and a grip says "take hold of me" —
        // and the whole point of this page is that the edges can be moved.
        for frame in [start, end] {
            let x = x_of(frame).clamp(rect.left() + 3.0, rect.right() - 3.0);
            let grip = egui::Rect::from_min_max(
                egui::pos2(x - 3.0, rect.top() - 2.0),
                egui::pos2(x + 3.0, rect.bottom() + 2.0),
            );
            p.rect_filled(grip, 2.0, theme::accent());
            // Two notches, the universal sign for a draggable edge.
            for dy in [-3.0, 3.0] {
                p.line_segment(
                    [
                        egui::pos2(x - 1.5, grip.center().y + dy),
                        egui::pos2(x + 1.5, grip.center().y + dy),
                    ],
                    egui::Stroke::new(1.0, theme::bg()),
                );
            }
        }

        // The playhead.
        let head = rect.left() + rect.width() * app.trim_scrub.clamp(0.0, 1.0);
        p.line_segment(
            [
                egui::pos2(head, rect.top() - 3.0),
                egui::pos2(head, rect.bottom() + 3.0),
            ],
            egui::Stroke::new(2.0, theme::text()),
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    // Fetch the frame when the drag ends, not on every pixel of it.
    response.drag_stopped() || (response.clicked() && moved)
}

fn scrubber(ui: &mut egui::Ui, app: &mut App, width: f32, total_frames: i64, fps: f64) {
    let (start, end) = app.trim_bounds(total_frames);

    if timeline(ui, app, width, total_frames) {
        app.request_scrub_frame();
    }

    ui.add_space(10.0);
    frame_boxes(ui, app, total_frames, fps);

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let here = app.trim_at_frame(total_frames);
        ui.label(
            RichText::new(clock(here as f64 / fps))
                .size(theme::FONT_BODY)
                .color(theme::text()),
        );
        ui.add_space(14.0);
        let inside = here >= start && here <= end;
        ui.label(
            RichText::new(if inside {
                "inside the part you are keeping"
            } else {
                "inside the part being cut"
            })
            .size(theme::FONT_SMALL)
            .color(if inside { theme::ok() } else { theme::faint() }),
        );
        // A legend, because two shades of a dark theme are not self-evident.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new("gray = cut")
                    .size(theme::FONT_TINY)
                    .color(theme::faint()),
            );
            ui.add_space(10.0);
            ui.label(
                RichText::new("purple = kept")
                    .size(theme::FONT_TINY)
                    .color(theme::accent_hover()),
            );
        });
    });
}

/// The first and last kept frame, typed exactly.
///
/// A dragged handle is a pixel wide at best: on a two-hour tape one pixel of
/// a 790-pixel bar is about nine seconds, so the slider can get close and
/// nothing more. Anyone cutting to a specific frame — the moment before a
/// camera start, the last frame before tape noise — needs to type the number,
/// and the number is what the engine actually takes.
///
/// The clock beside each box is the same instant said the other way. Frames
/// are exact and unreadable; a timecode is readable and ambiguous at the
/// edges. Showing both costs a line and settles which is which.
fn frame_boxes(ui: &mut egui::Ui, app: &mut App, total_frames: i64, fps: f64) {
    let (start, end) = app.trim_bounds(total_frames);
    let last = (total_frames - 1).max(0);
    let (mut new_start, mut new_end) = (start, end);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 22.0;

        for (label, value, lo, hi) in [
            ("First frame kept", &mut new_start, 0, end - 1),
            ("Last frame kept", &mut new_end, start + 1, last),
        ] {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(label)
                        .size(theme::FONT_SMALL)
                        .color(theme::muted()),
                );
                ui.add_space(2.0);
                ui.add(
                    egui::DragValue::new(value)
                        .speed(1.0)
                        .range(lo.max(0)..=hi.max(0))
                        // Committed on Enter or focus loss rather than per
                        // keystroke: typing 1200 into a box that reacts to
                        // every digit would seek to frame 1, then 12, then
                        // 120, and each of those starts a render.
                        .update_while_editing(false),
                );
                ui.add_space(2.0);
                ui.label(
                    RichText::new(clock(*value as f64 / fps.max(0.001)))
                        .size(theme::FONT_TINY)
                        .color(theme::faint()),
                );
            });
        }

        ui.vertical(|ui| {
            ui.label(
                RichText::new("Kept")
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
            );
            ui.add_space(4.0);
            let frames = (new_end - new_start + 1).max(0);
            ui.label(
                RichText::new(format!("{frames} frames"))
                    .size(theme::FONT_BODY)
                    .color(theme::text()),
            );
            ui.label(
                RichText::new(format!("{} of {}", clock(frames as f64 / fps.max(0.001)), clock(total_frames as f64 / fps.max(0.001))))
                    .size(theme::FONT_TINY)
                    .color(theme::faint()),
            );
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::secondary(ui, "Keep all of it").clicked() {
                new_start = 0;
                new_end = last;
            }
            ui.add_space(8.0);
            // Trimming to where you are looking is the common case, and
            // doing it by eye on the bar then correcting in the box is two
            // steps for something the playhead already knows.
            let here = app.trim_at_frame(total_frames);
            if here > new_start && theme::secondary(ui, "End here").clicked() {
                new_end = here;
            }
            if here < new_end && theme::secondary(ui, "Start here").clicked() {
                new_start = here;
            }
        });
    });

    if (new_start, new_end) != (start, end) {
        app.set_trim_range(new_start, new_end, total_frames);
    }
}

fn details(ui: &mut egui::Ui, app: &mut App, total_frames: i64, fps: f64) {
    let mut open = app.is_open("trim-details");
    if theme::disclosure(ui, "Details", &mut open) {
        app.set_open("trim-details", open);
    }
    if !open {
        return;
    }
    let (start, end) = app.trim_bounds(total_frames);
    ui.add_space(8.0);
    egui::Frame::NONE
        .fill(theme::bg())
        .corner_radius(theme::RADIUS_SMALL)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(COLUMN - 58.0);
            for para in [
                format!("Keeping source frames {start} to {end} inclusive, of {total_frames} at {fps:.3} fps."),
                "The cut is made on source frames before anything else runs, so trimmed material is never deinterlaced or filtered — which is also why trimming makes the export faster rather than slower.".to_string(),
                "The audio is cut to the same ranges with atrim and concat, not left whole. A container is as long as its longest stream, so an untrimmed sound track would leave the file reporting the full running time with a frozen picture at the end. That is a bug this app had.".to_string(),
                "Frame numbers are clamped to the real clip length inside the generated script, because a container's declared duration is often slightly wrong on a tape capture and a frame number past the end would stop the render.".to_string(),
                "Only one kept range is offered here. The engine supports several, so a config written by hand can use them.".to_string(),
            ] {
                theme::paragraph(ui, RichText::new(para)
                            .size(theme::FONT_SMALL)
                            .color(theme::muted()),);
                ui.add_space(8.0);
            }
        });
}

// ── The arithmetic ──────────────────────────────────────────────────────────
//
// Free functions rather than methods on App, so they can be tested without
// building a whole application. Frame maths that is off by one is invisible
// on screen and wrong in the file.

/// The kept range in source frames, defaulting to the whole file.
///
/// Always returns a usable pair. A config carrying several ranges — which the
/// engine allows and this page does not author — reports the span from the
/// first start to the last end, so the screen describes something true rather
/// than silently showing only range one.
pub fn bounds(ranges: &[(i64, i64)], total_frames: i64) -> (i64, i64) {
    let last = (total_frames - 1).max(0);
    if ranges.is_empty() {
        return (0, last);
    }
    let start = ranges.iter().map(|r| r.0).min().unwrap_or(0);
    let end = ranges.iter().map(|r| r.1).max().unwrap_or(last);
    let start = start.clamp(0, last);
    (start, end.clamp(start, last))
}

/// The frame a scrubber at `pos` (0..1) is sitting on.
pub fn frame_at(pos: f32, total_frames: i64) -> i64 {
    let last = (total_frames - 1).max(0);
    let pos = pos.clamp(0.0, 1.0) as f64;
    ((pos * last as f64).round() as i64).clamp(0, last)
}

/// The ranges to store for a chosen start and end.
///
/// Empty when the selection covers everything. Storing an explicit
/// full-length range would work, but it makes the generated script carry a
/// trim that does nothing and the Export page claim a cut that is not
/// happening.
pub fn ranges_for(start: i64, end: i64, total_frames: i64) -> Vec<(i64, i64)> {
    let last = (total_frames - 1).max(0);
    let start = start.clamp(0, last);
    let end = end.clamp(start, last);
    if start == 0 && end == last {
        Vec::new()
    } else {
        vec![(start, end)]
    }
}

/// Seconds as m:ss.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOTAL: i64 = 750; // 25 seconds at 30 fps

    #[test]
    fn no_ranges_means_the_whole_file() {
        assert_eq!(bounds(&[], TOTAL), (0, 749));
    }

    /// Selecting everything must store nothing, or the script carries a trim
    /// that does nothing and Export claims a cut that is not happening.
    #[test]
    fn a_full_length_selection_stores_no_range() {
        assert!(ranges_for(0, 749, TOTAL).is_empty());
        assert!(ranges_for(0, 999, TOTAL).is_empty(), "clamped, then empty");
    }

    #[test]
    fn a_real_selection_round_trips() {
        let r = ranges_for(120, 600, TOTAL);
        assert_eq!(r, vec![(120, 600)]);
        assert_eq!(bounds(&r, TOTAL), (120, 600));
    }

    /// The last frame is total - 1. An off-by-one here asks the renderer for
    /// a frame past the end, which the script clamps but which would also
    /// make every duration on screen a frame too long.
    #[test]
    fn the_scrubber_reaches_the_last_frame_and_no_further() {
        assert_eq!(frame_at(0.0, TOTAL), 0);
        assert_eq!(frame_at(1.0, TOTAL), 749);
        assert_eq!(frame_at(1.5, TOTAL), 749, "clamped");
        assert_eq!(frame_at(-1.0, TOTAL), 0, "clamped");
    }

    #[test]
    fn an_end_before_its_start_is_not_allowed() {
        assert_eq!(ranges_for(500, 100, TOTAL), vec![(500, 500)]);
    }

    #[test]
    fn out_of_range_values_are_clamped_not_wrapped() {
        assert_eq!(ranges_for(-50, 10_000, TOTAL), Vec::new());
        assert_eq!(ranges_for(10_000, 10_000, TOTAL), vec![(749, 749)]);
    }

    /// A one-frame file must not produce a negative last frame.
    #[test]
    fn a_degenerate_file_does_not_underflow() {
        assert_eq!(bounds(&[], 1), (0, 0));
        assert_eq!(bounds(&[], 0), (0, 0));
        assert_eq!(frame_at(1.0, 0), 0);
    }

    /// Several ranges are legal in the config even though this page writes
    /// only one; the screen must describe the real span rather than range one.
    #[test]
    fn several_ranges_report_their_full_span() {
        assert_eq!(bounds(&[(600, 700), (100, 200)], TOTAL), (100, 700));
    }

    #[test]
    fn the_clock_reads_as_a_running_time() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(61.4), "1:01");
        assert_eq!(clock(3599.0), "59:59");
        // Never negative, whatever the arithmetic upstream produced.
        assert_eq!(clock(-5.0), "0:00");
    }
}

#[cfg(test)]
mod handle_tests {
    use super::*;

    /// The two handles must never cross. A start past the end is an empty
    /// range, and the generated script would render nothing at all — an
    /// export that "succeeds" with a zero-length file is the worst possible
    /// outcome of a slider being dragged too far.
    #[test]
    fn the_handles_cannot_cross() {
        let total = 1000;
        // What the timeline does when the start handle is dragged past the
        // end: clamped to one frame short of it.
        let end = 400;
        let dragged_to = 900;
        let start = dragged_to.min(end - 1).max(0);
        let ranges = ranges_for(start, end, total);
        assert_eq!(ranges, vec![(399, 400)]);
        let (s, e) = bounds(&ranges, total);
        assert!(s < e, "the range must keep at least two frames");
    }

    /// The same from the other side.
    #[test]
    fn the_end_handle_cannot_pass_the_start() {
        let total = 1000;
        let start = 600;
        let dragged_to = 100;
        let end = dragged_to.max(start + 1).min(total - 1);
        assert_eq!(ranges_for(start, end, total), vec![(600, 601)]);
    }

    /// Keeping everything is stored as no ranges at all, so an untrimmed
    /// export does not run the trim path for nothing.
    #[test]
    fn keeping_the_whole_file_stores_no_range() {
        assert!(ranges_for(0, 999, 1000).is_empty());
        assert_eq!(ranges_for(0, 998, 1000), vec![(0, 998)]);
        assert_eq!(ranges_for(1, 999, 1000), vec![(1, 999)]);
    }

    /// A typed frame number out of range is clamped rather than refused:
    /// the box is bounded, but a config loaded from JSON is not.
    #[test]
    fn frames_past_the_end_are_clamped() {
        assert_eq!(ranges_for(0, 99_999, 1000), Vec::new());
        assert_eq!(ranges_for(-50, 500, 1000), vec![(0, 500)]);
    }

    /// A one-frame file must not produce a backwards range.
    #[test]
    fn a_single_frame_file_does_not_underflow() {
        assert!(ranges_for(0, 0, 1).is_empty());
        let (s, e) = bounds(&[], 1);
        assert_eq!((s, e), (0, 0));
    }
}
