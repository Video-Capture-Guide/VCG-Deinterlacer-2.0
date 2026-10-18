//! The Crop screen.
//!
//! The question this page answers for a newcomer is not "what crop preset do
//! you want", which is unanswerable without knowing what blanking is. It is
//! "here is the black edge we found on your tape, shall we remove it" — with
//! the frame on screen and the proposed cut drawn over it, so the answer is
//! visible rather than numeric.
//!
//! The recommendation comes from `vcg_core::analysis::edges`, measured off the
//! user's own capture. That matters because the presets are symmetric and real
//! capture-card blanking usually is not: on the sample used to develop this,
//! the measured crop is 14 columns a side where the standard preset takes 8.
//!
//! Numbers live behind Details, and the four manual values behind "Set it
//! myself", following the same rule as Check video.

use eframe::egui::{self, RichText};

use vcg_core::analysis::edges::{Detection, EdgeCrop, SAFETY_MARGIN};
use vcg_core::config::{CaptureMethod, Config, CropPreset, VideoFormat};

use crate::check::COLUMN;
use crate::pages::PageAction;
use crate::theme::{self, Tag};
use crate::App;

/// How the four values on screen were arrived at.
///
/// Derived from the config rather than stored, so it cannot disagree with what
/// the pipeline will actually do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    Measured,
    Standard,
    Nothing,
    Manual,
}

pub fn crop_page(ui: &mut egui::Ui, app: &mut App) -> PageAction {
    let action = PageAction::default();
    ui.set_max_width(COLUMN);

    theme::page_header(ui, "Crop the edges", "");
    ui.add_space(14.0);

    let measured = match app.report.as_ref().and_then(|r| r.edges) {
        Some(Detection::Found(c)) => Some(c),
        _ => None,
    };
    let why_not = match app.report.as_ref().and_then(|r| r.edges) {
        Some(Detection::Inconclusive(why)) => Some(why),
        _ => None,
    };

    // ── ① What we found ─────────────────────────────────────────────────────
    crate::check::section_number(ui, 1, "What we found");
    ui.add_space(10.0);
    finding_card(ui, app, measured, why_not);
    ui.add_space(22.0);

    // ── ② The picture ───────────────────────────────────────────────────────
    crate::check::section_number(ui, 2, "Your video, with the crop marked");
    ui.add_space(10.0);
    picture_card(ui, app);
    ui.add_space(22.0);

    // ── ③ What to do ────────────────────────────────────────────────────────
    //
    // One decision, not two. The band used to have its own toggle above this,
    // which meant the bottom could be cropped by the band *and* again by the
    // manual value, with neither control showing the total.
    crate::check::section_number(ui, 3, "What to crop");
    ui.add_space(10.0);
    choice_card(ui, app, measured);

    action
}

/// The current choice, read back from the config.
fn current_choice(c: &Config, measured: Option<EdgeCrop>) -> Choice {
    match c.crop_preset {
        Some(CropPreset::None) => Choice::Nothing,
        Some(CropPreset::Bt601) | Some(CropPreset::Overscan) | None => Choice::Standard,
        Some(CropPreset::Manual) => {
            // Compared against the recommendation *with its margin*, which is
            // what gets written when the option is picked.
            let same = measured.is_some_and(|m| {
                let r = m.with_margin(SAFETY_MARGIN);
                (r.left, r.right, r.top, r.bottom)
                    == (c.crop_left, c.crop_right, c.crop_top, c.crop_bottom)
            });
            if same {
                Choice::Measured
            } else {
                Choice::Manual
            }
        }
    }
}

/// What the pipeline will actually remove, for the overlay.
///
/// `bottom` and `band` are the two halves of one crop, split apart only so the
/// overlay can shade them differently: the user should be able to see which
/// part of the bottom strip is the flickering band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Strips {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    /// Bottom crop that is not the band.
    pub bottom: u32,
    /// The part of the bottom crop that is the flickering band.
    pub band: u32,
}

impl Strips {
    pub fn any(&self) -> bool {
        self.left + self.right + self.top + self.bottom + self.band > 0
    }
}

pub fn effective_strips(c: &Config) -> Strips {
    let (left, right, top, bottom) = match c.effective_crop_preset() {
        CropPreset::None => (0, 0, 0, 0),
        CropPreset::Bt601 => (8, 8, 0, 0),
        // The vertical half of the overscan preset is applied after
        // deinterlacing, but it still ends up cropped, so the overlay shows it.
        CropPreset::Overscan => (8, 8, 2, 4),
        CropPreset::Manual => (c.crop_left, c.crop_right, c.crop_top, c.crop_bottom),
    };
    // The band is part of the bottom crop, never added to it. Only a manual
    // crop carries one; the presets have no idea where the band is.
    let band = if c.effective_crop_preset() == CropPreset::Manual {
        c.head_switching_rows.min(bottom)
    } else {
        0
    };
    Strips {
        left,
        right,
        top,
        bottom: bottom - band,
        band,
    }
}

/// The extra columns the pipeline takes off the sides to hold the shape.
///
/// Mirrors `keep_aspect` in the .vpy generator gate for gate, because this is
/// drawn on the user's picture as a promise about what will be removed. The
/// generator only trims when rows have come off the top or bottom, only on a
/// manual crop, and only when it is going to resize onto a fixed target —
/// the overscan preset sets a pixel aspect instead, and an HD source is never
/// touched. Get any of those wrong here and the overlay shades picture that
/// is actually kept.
///
/// Split half and half, which is what `_vcg_ax // 2` does: the trim is always
/// a multiple of four, so both sides are whole and equal.
pub fn aspect_extra(c: &Config, source_w: u32, source_h: u32) -> (u32, u32) {
    if !c.keep_aspect
        || !c.par_correction
        || c.is_hd_source()
        || c.effective_crop_preset() != CropPreset::Manual
        || c.crop_top + c.crop_bottom == 0
    {
        return (0, 0);
    }
    let st = effective_strips(c);
    let kept_w = source_w.saturating_sub(st.left + st.right);
    let kept_h = source_h.saturating_sub(st.top + st.bottom + st.band);
    let target_h = if c.format == VideoFormat::Pal { 576 } else { 480 };
    let trim = Config::aspect_trim(kept_w, kept_h, target_h, c.reference_width());
    (trim / 2, trim - trim / 2)
}

/// The frame size the finished file will have.
///
/// The PAR-correction target when the pipeline resizes, and the kept region
/// itself when it does not — an HD source, or PAR correction switched off.
pub fn output_size(c: &Config, source_w: u32, source_h: u32) -> (u32, u32) {
    if c.par_correction && !c.is_hd_source() {
        return if c.format == VideoFormat::Pal {
            (768, 576)
        } else {
            (640, 480)
        };
    }
    // No resize: what comes out is the kept region, shape trim included.
    let st = effective_strips(c);
    let kept_w = source_w.saturating_sub(st.left + st.right);
    let kept_h = source_h.saturating_sub(st.top + st.bottom + st.band);
    (kept_w, kept_h)
}

// ── ① ───────────────────────────────────────────────────────────────────────

/// The standing explanation, shown whatever the measurement found.
///
/// Worth stating plainly and once: a tape capture *always* looks like this,
/// nothing is wrong with the user's file, and the arithmetic has been done for
/// them. Wording supplied by the user, so changes to it are their call.
fn intro(ui: &mut egui::Ui, config: &Config) {
    // What a source does at its edges depends on how it was digitised, and
    // the same paragraph cannot be true of all three. Telling a DV camcorder
    // owner their file "usually has black bars from the capture card" is
    // wrong about a file that never met one.
    //
    // There used to be a second paragraph under this, naming SAR and PAR and
    // saying we had "did all the math". It is gone: the reader does not have
    // to be told the arithmetic was done, and the result of that arithmetic
    // is on screen a few lines further down in the numbers and the overlay.
    let edges = if config.is_hd_source() {
        "This is HD camcorder footage, not a tape capture. It has no capture-card \
         overscan at the sides and no switching noise at the bottom, because the \
         picture was digital from the moment it was recorded."
    } else if config.capture_method == CaptureMethod::Dv {
        "MiniDV and Digital8 camcorders did not create black bars at the side or \
         switching noise at the bottom. That means you shouldn't crop this video."
    } else {
        "Videos captured from sources like VHS and Video8 usually have black bars on the \
         left and right, and switching noise at the bottom. That's totally normal. Most \
         users prefer to crop that away for clean edges. (Although some expert users \
         prefer to keep it, for reasons we won't get into here.)"
    };

    theme::paragraph(ui, RichText::new(edges)
                .size(theme::FONT_BODY)
                .color(theme::muted()),);
}

fn finding_card(
    ui: &mut egui::Ui,
    app: &mut App,
    measured: Option<EdgeCrop>,
    why_not: Option<&'static str>,
) {
    let inner = COLUMN - 34.0;

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        intro(ui, &app.config);
        ui.add_space(14.0);

        match (measured, why_not) {
            // Something to remove.
            (Some(c), _) if !c.with_margin(SAFETY_MARGIN).is_nothing() => {
                theme::tag(ui, Tag::FixRecommended, Some("Recommended"));
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Here is what we found on your video")
                        .size(theme::FONT_SECTION)
                        .color(theme::text()),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(plain_description(&c))
                        .size(theme::FONT_BODY)
                        .color(theme::muted()),
                );
                if c.is_lopsided() {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("The border is wider on one side than the other, which is normal — it depends on how your capture device was lined up. A fixed setting cannot follow that, so these numbers were measured from your own file.")
                            .size(theme::FONT_BODY)
                            .color(theme::muted()),
                    );
                }
                ui.add_space(12.0);
                details(ui, app, Some(&c), None);
            }

            // Measured, and there is nothing there.
            (Some(_), _) => {
                theme::tag(ui, Tag::LooksGood, Some("Nothing to do"));
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Your picture already reaches the edge")
                        .size(theme::FONT_SECTION)
                        .color(theme::text()),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("We looked along all four edges and found no black border and no switching noise. Nothing will be cropped, so none of your picture is thrown away.")
                        .size(theme::FONT_BODY)
                        .color(theme::muted()),
                );
                ui.add_space(12.0);
                details(ui, app, Some(&EdgeCrop::default()), None);
            }

            // Could not tell.
            (None, Some(why)) => {
                theme::tag(ui, Tag::Info, Some("Could not measure"));
                ui.add_space(8.0);
                ui.label(
                    RichText::new("We could not check the edges on this file")
                        .size(theme::FONT_SECTION)
                        .color(theme::text()),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("The reason was: {why}. The usual setting is being used instead, which trims a small amount from the left and right. You can change it below if it looks wrong on the picture."))
                        .size(theme::FONT_BODY)
                        .color(theme::muted()),
                );
                ui.add_space(12.0);
                details(ui, app, None, Some(why));
            }

            // The analysis has not run.
            (None, None) => {
                ui.label(
                    RichText::new("The edges have not been checked yet. Go back to Check video and run the check, and a measured crop will appear here.")
                        .size(theme::FONT_BODY)
                        .color(theme::muted()),
                );
            }
        }
    });
}

/// One plain sentence per edge that has something on it.
fn plain_description(c: &EdgeCrop) -> String {
    let mut parts: Vec<String> = Vec::new();
    if c.left > 0 {
        parts.push(format!("{} pixels on the left", c.left));
    }
    if c.right > 0 {
        parts.push(format!("{} on the right", c.right));
    }
    if c.top > 0 {
        parts.push(format!("{} along the top", c.top));
    }
    let blanking_bottom = c.bottom.saturating_sub(c.noise_rows);
    if blanking_bottom > 0 {
        parts.push(format!("{blanking_bottom} along the bottom"));
    }
    if c.noise_rows > 0 {
        parts.push(format!("{} of noise at the very bottom", c.noise_rows));
    }

    let list = match parts.len() {
        0 => return "There is nothing at the edges to remove.".to_string(),
        1 => parts[0].clone(),
        _ => {
            let last = parts.pop().unwrap();
            format!("{}, and {last}", parts.join(", "))
        }
    };
    format!(
        "Your capture recorded a little past the edge of the picture: {list}. \
         Removing it means the black does not end up in your finished video."
    )
}

fn details(
    ui: &mut egui::Ui,
    app: &mut App,
    measured: Option<&EdgeCrop>,
    why_not: Option<&'static str>,
) {
    let st = effective_strips(&app.config);
    let (l, r, t, b) = (st.left, st.right, st.top, st.bottom + st.band);
    let source = app
        .report
        .as_ref()
        .and_then(|rep| rep.info)
        .map(|i| (i.width, i.height));

    let mut open = app.is_open("crop-details");
    if theme::disclosure(ui, "Details", &mut open) {
        app.set_open("crop-details", open);
    }
    if !open {
        return;
    }
    ui.add_space(8.0);
    egui::Frame::NONE
        .fill(theme::bg())
        .corner_radius(theme::RADIUS_SMALL)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
        if let Some(m) = measured {
            ui.label(
                RichText::new(format!(
                    "Measured from 14 frames spread through the file: left {}, right {}, \
                     top {}, bottom {}{}.",
                    m.left,
                    m.right,
                    m.top,
                    m.bottom,
                    if m.noise_rows > 0 {
                        format!(" (of which {} rows are head-switching noise)", m.noise_rows)
                    } else {
                        String::new()
                    }
                ))
                .size(theme::FONT_SMALL)
                .color(theme::muted()),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "A line counts as border when it is both darker and flatter than the \
                     picture — darkness alone would mistake a night scene for a border. \
                     The soft ramp at the picture edge is followed only while it keeps \
                     getting brighter inward, which is what separates it from genuinely \
                     dark content. The head-switching band is found a different way: it \
                     carries far more horizontal detail than any picture does.",
                )
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
            );
        }
        if let Some(why) = why_not {
            ui.label(
                RichText::new(format!("The measurement returned: {why}."))
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
            );
        }
        ui.add_space(6.0);
        if let Some((w, h)) = source {
            let kept_w = w.saturating_sub(l + r);
            let kept_h = h.saturating_sub(t + b);
            ui.label(
                RichText::new(format!(
                    "Applying left {l}, right {r}, top {t}, bottom {b} to {w}×{h} leaves \
                     {kept_w}×{kept_h}."
                ))
                .size(theme::FONT_SMALL)
                .color(theme::muted()),
            );
            // The same arithmetic the script runs, so the number shown here is
            // the number used. Zero unless rows actually came off: nothing
            // else may touch the sides.
            let (target_w, target_h) = if app.config.format == VideoFormat::Pal {
                (768, 576)
            } else {
                (640, 480)
            };
            let trim = if t + b > 0 {
                Config::aspect_trim(kept_w, kept_h, target_h, app.config.reference_width())
            } else {
                0
            };
            if trim > 0 {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "{} lines came off the top or bottom. What is left is then \
                         resized onto a fixed {target_w}×{target_h}, which only tells \
                         the truth for one region shape — so {trim} columns come off \
                         the sides ({} each), leaving {}×{kept_h}. Without that the \
                         picture would be subtly the wrong shape.",
                        t + b,
                        trim / 2,
                        kept_w.saturating_sub(trim),
                    ))
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
                );
            }
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "The left and right cut is made before deinterlacing; the top and bottom \
                 cut waits until afterwards, because removing lines from an interlaced \
                 frame would discard half of each one.",
            )
            .size(theme::FONT_SMALL)
            .color(theme::faint()),
        );
        });
}

// ── ② ───────────────────────────────────────────────────────────────────────

fn picture_card(ui: &mut egui::Ui, app: &mut App) {
    let inner = COLUMN - 34.0;
    let strips = effective_strips(&app.config);
    let (sw, sh) = app.source_size().unwrap_or((0, 0));
    let aspect = aspect_extra(&app.config, sw, sh);

    theme::card().show(ui, |ui| {
        ui.set_width(inner);

        match &app.crop_frame {
            Some(texture) => {
                let size = texture.size_vec2();
                let scale = inner / size.x;
                let shown = size * scale;
                let (rect, _) = ui.allocate_exact_size(shown, egui::Sense::hover());
                if ui.is_rect_visible(rect) {
                    egui::Image::new(texture)
                        .corner_radius(theme::RADIUS_SMALL)
                        .paint_at(ui, rect);
                    overlay(ui, rect, scale, strips, aspect);
                }
                ui.add_space(8.0);
                let note = if aspect.0 + aspect.1 > 0 {
                    "The gray strips are the black border being trimmed; the amber strip \
                     along the bottom is the flickering band; the blue strips at the sides \
                     are picture given up to keep the shape right, because rows came off \
                     the top or bottom. The outline is what the finished video will \
                     contain. If any of your picture is under the shading, choose \"Set it \
                     myself\" below and reduce that side."
                } else {
                    "The gray strips are the black border being trimmed; the amber strip \
                     along the bottom is the flickering band. The outline is what the \
                     finished video will contain. If any of your picture is under the \
                     shading, choose \"Set it myself\" below and reduce that side."
                };
                theme::paragraph(ui, RichText::new(note)
                            .size(theme::FONT_SMALL)
                            .color(theme::faint()),);
            }
            None => {
                let height = (inner * 3.0 / 4.0).floor();
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(inner, height), egui::Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter()
                        .rect_filled(rect, theme::RADIUS_SMALL, theme::panel2());
                }
                ui.add_space(8.0);
                let waiting = app
                    .crop_frame_job
                    .as_ref()
                    .is_some_and(|j| !j.finished);
                let note = if waiting {
                    ui.ctx().request_repaint();
                    "Fetching a frame from your video…"
                } else {
                    "No frame could be read from this file, so the crop cannot be shown \
                     on the picture. The numbers below still apply."
                };
                ui.label(
                    RichText::new(note)
                        .size(theme::FONT_SMALL)
                        .color(theme::faint()),
                );
            }
        }
    });
}

/// Shade the strips that will be cropped, and rule the boundary.
///
/// `aspect` is the extra columns taken off each side to hold the shape. They
/// are part of what gets removed, so leaving them out of the outline drew a
/// keep-region wider than the one the render produces — the overlay promised
/// picture that the finished file does not contain.
fn overlay(ui: &egui::Ui, rect: egui::Rect, scale: f32, strips: Strips, aspect: (u32, u32)) {
    let painter = ui.painter_at(rect);
    // From the palette, because these are drawn over the user's own picture
    // and a dark scrim that reads as "removed" on a dark page disappears into
    // a light one.
    let shade = theme::crop_shade();
    // The band gets its own color, because it is its own choice: the user has
    // to be able to see which strip the toggle above controls.
    let band_shade = theme::crop_band();
    // And the shape trim a third, because it is not a border being removed —
    // it is picture, given up to keep the geometry right. Someone looking at
    // their own faces near the edge needs to see which of the two it is.
    let aspect_shade = theme::crop_aspect();

    // Scaled strip widths, with a floor of one pixel so an 8-column crop is
    // still visible once the frame is scaled down to the column width.
    let px = |v: u32| -> f32 {
        if v == 0 {
            0.0
        } else {
            (v as f32 * scale).max(1.0)
        }
    };
    let lw = px(strips.left);
    let rw = px(strips.right);
    let th = px(strips.top);
    let bb = px(strips.bottom);
    let band = px(strips.band);
    // The shape trim comes off what is left after the border, so it is drawn
    // inboard of it.
    let al = px(aspect.0);
    let ar = px(aspect.1);

    let fill = |min: egui::Pos2, max: egui::Pos2, color: egui::Color32| {
        painter.rect_filled(egui::Rect::from_min_max(min, max), 0.0, color);
    };

    if lw > 0.0 {
        fill(rect.left_top(), egui::pos2(rect.left() + lw, rect.bottom()), shade);
    }
    if rw > 0.0 {
        fill(egui::pos2(rect.right() - rw, rect.top()), rect.right_bottom(), shade);
    }
    if th > 0.0 {
        fill(rect.left_top(), egui::pos2(rect.right(), rect.top() + th), shade);
    }
    // The band sits at the very bottom of the frame, with any bottom blanking
    // above it — that is the order they occur in on the tape.
    if band > 0.0 {
        fill(
            egui::pos2(rect.left(), rect.bottom() - band),
            rect.right_bottom(),
            band_shade,
        );
    }
    if bb > 0.0 {
        fill(
            egui::pos2(rect.left(), rect.bottom() - band - bb),
            egui::pos2(rect.right(), rect.bottom() - band),
            shade,
        );
    }

    // The shape trim, inboard of the border on each side.
    if al > 0.0 {
        fill(
            egui::pos2(rect.left() + lw, rect.top() + th),
            egui::pos2(rect.left() + lw + al, rect.bottom() - bb - band),
            aspect_shade,
        );
    }
    if ar > 0.0 {
        fill(
            egui::pos2(rect.right() - rw - ar, rect.top() + th),
            egui::pos2(rect.right() - rw, rect.bottom() - bb - band),
            aspect_shade,
        );
    }

    // The kept area, outlined so the boundary is unambiguous even where the
    // picture behind it is already dark.
    let keep = egui::Rect::from_min_max(
        egui::pos2(rect.left() + lw + al, rect.top() + th),
        egui::pos2(rect.right() - rw - ar, rect.bottom() - bb - band),
    );
    if keep.width() > 2.0 && keep.height() > 2.0 && strips.any() {
        painter.rect_stroke(
            keep,
            0.0,
            egui::Stroke::new(1.0, theme::accent()),
            egui::StrokeKind::Inside,
        );
    }
}

// ── ③ ───────────────────────────────────────────────────────────────────────

fn choice_card(ui: &mut egui::Ui, app: &mut App, measured: Option<EdgeCrop>) {
    let inner = COLUMN - 34.0;

    // When the measurement found nothing, "crop what we found" and "crop
    // nothing" are the same instruction written twice, and on a DV file —
    // which has no blanking to find — that is the normal case. Reported from
    // testing as two options that read differently and do the same thing.
    //
    // The measured row is the one that goes: it is the one whose label has
    // to be written around the absence of a number, and the plain "do not
    // crop anything" is what the user is actually choosing. Its
    // recommendation badge moves across with it.
    let nothing_to_crop = measured.is_some_and(|m| m.with_margin(SAFETY_MARGIN).is_nothing());
    let current = match current_choice(&app.config, measured) {
        // A zero manual crop and an explicit no-crop are indistinguishable
        // on screen, so they must not light up different rows.
        Choice::Measured if nothing_to_crop => Choice::Nothing,
        other => other,
    };

    theme::card().show(ui, |ui| {
        ui.set_width(inner);

        if let Some(m) = measured
            && !nothing_to_crop
        {
            // The band is set by its own toggle above, so it is not written
            // into the crop values here — but it *is* named in the label.
            // Listing "0 bottom" next to a section that plainly removes ten
            // lines from the bottom reads as a contradiction, and nobody
            // should have to know that one is blanking and the other is not.
            let r = m.with_margin(SAFETY_MARGIN);
            let label = measured_label(&r);
            if radio_row(ui, inner, current == Choice::Measured, &label) {
                app.config.crop_preset = Some(CropPreset::Manual);
                app.config.crop_left = r.left;
                app.config.crop_right = r.right;
                app.config.crop_top = r.top;
                app.config.crop_bottom = r.bottom;
                app.config.head_switching_rows = r.noise_rows;
            }
        }

        // The fixed 8-a-side preset is offered only when there is nothing
        // measured to offer instead. Alongside a measurement it is simply the
        // worse answer, and calling it "standard" makes it sound like the
        // recommended one. It stays reachable when a stale config has it
        // selected, so the card can never show nothing selected.
        if (measured.is_none() || current == Choice::Standard)
            && radio_row(
                ui,
                inner,
                current == Choice::Standard,
                "Trim the usual amount for a tape capture: 8 pixels each side",
            )
        {
            app.config.crop_preset = Some(CropPreset::Bt601);
        }

        if radio_row(
            ui,
            inner,
            current == Choice::Nothing,
            if nothing_to_crop {
                "Do not crop anything — the picture already reaches every edge (recommended)"
            } else {
                "Do not crop anything — keep the frame exactly as captured"
            },
        ) {
            app.config.crop_preset = Some(CropPreset::None);
        }

        if radio_row(ui, inner, current == Choice::Manual, "Set it myself") {
            // Start from whatever is on screen now, so the sliders do not jump.
            let st = effective_strips(&app.config);
            // The band is part of the bottom crop, so it comes along.
            let (l, r, t, b) = (st.left, st.right, st.top, st.bottom + st.band);
            app.config.crop_preset = Some(CropPreset::Manual);
            app.config.crop_left = l;
            app.config.crop_right = r;
            app.config.crop_top = t;
            app.config.crop_bottom = b;
            // Nudge one value so the choice reads back as Manual rather than
            // snapping to Measured when the numbers happen to match.
            if measured.is_some_and(|m| {
                let k = m.with_margin(SAFETY_MARGIN);
                (k.left, k.right, k.top, k.bottom) == (l, r, t, b)
            }) {
                app.config.crop_left = l.saturating_add(2);
            }
        }

        let (w, h) = app
            .report
            .as_ref()
            .and_then(|rep| rep.info)
            .map(|i| (i.width, i.height))
            .unwrap_or((720, 480));

        if current == Choice::Manual {
            ui.add_space(12.0);
            manual_values(ui, app, w, h);
        }

        ui.add_space(12.0);
        ui.label(
            RichText::new(outcome_line(&app.config, w, h))
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
        );
    });
}

/// The label for the measured option.
///
/// Zero sides are left out — "0 top, 0 bottom" is noise — and where the bottom
/// crop is mostly the flickering band, it says so, because a bare number there
/// looks like an awful lot of picture to be throwing away.
fn measured_label(r: &EdgeCrop) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (amount, side) in [
        (r.left, "left"),
        (r.right, "right"),
        (r.top, "top"),
        (r.bottom, "bottom"),
    ] {
        if amount > 0 {
            parts.push(format!("{amount} {side}"));
        }
    }
    if parts.is_empty() {
        return "Crop nothing — the picture already reaches every edge (recommended)".to_string();
    }
    let list = if parts.len() == 1 {
        parts[0].clone()
    } else {
        let last = parts.pop().unwrap();
        format!("{}, {last}", parts.join(", "))
    };
    let band = if r.noise_rows > 0 {
        format!(", with {} of the bottom being the switching noise", r.noise_rows)
    } else {
        String::new()
    };
    format!("Crop what we found: {list}{band} (recommended)")
}

/// One line saying what actually comes out, which is the question all of the
/// above is really answering.
fn outcome_line(config: &Config, width: u32, height: u32) -> String {
    let st = effective_strips(config);
    let kept_w = width.saturating_sub(st.left + st.right);
    let kept_h = height.saturating_sub(st.top + st.bottom + st.band);
    if kept_w == 0 || kept_h == 0 {
        return "Nothing would be left of the frame.".to_string();
    }

    let (target_w, target_h) = if config.format == VideoFormat::Pal {
        (768, 576)
    } else {
        (640, 480)
    };
    let trim = if st.top + st.bottom + st.band > 0 {
        Config::aspect_trim(kept_w, kept_h, target_h, config.reference_width())
    } else {
        0
    };
    let final_w = kept_w.saturating_sub(trim);

    let shape = if trim > 0 {
        format!(
            " ({trim} of those columns keep the shape right, since lines came off the bottom)"
        )
    } else {
        String::new()
    };

    if config.par_correction && !config.is_hd_source() {
        format!(
            "Result: {width}×{height} becomes {final_w}×{kept_h}{shape}, exported as {target_w}×{target_h} with square pixels."
        )
    } else {
        format!("Result: {width}×{height} becomes {final_w}×{kept_h}{shape}.")
    }
}

/// A radio row with the hit area spanning the card, so the label is clickable.
fn radio_row(ui: &mut egui::Ui, width: f32, selected: bool, label: &str) -> bool {
    let height = 34.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let hovered = response.hovered();

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if hovered || selected {
            painter.rect_filled(
                rect,
                theme::RADIUS_SMALL,
                if selected {
                    theme::accent_soft()
                } else {
                    theme::panel2()
                },
            );
        }
        let center = egui::pos2(rect.left() + 17.0, rect.center().y);
        painter.circle_stroke(
            center,
            7.0,
            egui::Stroke::new(
                1.5,
                if selected {
                    theme::accent()
                } else if hovered {
                    theme::line_hover()
                } else {
                    theme::line()
                },
            ),
        );
        if selected {
            painter.circle_filled(center, 3.5, theme::accent());
        }
        painter.text(
            egui::pos2(rect.left() + 34.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(theme::FONT_BODY),
            if selected { theme::text() } else { theme::muted() },
        );
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    theme::focus_ring(ui, &response, theme::RADIUS_SMALL);
    response.clicked() && !selected
}

fn manual_values(ui: &mut egui::Ui, app: &mut App, width: u32, height: u32) {
    // A quarter of the dimension is far more than any real border and keeps a
    // dragged value from collapsing the frame.
    let max_x = width / 4;
    let max_y = height / 4;

    // The four boxes hold the real totals, including the columns taken off
    // the sides to hold the aspect ratio.
    //
    // They used to hold only what the user typed, with the compensation
    // shown underneath, and that was reported twice as confusing: change the
    // bottom and the side boxes sit still while the picture and the result
    // line both move. The objection to folding it in was a feedback loop —
    // the compensation is computed *from* the side crop, so writing it back
    // changes its own input.
    //
    // It converges in one step, and that is not luck. The trim exists to
    // make the kept width equal `reference x kept_height / target_height`,
    // so once the sides already add up to that, the generator's `excess` is
    // zero and it adds nothing further. Folding the compensation in reaches
    // that fixed point immediately, and the next recomputation agrees.
    //
    // Only the rows rebalance the sides. Editing Left or Right directly is
    // the user overriding the shape on purpose, and a box that fought back
    // would be worse than the problem being fixed.
    // The user's own side crop, remembered apart from what the boxes show.
    // Seeded from the config the first time, which is the measured
    // recommendation before any compensation has been folded in.
    let base = app
        .crop_sides_base
        .unwrap_or((app.config.crop_left, app.config.crop_right));

    let mut rows_changed = false;
    let mut sides_changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        sides_changed |= value_box(ui, "Left", &mut app.config.crop_left, max_x);
        sides_changed |= value_box(ui, "Right", &mut app.config.crop_right, max_x);
        rows_changed |= value_box(ui, "Top", &mut app.config.crop_top, max_y);
        rows_changed |= value_box(ui, "Bottom", &mut app.config.crop_bottom, max_y);
    });

    if sides_changed {
        // A side typed directly is a total, so the base is that total less
        // whatever compensation is currently in it. The user is overriding
        // the shape on purpose and the number stays where they put it.
        let (el, er) = compensation_for(&app.config, base, width, height);
        app.crop_sides_base = Some((
            app.config.crop_left.saturating_sub(el),
            app.config.crop_right.saturating_sub(er),
        ));
    } else if rows_changed {
        // Rows changed, so the shape did: recompute the sides from the base.
        app.crop_sides_base = Some(base);
        apply_compensation(&mut app.config, base, width, height);
    }
    ui.add_space(8.0);
    let kept_w = width.saturating_sub(app.config.crop_left + app.config.crop_right);
    let kept_h = height.saturating_sub(app.config.crop_top + app.config.crop_bottom);
    // No separate trim to subtract any more: the side boxes already hold it,
    // so what they leave is what comes out.
    ui.label(
        RichText::new(format!("Leaves {kept_w}×{kept_h} pixels."))
            .size(theme::FONT_SMALL)
            .color(theme::faint()),
    );

    // Why the side numbers moved when only the bottom was touched.
    if app.config.crop_top + app.config.crop_bottom > 0 {
        let removed = app.config.crop_top + app.config.crop_bottom;
        ui.add_space(4.0);
        theme::paragraph(ui, RichText::new(format!(
                    "Left and Right include the columns needed to keep the shape right. \
                     Taking {removed} rows off the top or bottom makes the frame squatter, \
                     so the sides come in to match — without that, everything in the \
                     finished video would look stretched. Change Left or Right yourself \
                     and they stay where you put them."
                ))
                .size(theme::FONT_SMALL)
                .color(theme::muted()),);
    }
}

/// One crop value. Returns true when it changed this frame.
///
/// The value is the real total for that edge — the blanking the user asked
/// for plus, on the sides, whatever was folded in to hold the shape. See
/// `manual_values` for why those are one number rather than two.
fn value_box(ui: &mut egui::Ui, label: &str, value: &mut u32, max: u32) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        ui.label(
            RichText::new(label)
                .size(theme::FONT_SMALL)
                .color(theme::muted()),
        );
        ui.add_space(2.0);
        // Stepped by two: an odd crop is illegal on subsampled chroma, and the
        // generator would silently round it anyway.
        changed = ui
            .add(
                egui::DragValue::new(value)
                    .speed(0.25)
                    .range(0..=max)
                    .update_while_editing(false)
                    .custom_formatter(|n, _| format!("{}", (n as u32 / 2) * 2))
                    .custom_parser(|s| {
                        s.trim().parse::<f64>().ok().map(|v| (v / 2.0).round() * 2.0)
                    }),
            )
            .changed();
    });
    changed
}

/// The compensation owed, computed from the user's own crop.
///
/// Deliberately measured against `base`, the blanking the user asked for,
/// rather than against whatever is currently in the config. That is what
/// breaks the circularity: the trim exists to correct a width, so computing
/// it from a width that already includes it is the loop that made this
/// feature look impossible. Against the base it is a plain function of the
/// rows, and adding it is a one-way step.
pub fn compensation_for(
    config: &Config,
    base: (u32, u32),
    width: u32,
    height: u32,
) -> (u32, u32) {
    if config.effective_crop_preset() != CropPreset::Manual
        || !config.keep_aspect
        || !config.par_correction
        || config.is_hd_source()
        || config.crop_top + config.crop_bottom == 0
    {
        return (0, 0);
    }
    let kept_w = width.saturating_sub(base.0 + base.1);
    let kept_h = height.saturating_sub(config.crop_top + config.crop_bottom);
    let target_h = if config.format == VideoFormat::Pal { 576 } else { 480 };
    let trim = Config::aspect_trim(kept_w, kept_h, target_h, config.reference_width());
    (trim / 2, trim - trim / 2)
}

/// Write the totals — the user's crop plus the compensation — into the config.
///
/// After this the generator finds nothing left to trim, because the kept
/// width is already the width it was going to trim to. So the numbers in the
/// boxes, the strips on the picture and the finished file all agree, and the
/// recomputation on the next frame changes nothing.
pub fn apply_compensation(config: &mut Config, base: (u32, u32), width: u32, height: u32) {
    let (el, er) = compensation_for(config, base, width, height);
    let limit = width / 4;
    config.crop_left = (base.0 + el).min(limit);
    config.crop_right = (base.1 + er).min(limit);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As measured on the user's capture, before the safety margin.
    fn measured() -> EdgeCrop {
        EdgeCrop {
            left: 14,
            right: 14,
            top: 0,
            bottom: 10,
            noise_rows: 10,
        }
    }

    /// What picking the recommendation writes into the config.
    fn recommended() -> Config {
        let r = measured().with_margin(SAFETY_MARGIN);
        Config {
            crop_preset: Some(CropPreset::Manual),
            crop_left: r.left,
            crop_right: r.right,
            crop_top: r.top,
            crop_bottom: r.bottom,
            head_switching_rows: r.noise_rows,
            ..Config::default()
        }
    }

    /// The overlay draws this on the user's own picture, so it has to be the
    /// amount the render actually removes. The case is the one reported from
    /// testing: 720×480 cropped 18/16/12/40 leaves 686×428, and holding 4:3
    /// costs 60 more columns, 30 a side.
    #[test]
    fn the_shape_trim_matches_what_the_render_takes() {
        let c = Config {
            crop_preset: Some(CropPreset::Manual),
            crop_left: 18,
            crop_right: 16,
            crop_top: 12,
            crop_bottom: 40,
            ..Config::default()
        };
        assert_eq!(aspect_extra(&c, 720, 480), (30, 30));
        // And it agrees with the arithmetic the generated script evaluates.
        let kept_w = 720 - 18 - 16;
        let kept_h = 480 - 12 - 40;
        assert_eq!(
            Config::aspect_trim(kept_w, kept_h, 480, c.reference_width()),
            60
        );
    }

    /// Nothing off the top or bottom means nothing off the sides. "Do not
    /// crop anything" has to mean it, and a user who set only a left and
    /// right crop asked for exactly that.
    #[test]
    fn no_rows_removed_means_no_shape_trim() {
        let c = Config {
            crop_preset: Some(CropPreset::Manual),
            crop_left: 18,
            crop_right: 16,
            crop_top: 0,
            crop_bottom: 0,
            ..Config::default()
        };
        assert_eq!(aspect_extra(&c, 720, 480), (0, 0));

        let none = Config {
            crop_preset: Some(CropPreset::None),
            ..Config::default()
        };
        assert_eq!(aspect_extra(&none, 720, 480), (0, 0));
    }

    /// The generator skips the trim when it is not resizing onto a fixed
    /// target. Drawing it anyway would shade picture that is kept.
    #[test]
    fn the_shape_trim_follows_the_generators_own_gates() {
        let base = Config {
            crop_preset: Some(CropPreset::Manual),
            crop_left: 18,
            crop_right: 16,
            crop_top: 12,
            crop_bottom: 40,
            ..Config::default()
        };
        assert_eq!(
            aspect_extra(
                &Config {
                    par_correction: false,
                    ..base.clone()
                },
                720,
                480
            ),
            (0, 0),
            "no resize, no trim"
        );
        assert_eq!(
            aspect_extra(
                &Config {
                    keep_aspect: false,
                    ..base.clone()
                },
                720,
                480
            ),
            (0, 0),
            "switched off explicitly"
        );
    }

    /// The bug this merge fixes: the band had its own toggle above the crop
    /// options, so the bottom could be cropped by the band *and* again by the
    /// manual value, with neither control showing the total. One number now.
    #[test]
    fn the_bottom_is_never_counted_twice() {
        let c = recommended();
        assert_eq!(c.crop_bottom, 12, "10 measured plus the margin");
        assert_eq!(c.head_switching_rows, 12, "all of it is the band");

        let st = effective_strips(&c);
        assert_eq!(
            st.bottom + st.band,
            c.crop_bottom,
            "the overlay must split the bottom crop, never add to it"
        );
        assert_eq!(st.band, 12, "and all of it shades as band");
        assert_eq!(st.bottom, 0);
    }

    #[test]
    fn the_margin_reaches_the_config() {
        let c = recommended();
        // 14 measured + 2 margin a side, 10 + 2 at the bottom.
        assert_eq!((c.crop_left, c.crop_right), (16, 16));
        assert_eq!(c.crop_top, 0, "nothing was found at the top");
        assert_eq!(c.crop_bottom, 12);
    }

    #[test]
    fn the_recommendation_reads_back_as_recommended() {
        assert_eq!(
            current_choice(&recommended(), Some(measured())),
            Choice::Measured
        );
        let edited = Config {
            crop_left: 20,
            ..recommended()
        };
        assert_eq!(current_choice(&edited, Some(measured())), Choice::Manual);
    }

    #[test]
    fn a_manual_crop_without_a_measurement_reads_as_manual() {
        let c = Config {
            crop_preset: Some(CropPreset::Manual),
            crop_left: 14,
            ..Config::default()
        };
        assert_eq!(current_choice(&c, None), Choice::Manual);
    }

    #[test]
    fn the_presets_read_back_too() {
        let preset = Config {
            crop_preset: Some(CropPreset::Bt601),
            ..Config::default()
        };
        assert_eq!(current_choice(&preset, Some(measured())), Choice::Standard);
        let none = Config {
            crop_preset: Some(CropPreset::None),
            ..Config::default()
        };
        assert_eq!(current_choice(&none, Some(measured())), Choice::Nothing);
    }

    /// The overlay must show what the pipeline will do, including the HD
    /// override that forces no crop whatever the user picked.
    #[test]
    fn the_overlay_follows_the_effective_preset() {
        let st = effective_strips(&recommended());
        assert_eq!((st.left, st.right, st.top), (16, 16, 0));

        let standard = Config {
            crop_preset: Some(CropPreset::Bt601),
            ..Config::default()
        };
        let st = effective_strips(&standard);
        assert_eq!((st.left, st.right, st.bottom, st.band), (8, 8, 0, 0));

        let nothing = Config {
            crop_preset: Some(CropPreset::None),
            ..Config::default()
        };
        assert!(!effective_strips(&nothing).any());
    }

    /// A preset has no idea where the band is, so it must not shade one.
    #[test]
    fn only_a_manual_crop_shades_a_band() {
        let c = Config {
            crop_preset: Some(CropPreset::Bt601),
            head_switching_rows: 12,
            ..Config::default()
        };
        assert_eq!(effective_strips(&c).band, 0);
    }

    /// A stale band larger than the crop it supposedly lives in must not
    /// produce a negative blanking strip.
    #[test]
    fn a_band_bigger_than_its_crop_cannot_underflow() {
        let c = Config {
            crop_preset: Some(CropPreset::Manual),
            crop_bottom: 4,
            head_switching_rows: 99,
            ..Config::default()
        };
        let st = effective_strips(&c);
        assert_eq!(st.band, 4);
        assert_eq!(st.bottom, 0);
    }

    #[test]
    fn an_hd_source_shows_no_crop_whatever_was_chosen() {
        use vcg_core::config::{SourceClass, SourceClassification};
        let hd = Config {
            source: SourceClassification {
                source_class: SourceClass::Avchd,
                ..Default::default()
            },
            ..recommended()
        };
        assert!(hd.is_hd_source(), "the premise of this test");
        assert!(
            !effective_strips(&hd).any(),
            "the overlay promised a crop the pipeline will not make"
        );
    }

    // ── The option labels ───────────────────────────────────────────────

    #[test]
    fn the_label_lists_one_crop_and_names_the_band_inside_it() {
        let label = measured_label(&measured().with_margin(SAFETY_MARGIN));
        assert!(label.contains("16 left"), "{label}");
        assert!(label.contains("16 right"), "{label}");
        assert!(label.contains("12 bottom"), "{label}");
        assert!(label.contains("switching noise"), "{label}");
        assert!(
            !label.contains("0 top"),
            "zero sides must be left out: {label}"
        );
    }

    #[test]
    fn the_label_drops_the_band_note_when_there_is_none() {
        let plain = EdgeCrop {
            left: 14,
            right: 12,
            top: 0,
            bottom: 0,
            noise_rows: 0,
        };
        let label = measured_label(&plain);
        assert!(label.contains("14 left"), "{label}");
        assert!(!label.contains("switching noise"), "{label}");
    }

    #[test]
    fn the_label_handles_having_nothing_to_say() {
        assert!(measured_label(&EdgeCrop::default()).contains("Crop nothing"));
    }

    /// The outcome line is the only place that states what actually comes out,
    /// so it has to agree with the pipeline including the aspect trim.
    #[test]
    fn the_outcome_line_reports_the_real_numbers() {
        let line = outcome_line(&recommended(), 720, 480);
        // 720-32 = 688 wide, 480-12 = 468 tall.
        assert!(line.contains("688×468"), "{line}");
        assert!(line.contains("640×480"), "{line}");
        assert!(line.contains("square pixels"), "{line}");
    }

    #[test]
    fn the_outcome_line_mentions_no_shape_trim_when_there_is_none() {
        let config = Config {
            crop_preset: Some(CropPreset::Manual),
            crop_left: 14,
            crop_right: 14,
            ..Config::default()
        };
        let line = outcome_line(&config, 720, 480);
        assert!(line.contains("692×480"), "{line}");
        assert!(!line.contains("keep the shape"), "{line}");
    }

    #[test]
    fn the_description_names_every_edge_that_has_something_on_it() {
        let text = plain_description(&measured());
        assert!(text.contains("14 pixels on the left"), "{text}");
        assert!(text.contains("14 on the right"), "{text}");
        assert!(text.contains("10 of noise at the very bottom"), "{text}");
        assert!(!text.contains("top"), "{text}");
    }

    #[test]
    fn bottom_blanking_and_noise_are_described_apart() {
        let c = EdgeCrop {
            left: 0,
            right: 0,
            top: 0,
            bottom: 14,
            noise_rows: 6,
        };
        let text = plain_description(&c);
        assert!(text.contains("8 along the bottom"), "{text}");
        assert!(text.contains("6 of noise"), "{text}");
    }

    #[test]
    fn nothing_found_gets_its_own_sentence() {
        let text = plain_description(&EdgeCrop::default());
        assert!(text.contains("nothing at the edges"), "{text}");
    }
}

#[cfg(test)]
mod compensation_tests {
    use super::*;

    fn manual(l: u32, r: u32, t: u32, b: u32) -> Config {
        Config {
            crop_preset: Some(CropPreset::Manual),
            crop_left: l,
            crop_right: r,
            crop_top: t,
            crop_bottom: b,
            ..Config::default()
        }
    }

    /// The case reported from testing: 720x480 cropped 18/16/12/40. The side
    /// boxes must end up holding the real totals.
    #[test]
    fn the_sides_hold_the_total_once_rows_come_off() {
        let base = (18, 16);
        let mut c = manual(base.0, base.1, 12, 40);
        apply_compensation(&mut c, base, 720, 480);
        assert_eq!((c.crop_left, c.crop_right), (48, 46));
    }

    /// Why this looked impossible: the trim is computed from the width it is
    /// correcting, so writing it back changes its own input. Measuring it
    /// against the user's base instead makes it a plain function of the rows,
    /// and applying it twice has to be the same as applying it once.
    #[test]
    fn applying_it_twice_changes_nothing() {
        let base = (18, 16);
        let mut c = manual(base.0, base.1, 12, 40);
        apply_compensation(&mut c, base, 720, 480);
        let once = (c.crop_left, c.crop_right);
        apply_compensation(&mut c, base, 720, 480);
        assert_eq!((c.crop_left, c.crop_right), once);
    }

    /// And the generator must then find nothing left to do, or the render
    /// would take the columns off a second time.
    #[test]
    fn the_generator_adds_nothing_afterwards() {
        let base = (18, 16);
        let mut c = manual(base.0, base.1, 12, 40);
        apply_compensation(&mut c, base, 720, 480);
        assert_eq!(aspect_extra(&c, 720, 480), (0, 0));
    }

    /// Taking the rows back off returns the sides exactly, which is only
    /// possible because the user's own numbers were kept.
    #[test]
    fn removing_the_rows_restores_the_users_own_crop() {
        let base = (18, 16);
        let mut c = manual(base.0, base.1, 0, 40);
        apply_compensation(&mut c, base, 720, 480);
        assert!(c.crop_left > base.0, "compensation went in");

        c.crop_bottom = 0;
        apply_compensation(&mut c, base, 720, 480);
        assert_eq!((c.crop_left, c.crop_right), base, "and came back out");
    }

    /// Both sides move by the same amount, so lopsided blanking stays
    /// lopsided and the picture's centre does not drift.
    #[test]
    fn the_centre_does_not_drift() {
        let base = (30, 4);
        let mut c = manual(base.0, base.1, 0, 40);
        apply_compensation(&mut c, base, 720, 480);
        assert_eq!(
            c.crop_left as i64 - c.crop_right as i64,
            base.0 as i64 - base.1 as i64
        );
    }

    /// Nothing is folded in where the generator would not have trimmed.
    #[test]
    fn it_respects_the_generators_gates() {
        let base = (18, 16);
        for tweak in [
            |c: &mut Config| c.par_correction = false,
            |c: &mut Config| c.keep_aspect = false,
        ] {
            let mut c = manual(base.0, base.1, 12, 40);
            tweak(&mut c);
            apply_compensation(&mut c, base, 720, 480);
            assert_eq!((c.crop_left, c.crop_right), base);
        }
    }

    /// What the boxes show has to be what the render produces.
    #[test]
    fn the_boxes_agree_with_the_rendered_width() {
        let base = (18, 16);
        let mut c = manual(base.0, base.1, 12, 40);
        apply_compensation(&mut c, base, 720, 480);
        let kept_w = 720 - c.crop_left - c.crop_right;
        let kept_h = 480 - c.crop_top - c.crop_bottom;
        let needed = 704.0 * kept_h as f64 / 480.0;
        assert!(
            (kept_w as f64 - needed).abs() <= 4.0,
            "kept {kept_w} against {needed:.1}"
        );
    }
}
