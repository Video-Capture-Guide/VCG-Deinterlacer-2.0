//! The Settings pages, for Full control mode.
//!
//! Eleven screens that each ask one question. Written as data rather than as
//! eleven hand-made pages: every one of them is a title, a sentence saying
//! what the setting is for, a short list of choices, and a Details panel for
//! someone who wants the actual filter. Giving each its own file would mean
//! eleven near-identical layouts drifting apart, and the shape of the screen
//! would depend on which page you were on.
//!
//! Three things make these different from a plain options list:
//!
//! * **What the analysis found is shown next to the choice.** In this mode the
//!   measurement is not applied for you, so the page has to say what it was —
//!   otherwise Full control means guessing.
//! * **The recommended option is marked.** Knowing every option is not the
//!   same as knowing which one to pick.
//! * **Nothing is hidden behind jargon on the face of the page.** The filter
//!   names, strengths and arguments live in Details, exactly as on Export.

use eframe::egui::{self, RichText};

use vcg_core::analysis::audio::AudioVerdict;
use vcg_core::analysis::scopes;
use vcg_core::analysis::histogram::{LumaHistogram, LEGAL_MAX, LEGAL_MIN};
use vcg_core::config::{
    ColorCorrection, Config, DehaloMode, DitherMethod, LevelsAdjustment, NoiseLevel,
};

use crate::check::COLUMN;
use crate::nav::Step;
use crate::pages::PageAction;
use crate::theme::{self, Tag};
use crate::worker::PreviewKind;
use crate::App;

/// One selectable answer.
struct Opt {
    label: &'static str,
    note: &'static str,
}

/// A page that picks one of a short list.
struct ChoicePage {
    title: &'static str,
    /// What this setting is for, in one or two sentences.
    lead: &'static str,
    options: &'static [Opt],
    /// Which option the config currently holds.
    get: fn(&Config) -> usize,
    /// Write the chosen option back.
    set: fn(&mut Config, usize),
    /// The technical account, for the Details disclosure.
    details: &'static [&'static str],
}

/// Render the Settings page for `step`, or `None` if it is not one of these.
pub fn settings_page(ui: &mut egui::Ui, app: &mut App, step: Step) -> Option<PageAction> {
    let action = PageAction::default();
    let page = page_for(step)?;
    ui.set_max_width(COLUMN);

    theme::page_header(ui, page.title, page.lead);
    ui.add_space(20.0);

    let inner_w = COLUMN - 34.0;

    let inner = COLUMN - 34.0;
    let current = (page.get)(&app.config);

    // The order on every one of these pages, graphs or not:
    //
    //   the analysis, the choice, the picture, then the scopes.
    //
    // The analysis comes first because it is the case for the choice: what
    // was measured on this tape, then what we think should be done about it,
    // and only then the control that does it. The before/after answers "what
    // does that do to my video", and the scopes are for someone who wants to
    // verify it — the last question, not the first. They used to open the
    // page, which put a density plot in front of a reader who had not yet
    // been told what was being decided.
    if let Some(found) = analysis_for(app, step) {
        analysis_card(ui, &found);
        ui.add_space(20.0);
    }

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        for (i, opt) in page.options.iter().enumerate() {
            if option_row(ui, inner, i == current, opt, i == recommended_index(app, step)) {
                (page.set)(&mut app.config, i);
            }
        }
    });
    ui.add_space(20.0);

    stage_pair(ui, app, step, current);

    graph_section(ui, app, step, page, inner_w);

    ui.add_space(16.0);
    let key = format!("set-{}", page.title);
    let mut open = app.is_open(&key);
    if theme::disclosure(ui, "Details", &mut open) {
        app.queue_open(&key, open);
    }
    if open {
        ui.add_space(8.0);
        egui::Frame::NONE
            .fill(theme::bg())
            .corner_radius(theme::RADIUS_SMALL)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(inner - 24.0);
                for (i, para) in page.details.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(8.0);
                    }
                    // Wrapping stated, not inherited: a label that inherits it
                    // from the layout runs off the right of the window the
                    // moment the layout changes.
                    theme::paragraph(ui, RichText::new(*para)
                                .size(theme::FONT_SMALL)
                                .color(theme::muted()),);
                }
                filter_snippet(ui, app, step);
            });
    }

    Some(action)
}

/// The scopes and graphs, below the picture.
///
/// Only two pages have any: Levels gets the brightness graphs and the RGB
/// parade, Color tint the vectorscope or RGB histogram. Everything else
/// draws nothing here.
fn graph_section(
    ui: &mut egui::Ui,
    app: &mut App,
    step: Step,
    page: &ChoicePage,
    inner_w: f32,
) {
    // The graph goes first on Levels: the shape is the finding, and the
    // sentence underneath is a caption for it.
    if step == Step::Levels {
        // One graph, of the frame on screen.
        //
        // It used to draw the whole-video average as bars with this moment
        // over it as a line, and the two never agreed, because they were not
        // in the same units — see `LumaHistogram::from_rgb`. With that fixed
        // they agree, and the aggregate earns no space: the recommendation
        // rests on it and says so in words below, while the thing a reader
        // can act on is the frame they are looking at.
        let moment = app
            .preview_rgb_for(&format!("stage-base{}", app.moment_key()))
            .map(|f| LumaHistogram::from_rgb(&f.rgb, f.width, f.height));
        if let Some(m) = &moment
            && !m.is_empty()
        {
            levels_graph(ui, m, "This moment");
            ui.add_space(14.0);

            // And the same frame with the correction applied, when one is
            // chosen — the question the page asks is what the correction
            // does, and a before with no after cannot answer it.
            let current = (page.get)(&app.config);
            if !is_no_op(Step::Levels, current) {
                let after_tag =
                    format!("stage-levels-on-{current}{}", app.moment_key());
                if let Some(after) = app
                    .preview_rgb_for(&after_tag)
                    .map(|f| LumaHistogram::from_rgb(&f.rgb, f.width, f.height))
                    .filter(|h| !h.is_empty())
                {
                    levels_graph(
                        ui,
                        &after,
                        &format!("This moment with {} applied", levels_label(current)),
                    );
                } else {
                    levels_placeholder(ui, inner_w);
                }
                ui.add_space(14.0);
            }
        }
        // And the parade, which says *where* across the frame a channel
        // runs out of room — a blown window on one side of the picture
        // wants a different answer from an overexposed whole frame.
        let tag = format!("stage-base{}", app.moment_key());
        let current = (page.get)(&app.config);
        theme::card().show(ui, |ui| {
            ui.set_width(inner_w);
            ui.label(
                RichText::new("RGB parade — red, green and blue across the frame")
                    .size(theme::FONT_BODY)
                    .color(theme::text()),
            );
            ui.add_space(8.0);
            // Before and after, once a correction is chosen. The parade is
            // the scope that says *where across the frame* a channel runs
            // out of room, so it is also the one that shows whether a
            // correction fixed that or merely moved it — which a single
            // parade of the source cannot answer.
            if is_no_op(Step::Levels, current) {
                parade_panel(ui, app, &tag, inner_w);
            } else {
                let after_tag = format!("stage-levels-on-{current}{}", app.moment_key());
                for (t, caption) in [
                    (&tag, "Before".to_string()),
                    (&after_tag, format!("After — {}", levels_label(current))),
                ] {
                    parade_panel(ui, app, t, inner_w);
                    ui.add_space(3.0);
                    ui.label(
                        RichText::new(caption)
                            .size(theme::FONT_SMALL)
                            .color(theme::faint()),
                    );
                    ui.add_space(10.0);
                }
            }
            ui.add_space(8.0);
            sampling_note(ui, "Brightness");
        });
        ui.add_space(14.0);
    }

    if step == Step::ColorTint {
        let tag = format!("stage-base{}", app.moment_key());
        theme::card().show(ui, |ui| {
            ui.set_width(inner_w);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Color scopes")
                        .size(theme::FONT_BODY)
                        .color(theme::text()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (kind, label) in [
                        (Scope::RgbHistogram, "RGB histogram"),
                        (Scope::Vectorscope, "Vectorscope"),
                    ] {
                        let on = app.scope_choice == kind;
                        if theme::chip_button(ui, label, on) {
                            app.scope_choice = kind;
                        }
                    }
                });
            });
            // Magnification, and only for the vectorscope — the histogram
            // already fills its axis. A cast worth correcting is often
            // slight enough to need more than the default.
            if app.scope_choice == Scope::Vectorscope {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Magnify")
                            .size(theme::FONT_SMALL)
                            .color(theme::muted()),
                    );
                    for mag in [1.0f64, 2.0, 4.0] {
                        let on = (app.scope_gain - mag).abs() < 0.01;
                        if theme::chip_button(ui, &format!("{mag:.0}x"), on) {
                            app.scope_gain = mag;
                        }
                    }
                });
            }
            ui.add_space(10.0);
            let kind = app.scope_choice;
            // Side by side once a correction is chosen, because the whole
            // question on this page is what the correction does to the
            // color — and a cast is far easier to see as two clouds than
            // as one you have to remember.
            let current = (page.get)(&app.config);
            let after_tag = format!(
                "stage-tint-on-{current}{}",
                app.moment_key()
            );
            if is_no_op(Step::ColorTint, current) {
                scope_panel(ui, app, &tag, kind, inner_w);
            } else {
                let each = (inner_w - 12.0) / 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.vertical(|ui| {
                        ui.set_width(each);
                        scope_panel(ui, app, &tag, kind, each);
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Before")
                                .size(theme::FONT_SMALL)
                                .color(theme::faint()),
                        );
                    });
                    ui.vertical(|ui| {
                        ui.set_width(each);
                        scope_panel(ui, app, &after_tag, kind, each);
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("After")
                                .size(theme::FONT_SMALL)
                                .color(theme::faint()),
                        );
                    });
                });
            }
            ui.add_space(10.0);
            sampling_note(ui, "Color");
        });
        ui.add_space(14.0);
    }
}

// ── Scopes ──────────────────────────────────────────────────────────────────

/// Which scope the Color tint page is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Vectorscope,
    RgbHistogram,
}

/// Draw a scope computed from the frame already on screen.
///
/// The scopes are built from the untouched preview frame, which is the same
/// frame the before/after is showing — so what the scope says and what the
/// picture shows are the same moment. Reading them from a separately fetched
/// frame would let the two drift apart on a cut.
fn scope_panel(
    ui: &mut egui::Ui,
    app: &mut App,
    tag: &str,
    kind: Scope,
    width: f32,
) -> bool {
    let Some(frame) = app.preview_rgb_for(tag) else {
        let height = 180.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        if ui.is_rect_visible(rect) {
            ui.painter()
                .rect_filled(rect, theme::RADIUS_SMALL, theme::panel2());
        }
        ui.ctx().request_repaint();
        return false;
    };

    // The gain is in the key. A cache keyed on the frame alone would hand
    // back the scope drawn at the previous magnification and the chips
    // would appear to do nothing.
    let key = format!("{tag}-{kind:?}-x{:.1}", app.scope_gain);
    if app.scope_texture(&key).is_none() {
        let gain = app.scope_gain;
        let img = match kind {
            Scope::Vectorscope => {
                scopes::vectorscope(&frame.rgb, frame.width, frame.height, gain)
            }
            Scope::RgbHistogram => {
                scopes::rgb_histogram(&frame.rgb, frame.width, frame.height)
            }
        };
        app.store_scope(&key, img, ui.ctx());
    }

    if let Some(texture) = app.scope_texture(&key) {
        let size = texture.size_vec2();
        // Vectorscopes are square and belong square; the stacked histogram
        // and the parade are wide, so they take the column width.
        let shown = match kind {
            // Square, and never wider than the space it has.
            Scope::Vectorscope => {
                let side = width.min(260.0);
                egui::vec2(side, side)
            }
            Scope::RgbHistogram => egui::vec2(width, width * size.y / size.x),
        };
        ui.vertical_centered(|ui| {
            ui.add(
                egui::Image::new(texture)
                    .fit_to_exact_size(shown)
                    .corner_radius(theme::RADIUS_SMALL),
            );
        });
        return true;
    }
    false
}

/// The RGB parade, for the Levels page.
fn parade_panel(ui: &mut egui::Ui, app: &mut App, tag: &str, width: f32) {
    let Some(frame) = app.preview_rgb_for(tag) else {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 170.0), egui::Sense::hover());
        if ui.is_rect_visible(rect) {
            ui.painter()
                .rect_filled(rect, theme::RADIUS_SMALL, theme::panel2());
        }
        ui.ctx().request_repaint();
        return;
    };

    let key = format!("{tag}-parade");
    if app.scope_texture(&key).is_none() {
        let img = scopes::parade(&frame.rgb, frame.width, frame.height);
        app.store_scope(&key, img, ui.ctx());
    }
    if let Some(texture) = app.scope_texture(&key) {
        let size = texture.size_vec2();
        ui.add(
            egui::Image::new(texture)
                .fit_to_exact_size(egui::vec2(width, width * size.y / size.x))
                .corner_radius(theme::RADIUS_SMALL),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (label, color) in [
                ("Red", theme::fix()),
                ("Green", theme::ok()),
                ("Blue", theme::info()),
            ] {
                ui.label(RichText::new(label).size(theme::FONT_TINY).color(color));
                ui.add_space(width / 3.0 - 34.0);
            }
        });
    }
}

/// What the filter changed, drawn and counted.
///
/// Reported from testing on the Halos page: "it's hard to tell if it is doing
/// anything". It was — a dehalo moves a handful of levels on a handful of
/// edges — but two pictures side by side cannot show that at any zoom, so the
/// page gave no way to tell a working filter from a broken one. The honest
/// fix is not a stronger filter, it is showing the difference.
fn difference_panel(
    ui: &mut egui::Ui,
    app: &mut App,
    before_tag: &str,
    after_tag: &str,
    width: f32,
) {
    let key = format!("diff-{before_tag}-{after_tag}-x{:.0}", app.diff_gain);

    // Both halves have to be in hand. Differencing against a half that has
    // not rendered yet would draw the whole frame as changed.
    let stats = match (
        app.preview_rgb_for(before_tag),
        app.preview_rgb_for(after_tag),
    ) {
        (Some(b), Some(a)) if b.width == a.width && b.height == a.height => {
            let (img, stats) =
                scopes::difference(&b.rgb, &a.rgb, b.width, b.height, app.diff_gain);
            if app.scope_texture(&key).is_none() {
                app.store_scope(&key, img, ui.ctx());
            }
            Some(stats)
        }
        _ => None,
    };
    let Some(stats) = stats else { return };

    ui.add_space(10.0);
    let mut open = app.is_open("diff-panel");
    if theme::disclosure(ui, "What changed", &mut open) {
        app.queue_open("diff-panel", open);
    }
    if !open {
        // Even closed, the headline number is worth having: it answers the
        // question without making anyone open anything.
        ui.add_space(4.0);
        ui.label(
            RichText::new(difference_sentence(&stats))
                .size(theme::FONT_SMALL)
                .color(if stats.peak == 0 { theme::fix() } else { theme::muted() }),
        );
        return;
    }

    ui.add_space(8.0);
    if let Some(texture) = app.scope_texture(&key) {
        let size = texture.size_vec2();
        let shown = egui::vec2(width, (width * size.y / size.x).floor());
        ui.add(
            egui::Image::new(texture)
                .fit_to_exact_size(shown)
                .corner_radius(theme::RADIUS_SMALL),
        );
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Amplify")
                .size(theme::FONT_SMALL)
                .color(theme::muted()),
        );
        for g in [4.0f32, 8.0, 16.0, 32.0] {
            let on = (app.diff_gain - g).abs() < 0.01;
            if theme::chip_button(ui, &format!("{g:.0}x"), on) {
                app.diff_gain = g;
            }
        }
    });
    ui.add_space(6.0);
    theme::paragraph(ui, RichText::new(difference_sentence(&stats))
                .size(theme::FONT_SMALL)
                .color(if stats.peak == 0 { theme::fix() } else { theme::text() }),);
    ui.add_space(4.0);
    theme::paragraph(ui, RichText::new(
                "Black is untouched; everything lit is what this setting altered, brightened \
                 so a change of a few levels can be seen at all. The brightness here is not \
                 the size of the change — the sentence above is.",
            )
            .size(theme::FONT_TINY)
            .color(theme::faint()),);
}

/// The difference as a sentence, which is what actually settles the question.
fn difference_sentence(d: &scopes::Difference) -> String {
    if d.peak == 0 {
        return "This setting changed nothing at all in this frame. Try another moment \
                before concluding it does nothing — a filter that acts on edges has \
                nothing to do on a flat shot."
            .to_string();
    }
    format!(
        "{:.0}% of the picture changed. The largest change to any pixel is {} \
         {} out of 255, and the average across the frame is {:.2}.",
        d.changed * 100.0,
        d.peak,
        if d.peak == 1 { "level" } else { "levels" },
        d.mean,
    )
}

/// Where the numbers on these pages come from, said plainly.
///
/// Worth stating because the two things on screen are measured differently:
/// the recommendation comes from many frames so one odd scene cannot drive
/// it, while a scope can only ever show one. Without this the user has no
/// way to know which they are looking at.
fn sampling_note(ui: &mut egui::Ui, what: &str) {
    theme::paragraph(ui, RichText::new(format!(
                "{what} was measured across 14 frames spaced through the whole video, so one unusual scene cannot drive the recommendation. The scope above shows a single frame — move the Moment slider to look at another part of the tape."
            ))
            .size(theme::FONT_TINY)
            .color(theme::faint()),);
}

/// Which filter stage a page previews, if any.
fn stage_for(step: Step) -> Option<vcg_core::vpy::PreviewStage> {
    use vcg_core::vpy::PreviewStage as P;
    Some(match step {
        Step::Levels => P::Levels,
        Step::ColorTint => P::ColorTint,
        Step::Noise => P::Noise,
        Step::Halos => P::Halos,
        Step::FilmGrain => P::FilmGrain,
        Step::Dithering => P::Dither,
        _ => return None,
    })
}

/// Whether the chosen option leaves the picture alone.
///
/// Not simply "option zero". On most pages the first choice is the one that
/// does nothing, but Smooth gradient fix is the other way round: its default
/// *is* the correction, and the do-nothing choice is last. Treating index
/// zero as neutral there showed one large still on the default and only
/// produced a comparison once the user picked something — exactly backwards,
/// since the default is the thing worth seeing.
fn is_no_op(step: Step, current: usize) -> bool {
    match step {
        // 0 = error diffusion, 1 = ordered, 2 = none.
        Step::Dithering => current == 2,
        _ => current == 0,
    }
}

/// A before/after pair for the current setting.
///
/// Both halves run the identical script and differ only in whether the one
/// stage is applied, so what changes between them is the filter and nothing
/// else. Rendering the whole export pipeline for each would be slower and
/// would mix in every other stage's effect, which is not the question the
/// page is asking.
///
/// When the setting is at "do nothing" there is nothing to compare, so one
/// picture is shown rather than two identical ones side by side.
fn stage_pair(ui: &mut egui::Ui, app: &mut App, step: Step, current: usize) {
    let Some(stage) = stage_for(step) else { return };
    if app.file.is_none() {
        return;
    }

    let strength = current.to_string();
    let (before_tag, after_tag) = app.request_stage_pair(stage, &strength);
    let inner = COLUMN - 34.0;
    let gap = 12.0;
    let each = ((inner - gap) / 2.0).floor();

    theme::card().show(ui, |ui| {
        ui.set_width(inner);

        let mut pan = None;
        if is_no_op(step, current) {
            // Nothing is being applied: a pair would be two identical
            // pictures claiming to show a difference.
            pan = pane(ui, app, &before_tag, inner, "Your video, unchanged");
        } else {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                // Either half can be dragged, and both move together.
                let a = pane(ui, app, &before_tag, each, "Before");
                let b = pane(ui, app, &after_tag, each, "After");
                pan = a.or(b);
            });
        }
        if let Some(d) = pan {
            app.zoom_at += d;
        }

        // What changed, for the filters whose whole point is to be subtle.
        if !is_no_op(step, current) {
            difference_panel(ui, app, &before_tag, &after_tag, inner);
        }

        ui.add_space(12.0);
        comparison_controls(ui, app, inner);

        // Only once there are two halves to explain. On the untouched state
        // there is one picture and no wait worth accounting for, and a
        // paragraph about why the comparison is slow is noise in front of a
        // reader who has not asked for a comparison yet.
        if !is_no_op(step, current) {
            ui.add_space(8.0);
            theme::paragraph(ui, RichText::new("Both halves are deinterlaced, so this is what the finished video will look like. That is why the first one takes a few seconds: the untouched half is the same picture on Levels, Color tint, Noise, Halos, Film grain and Smooth gradient fix, so it is rendered once and reused on all six. Moving the Moment slider starts again at the new moment.")
                        .size(theme::FONT_TINY)
                        .color(theme::faint()),);
        }
    });
    ui.add_space(18.0);
}

/// One half of a comparison, magnified and panned by the shared controls.
///
/// Zoom is done with the texture's UV rectangle rather than by re-rendering:
/// the frame is already at output resolution, so enlarging a part of it costs
/// nothing and keeps both halves in exact step. Re-rendering a crop would be
/// slower and would let the two sides drift apart by a pixel.
fn pane(ui: &mut egui::Ui, app: &App, tag: &str, width: f32, caption: &str) -> Option<egui::Vec2> {
    let mut dragged = None;
    ui.vertical(|ui| {
        ui.set_width(width);
        match app.setting_preview_for(tag) {
            Some(texture) => {
                let size = texture.size_vec2();
                let shown = egui::vec2(width, (width * size.y / size.x).floor());
                let (rect, response) =
                    ui.allocate_exact_size(shown, egui::Sense::click_and_drag());

                let half = 0.5 / app.zoom.max(1.0);
                // Clamped so the view cannot be dragged off the picture,
                // which would show the texture's edge clamp as smeared bars.
                let cx = app.zoom_at.x.clamp(half, 1.0 - half);
                let cy = app.zoom_at.y.clamp(half, 1.0 - half);
                let uv = egui::Rect::from_min_max(
                    egui::pos2(cx - half, cy - half),
                    egui::pos2(cx + half, cy + half),
                );

                if ui.is_rect_visible(rect) {
                    egui::Image::new(texture)
                        .uv(uv)
                        .corner_radius(theme::RADIUS_SMALL)
                        .paint_at(ui, rect);
                }
                if app.zoom > 1.0 {
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    if response.dragged() {
                        // In frame coordinates, so a drag moves the picture
                        // with the pointer at every magnification.
                        let d = response.drag_delta();
                        dragged = Some(egui::vec2(
                            -d.x / rect.width() * (uv.width()),
                            -d.y / rect.height() * (uv.height()),
                        ));
                    }
                }
            }
            None => {
                let height = (width * 3.0 / 4.0).floor();
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter()
                        .rect_filled(rect, theme::RADIUS_SMALL, theme::panel2());
                }
                ui.ctx().request_repaint();
            }
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(caption)
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
        );
        // How long this one has been going, and why.
        //
        // Reported from testing: a long tape leaves the panes gray for a
        // while with nothing to say whether anything is happening. There is
        // no frame count to show — one frame is being rendered — so this
        // says what it is doing and for how long, which is the honest
        // version of the progress bar that was asked for.
        if let Some(line) = app.preview_progress_for(tag) {
            ui.add_space(2.0);
            theme::paragraph(ui, RichText::new(line).size(theme::FONT_TINY).color(theme::faint()),);
        }
    });
    dragged
}

/// The controls shared by every comparison: which moment, and how close.
///
/// One set for all the pages, so moving to a different moment compares the
/// same moment everywhere rather than leaving each page on its own frame.
pub fn comparison_controls(ui: &mut egui::Ui, app: &mut App, width: f32) {
    let duration = app
        .report
        .as_ref()
        .or(app.file_info.as_ref())
        .and_then(|r| r.info)
        .map(|i| i.duration)
        .unwrap_or(0.0);

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Moment")
                .size(theme::FONT_SMALL)
                .color(theme::muted()),
        );
        ui.add_space(8.0);
        let mut at = app.preview_at;
        ui.spacing_mut().slider_width = (width - 320.0).max(120.0);
        let r = ui.add(egui::Slider::new(&mut at, 0.0..=1.0).show_value(false));
        // The value is written on every frame so the clock beside it tracks
        // the handle; what the drag suppresses is the *rendering*, not the
        // setting. The moment is part of every cache key, so nothing stale
        // can be shown once the drag ends.
        if r.changed() {
            app.set_preview_at(at);
        }
        app.set_scrubbing(r.dragged());
        ui.add_space(8.0);
        ui.label(
            RichText::new(crate::trim::clock(duration * app.preview_at as f64))
                .size(theme::FONT_SMALL)
                .color(theme::text()),
        );
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Zoom")
                .size(theme::FONT_SMALL)
                .color(theme::muted()),
        );
        ui.add_space(8.0);
        for level in [1.0f32, 2.0, 4.0, 8.0] {
            let on = (app.zoom - level).abs() < 0.01;
            let label = if level == 1.0 {
                "Fit".to_string()
            } else {
                format!("{level:.0}x")
            };
            if theme::chip_button(ui, &label, on) {
                app.zoom = level;
                // Back to the middle on a change of magnification: a pan
                // from the previous level is meaningless at the new one.
                app.zoom_at = egui::vec2(0.5, 0.5);
            }
        }
        if app.zoom > 1.0 {
            ui.add_space(10.0);
            ui.label(
                RichText::new("drag the picture to move around")
                    .size(theme::FONT_TINY)
                    .color(theme::faint()),
            );
        }
    });
}

/// What the analysis measured for this setting, if anything.
/// What the analysis found, and what to do about it.
///
/// Two separate things, and they were run together in one paragraph. A reader
/// could not tell which half was a fact about their tape and which was our
/// opinion of it — and on the Levels page those two point opposite ways: the
/// picture *is* outside the legal range, and the advice *is* to leave it
/// alone. Splitting them lets both be said plainly.
pub struct Analysis {
    /// What was measured, in numbers. Never a verdict on its own: "Light
    /// haloing" is a bucket, "12% of strong edges overshoot" is the finding
    /// the bucket came from, and the second is what lets someone disagree.
    measurement: String,
    recommendation: String,
}

fn analysis_for(app: &App, step: Step) -> Option<Analysis> {
    let report = app.report.as_ref()?;
    // The option our own recommendation points at, named the way the page
    // names it, so the advice and the radio button cannot drift apart.
    let recommended = page_for(step)
        .and_then(|p| p.options.get(recommended_index(app, step)))
        .map(|o| o.label);

    let a = match step {
        Step::Levels => {
            let l = report.levels.as_ref()?;
            let (min, max) = (l.min_y.unwrap_or(0), l.max_y.unwrap_or(255));
            let hist = report.histogram.as_ref();
            let share = hist.map(|h| (h.below_legal() * 100.0, h.above_legal() * 100.0));
            let measurement = match share {
                Some((below, above)) if below > 0.0 || above > 0.0 => format!(
                    "Brightness runs from {min} to {max} on a 0-255 scale, against a legal \
                     window of 16 to 235. {below:.2}% of the picture sits below the floor \
                     and {above:.2}% above the ceiling. Measured from still frames sampled \
                     across the file — ten for the range, fourteen spanning the middle 80% \
                     for the percentages — not from every frame."
                ),
                _ => format!(
                    "Brightness runs from {min} to {max} on a 0-255 scale. The legal window \
                     is 16 to 235, so all of it is inside. Measured from ten still frames \
                     sampled across the file, not from every frame."
                ),
            };
            let recommendation = if l.needs_adjustment {
                "Leave the brightness alone. The values outside the legal range are real \
                 picture the deck recorded — detail in a bright window or a dark corner — \
                 and clamping throws them away permanently. Only clamp if you have been \
                 asked for a broadcast-legal file."
                    .to_string()
            } else {
                "Leave the brightness alone. Nothing is outside the legal range, so there \
                 is nothing for either correction to do but move picture that is already \
                 where it should be."
                    .to_string()
            };
            Analysis {
                measurement,
                recommendation,
            }
        }

        Step::ColorTint => {
            let c = report.color.as_ref()?;
            match c.color_cast {
                Some(cast) => Analysis {
                    measurement: format!(
                        "A consistent {cast:?} cast across the whole tape. Neutral gray is \
                         128 on both color axes; this video averages {:.1} on U and {:.1} \
                         on V, which is {:+.1} and {:+.1} away.",
                        128.0 + c.u_correction,
                        128.0 + c.v_correction,
                        c.u_correction,
                        c.v_correction
                    ),
                    recommendation: format!(
                        "Correct it{}. A cast this consistent is the tape or the capture \
                         card, not the scene — correcting it moves the whole picture back \
                         towards neutral without touching anything else.",
                        recommended.map(|r| format!(" — \"{r}\"")).unwrap_or_default()
                    ),
                },
                None => Analysis {
                    measurement: "No consistent color cast. Neutral gray is 128 on both \
                                  color axes and this video sits within a point of it on \
                                  each."
                        .to_string(),
                    recommendation: "Leave the color alone. There is no cast to remove, \
                                     and a correction would introduce one."
                        .to_string(),
                },
            }
        }

        Step::Noise => {
            let n = report.noise.as_ref().filter(|n| n.analyzed)?;
            // The number the classifier actually decides on, not the bucket
            // it put the tape in. TOUT counts pixels that flicker out of step
            // with their neighbors — speckle rather than motion — which is
            // what tape noise looks like to a measurement.
            let measurement = format!(
                "{:.2}% of pixels flicker independently of their neighbors, measured \
                 across {} samples. That is the speckle a tape adds: under 1.2% is clean, \
                 1.2-3% light, 3-8% moderate, above 8% heavy. Frame-to-frame change \
                 averages {:.1} levels.",
                n.avg_variance * 100.0,
                n.samples_analyzed,
                n.avg_diff
            );
            let recommendation = match n.recommendation {
                vcg_core::analysis::Recommendation::None => {
                    "Leave it alone. There is little enough speckle that denoising would \
                     cost more detail than it removes noise."
                        .to_string()
                }
                _ => format!(
                    "Denoise{}. Tape speckle is the one thing on this page that gets worse \
                     every time the file is re-encoded, because the encoder spends bits \
                     describing noise instead of picture.",
                    recommended.map(|r| format!(" — \"{r}\"")).unwrap_or_default()
                ),
            };
            Analysis {
                measurement,
                recommendation,
            }
        }

        Step::Halos => {
            let h = report.halo.as_ref().filter(|h| h.analyzed)?;
            let measurement = format!(
                "{:.0}% of strong edges overshoot — a bright rim on the light side of a \
                 hard edge, a dark one on the other. Under 5% is clean, 5-15% light, \
                 above 15% strong.",
                h.halo_ratio * 100.0
            );
            let recommendation = match h.recommendation {
                vcg_core::analysis::Recommendation::None => {
                    "Leave the edges alone. There is not enough ringing here to be worth \
                     the softening that removing it costs."
                        .to_string()
                }
                _ => format!(
                    "Remove it{}. Haloing is added by the deck and the capture card \
                     sharpening an already-sharp edge, so it is not detail being lost — \
                     but watch the fine detail next to edges, which is what the correction \
                     can soften.",
                    recommended.map(|r| format!(" — \"{r}\"")).unwrap_or_default()
                ),
            };
            Analysis {
                measurement,
                recommendation,
            }
        }

        Step::ColorBleeding => {
            use vcg_core::analysis::classify::BleedLevel;
            let b = report.bleeding.as_ref()?;
            let measurement = format!(
                "Color edges sit {:.1} pixels wider than the luma edges they belong to \
                 horizontally, {:.1} vertically, at an average saturation of {:.0}. That \
                 scores {:.0}, which reads as {:?}.",
                b.avg_h_spread, b.avg_v_spread, b.avg_saturation, b.bleeding_score, b.bleed_level
            );
            let recommendation = if b.recommendation {
                "Try +1, then look at a hard edge. Chroma arrives late through a tape \
                 machine, so the color sits to the right of the thing it belongs to and \
                 the fix moves it left — the plus end of the slider. If the fringe jumps \
                 to the other side, you have gone one too far."
                    .to_string()
            } else {
                match b.bleed_level {
                    BleedLevel::None => "Leave this at zero. There is no measurable bleeding \
                                         to correct, and a shift would put a color fringe \
                                         on an edge that does not have one."
                        .to_string(),
                    _ => "Leave this at zero. There is a little bleeding, but not enough to \
                          be worth a whole-pixel shift: at this level it is about as likely \
                          to put a fringe on the other side of an edge as to take one off."
                        .to_string(),
                }
            };
            Analysis {
                measurement,
                recommendation,
            }
        }

        _ => return None,
    };
    Some(a)
}

/// Which option the analysis points at, so the page can mark it.
///
/// `usize::MAX` when there is nothing to recommend, which no index matches.
fn recommended_index(app: &App, step: Step) -> usize {
    let Some(report) = app.report.as_ref() else {
        return usize::MAX;
    };
    match step {
        // Always "leave it alone", even when the picture is outside the
        // legal range. Clamping is the only option here that destroys
        // data, and out-of-range values on a tape capture are usually real
        // picture the deck recorded rather than an error to be corrected.
        // Anyone who needs broadcast compliance knows to ask for it; nobody
        // benefits from being nudged into it by default.
        Step::Levels => report.levels.as_ref().map_or(usize::MAX, |_| 0),
        Step::ColorTint => report
            .color
            .as_ref()
            .map_or(usize::MAX, |c| if c.color_cast.is_some() { 1 } else { 0 }),
        Step::Noise => report.noise.as_ref().filter(|n| n.analyzed).map_or(
            usize::MAX,
            |n| match n.recommendation {
                vcg_core::analysis::Recommendation::Heavy
                | vcg_core::analysis::Recommendation::Strong => 2,
                vcg_core::analysis::Recommendation::None => 0,
                _ => 1,
            },
        ),
        Step::Halos => report.halo.as_ref().filter(|h| h.analyzed).map_or(
            usize::MAX,
            |h| match h.recommendation {
                vcg_core::analysis::Recommendation::Strong
                | vcg_core::analysis::Recommendation::Heavy => 2,
                vcg_core::analysis::Recommendation::None => 0,
                _ => 1,
            },
        ),
        _ => usize::MAX,
    }
}

/// The luma histogram, drawn.
///
/// The probe can already say "brightness runs from 2 to 252", but two numbers
/// cannot show shape — whether the picture is bunched in the middle, clipped
/// flat against black, or spread evenly. That shape is the reason to look at
/// levels at all, so it is drawn: the legal 16-235 window lit, everything
/// outside it dimmed and marked, and the two limits ruled.
fn levels_graph(ui: &mut egui::Ui, hist: &LumaHistogram, title: &str) {
    const HEIGHT: f32 = 120.0;
    let inner = COLUMN - 34.0;

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        ui.label(
            RichText::new(title)
                .size(theme::FONT_BODY)
                .color(theme::text()),
        );
        ui.add_space(8.0);

        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(inner, HEIGHT), egui::Sense::hover());
        if ui.is_rect_visible(rect) {
            let p = ui.painter_at(rect);
            let x_of = |value: usize| rect.left() + rect.width() * (value as f32 / 255.0);

            p.rect_filled(rect, theme::RADIUS_SMALL, theme::bg());

            // The illegal margins, shaded, so "outside the range" is a place
            // on the picture rather than a sentence.
            let lo = x_of(LEGAL_MIN as usize);
            let hi = x_of(LEGAL_MAX as usize);
            for band in [
                egui::Rect::from_min_max(rect.left_top(), egui::pos2(lo, rect.bottom())),
                egui::Rect::from_min_max(egui::pos2(hi, rect.top()), rect.right_bottom()),
            ] {
                p.rect_filled(band, 0.0, theme::panel2());
            }

            // The bars, with anything outside the legal window marked.
            let bars = hist.normalized();
            let w = (rect.width() / 256.0).max(1.0);
            for (value, height) in bars.iter().enumerate() {
                if *height <= 0.0 {
                    continue;
                }
                let x = x_of(value);
                let top = rect.bottom() - (rect.height() - 8.0) * *height;
                let outside = value < LEGAL_MIN as usize || value > LEGAL_MAX as usize;
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x, top),
                        egui::pos2(x + w, rect.bottom()),
                    ),
                    0.0,
                    if outside { theme::fix() } else { theme::accent() },
                );
            }

            // This moment, drawn over the top as a line.
            //
            // The limits.
            for x in [lo, hi] {
                p.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(1.0, theme::line_hover()),
                );
            }
        }

        // The scale. 16 and 235 carry the ticks that matter — they are where
        // the legal window starts and ends — so they are labeled even though
        // they fall at awkward places, and the round numbers fill in between.
        const TICKS: [(u8, &str); 7] = [
            (0, "0"),
            (16, "16"),
            (64, "64"),
            (128, "128"),
            (192, "192"),
            (235, "235"),
            (255, "255"),
        ];
        let (axis, _) = ui.allocate_exact_size(egui::vec2(inner, 18.0), egui::Sense::hover());
        if ui.is_rect_visible(axis) {
            let p = ui.painter_at(axis);
            for (value, label) in TICKS {
                let x = axis.left() + axis.width() * (value as f32 / 255.0);
                // Ticks at the very ends would be drawn half outside the
                // panel, so their labels are pulled inward instead.
                let align = match value {
                    0 => egui::Align2::LEFT_TOP,
                    255 => egui::Align2::RIGHT_TOP,
                    _ => egui::Align2::CENTER_TOP,
                };
                let legal = value == 16 || value == 235;
                p.line_segment(
                    [
                        egui::pos2(x, axis.top()),
                        egui::pos2(x, axis.top() + if legal { 5.0 } else { 3.0 }),
                    ],
                    egui::Stroke::new(1.0, if legal { theme::line_hover() } else { theme::line() }),
                );
                p.text(
                    egui::pos2(x, axis.top() + 6.0),
                    align,
                    label,
                    egui::FontId::proportional(theme::FONT_TINY),
                    if legal { theme::muted() } else { theme::faint() },
                );
            }
        }

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("black")
                    .size(theme::FONT_TINY)
                    .color(theme::faint()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new("white")
                        .size(theme::FONT_TINY)
                        .color(theme::faint()),
                );
            });
        });

        ui.add_space(8.0);
        let (lo, hi) = hist.extremes().unwrap_or((0, 255));
        let below = hist.below_legal();
        let above = hist.above_legal();
        let outside = lo < LEGAL_MIN || hi > LEGAL_MAX;
        wrapped(
            ui,
            &levels_caption(lo, hi, below, above),
            theme::FONT_SMALL,
            if outside { theme::fix() } else { theme::ok() },
        );
    });
}

/// The chosen levels correction, named the way the option names it.
///
/// Taken from the page's own option list rather than written out again, so
/// the graph's title and the radio button the user clicked cannot drift
/// apart.
fn levels_label(current: usize) -> &'static str {
    LEVELS
        .options
        .get(current)
        .map(|o| o.label)
        .unwrap_or("the correction")
}

/// A gray panel the size of the graph, while the corrected frame renders.
///
/// Sized to match, so the page does not jump when the real one arrives.
fn levels_placeholder(ui: &mut egui::Ui, inner: f32) {
    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(inner, 148.0), egui::Sense::hover());
        if ui.is_rect_visible(rect) {
            ui.painter()
                .rect_filled(rect, theme::RADIUS_SMALL, theme::panel2());
        }
        ui.ctx().request_repaint();
    });
}

/// What the graph says in words, from the same numbers it is drawn from.
///
/// Reports the extremes, always. An earlier version only spoke up when more
/// than 0.05% of the picture was outside, which on the sample meant the graph
/// said "everything sits inside the legal range" directly above a card saying
/// it ran from 10 to 229 — both on screen at once, one of them wrong. A
/// tolerance is the right idea for deciding whether to *recommend* a fix; it
/// is the wrong thing to let decide what the screen *claims*.
fn levels_caption(lo: u8, hi: u8, below: f64, above: f64) -> String {
    let mut parts: Vec<String> = Vec::new();
    if lo < LEGAL_MIN {
        parts.push(format!(
            "the darkest pixels reach {lo}, below the legal floor of {LEGAL_MIN} ({})",
            share(below)
        ));
    }
    if hi > LEGAL_MAX {
        parts.push(format!(
            "the brightest reach {hi}, above the legal ceiling of {LEGAL_MAX} ({})",
            share(above)
        ));
    }
    match parts.len() {
        0 => format!(
            "Everything sits inside the legal range: {lo} to {hi}, within {LEGAL_MIN}-{LEGAL_MAX}."
        ),
        1 => format!("Brightness runs from {lo} to {hi} — {}.", parts[0]),
        _ => format!(
            "Brightness runs from {lo} to {hi} — {}, and {}.",
            parts[0], parts[1]
        ),
    }
}

/// A proportion, worded rather than printed as 0.02%.
fn share(fraction: f64) -> String {
    let pct = fraction * 100.0;
    if pct < 0.01 {
        "a handful of pixels".to_string()
    } else if pct < 1.0 {
        format!("{pct:.2}% of the picture")
    } else {
        format!("{pct:.1}% of the picture")
    }
}

/// The actual filter this page's current setting contributes, quoted.
///
/// Generated from the same code the render uses, so it cannot describe a
/// setting the export does not apply. Prose drifts — a panel reading
/// "AddGrain at variance 1.0" goes on saying it after the strength changes,
/// and nobody notices for a year. This updates when the radio button does
/// because it is the same function.
fn filter_snippet(ui: &mut egui::Ui, app: &App, step: Step) {
    let Some(stage) = stage_for(step) else { return };
    let snippet = vcg_core::vpy::stage_snippet(&app.config, stage);

    ui.add_space(12.0);
    ui.label(
        RichText::new("What this runs, with the setting you have chosen")
            .size(theme::FONT_SMALL)
            .strong()
            .color(theme::faint()),
    );
    ui.add_space(4.0);

    if snippet.trim().is_empty() {
        ui.label(
            RichText::new(
                "Nothing. This option adds no filter to the pipeline at all — the frames \
                 pass through untouched.",
            )
            .size(theme::FONT_SMALL)
            .color(theme::muted()),
        );
        return;
    }

    egui::Frame::NONE
        .fill(theme::panel2())
        .corner_radius(theme::RADIUS_SMALL)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for line in snippet.lines() {
                if line.trim().is_empty() {
                    ui.add_space(4.0);
                    continue;
                }
                // Monospace, and never wrapped: a wrapped filter call is
                // harder to read than one that scrolls, and these are the
                // lines someone would paste into a script of their own.
                ui.label(
                    RichText::new(line)
                        .size(theme::FONT_TINY)
                        .monospace()
                        .color(if line.trim_start().starts_with('#') {
                            theme::faint()
                        } else {
                            theme::text()
                        }),
                );
            }
        });
}

fn analysis_card(ui: &mut egui::Ui, a: &Analysis) {
    let inner = COLUMN - 34.0;
    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        theme::tag(ui, Tag::Info, Some("Analysis"));
        ui.add_space(10.0);

        for (heading, body, color) in [
            ("Measurement", &a.measurement, theme::muted()),
            ("Recommendation", &a.recommendation, theme::text()),
        ] {
            ui.label(
                RichText::new(heading)
                    .size(theme::FONT_SMALL)
                    .strong()
                    .color(theme::faint()),
            );
            ui.add_space(3.0);
            theme::paragraph(ui, RichText::new(body).size(theme::FONT_BODY).color(color));
            if heading == "Measurement" {
                ui.add_space(12.0);
            }
        }
    });
}

/// A radio row, for pages outside this module that want the same control.
pub fn radio(
    ui: &mut egui::Ui,
    width: f32,
    selected: bool,
    title: &'static str,
    note: &'static str,
) -> bool {
    option_row(ui, width, selected, &Opt { label: title, note }, false)
}

fn option_row(
    ui: &mut egui::Ui,
    width: f32,
    selected: bool,
    opt: &Opt,
    recommended: bool,
) -> bool {
    let height = if opt.note.is_empty() { 36.0 } else { 52.0 };
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

        let label = if recommended {
            format!("{} — recommended", opt.label)
        } else {
            opt.label.to_string()
        };
        let title_y = if opt.note.is_empty() {
            rect.center().y
        } else {
            rect.center().y - 9.0
        };
        painter.text(
            egui::pos2(rect.left() + 34.0, title_y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(theme::FONT_BODY),
            if selected { theme::text() } else { theme::muted() },
        );
        if !opt.note.is_empty() {
            painter.text(
                egui::pos2(rect.left() + 34.0, rect.center().y + 10.0),
                egui::Align2::LEFT_CENTER,
                opt.note,
                egui::FontId::proportional(theme::FONT_SMALL),
                theme::faint(),
            );
        }
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    theme::focus_ring(ui, &response, theme::RADIUS_SMALL);
    response.clicked() && !selected
}

// ── The pages ───────────────────────────────────────────────────────────────

fn page_for(step: Step) -> Option<&'static ChoicePage> {
    Some(match step {
        Step::Levels => &LEVELS,
        Step::ColorTint => &COLOUR_TINT,
        Step::Noise => &NOISE,
        Step::Halos => &HALOS,
        Step::FilmGrain => &FILM_GRAIN,
        Step::Dithering => &DITHERING,
        Step::Upscale => &UPSCALE,
        _ => return None,
    })
}

static LEVELS: ChoicePage = ChoicePage {
    title: "Levels",
    lead: "How bright the darkest and lightest parts of the picture are allowed to be.",
    options: &[
        Opt {
            label: "Leave the brightness alone",
            note: "Keeps every value the deck recorded. Highlights above 235 stay in the file but still look washed out on playback.",
        },
        Opt {
            label: "Clamp to legal (16-235)",
            note: "Cuts everything outside the window away for good. Only for broadcast delivery.",
        },
        Opt {
            label: "Stretch to fill the range",
            note: "Pulls the whole range into 16-235, so detail above the ceiling becomes visible. Changes every pixel.",
        },
    ],
    get: |c| match c.levels_adjustment {
        LevelsAdjustment::None => 0,
        LevelsAdjustment::Clamp => 1,
        LevelsAdjustment::Stretch => 2,
    },
    set: |c, i| {
        c.levels_adjustment = match i {
            1 => LevelsAdjustment::Clamp,
            2 => LevelsAdjustment::Stretch,
            _ => LevelsAdjustment::None,
        }
    },
    details: &[
        "Broadcast video keeps black at luma 16 and white at 235, leaving headroom either side. Tape captures routinely sit outside that: a capture card with its levels misaligned will put black at 0 or white at 255, and the detail beyond the limits is still there in the file.",
        "How this was measured. Fourteen frames spanning the middle 80% of the video are decoded at native resolution in 4:4:4, and every luma value in them is counted. The frames are read as stored rather than converted to grayscale, because a grayscale conversion expands 16-235 to 0-255 and the whole question here is where the picture sits against those two numbers.",
        "Twenty pixels are ignored at every edge before counting. Capture-card blanking is not dark picture, it is the absence of a signal, and it sits at or below the legal floor on every frame of every tape; the head-switching band along the bottom is torn signal that can read as either extreme. Counted with the picture, those edges put the black point on the floor and report a healthy tape as crushed. The inset is comfortably clear of the worst blanking seen here — the measured crop on this video is listed on the Crop page — and still leaves around 97% of the frame, which is far more picture than a histogram needs.",
        "What the three actually do, because the difference matters and two of them are easy to mix up. Clamp *cuts*: everything below 16 becomes 16 and everything above 235 becomes 235, and that detail is gone from the file for good. It does not compress the range — it discards the ends of it. Stretch *maps*: this video's own measured range is expanded to fill 16-235, so values that were above the ceiling come down into view. Leave alone does nothing at all.",
        "Which to pick, with a worked example. Say a white shirt with a woven pattern sits up around 250. Leave alone keeps that pattern in the file, but the file is tagged limited-range, so players show everything over 235 as flat white and the pattern looks washed out on screen. Clamp destroys it permanently — it is the one choice that cannot be undone later. Stretch pulls it down into the visible window and the pattern appears. That is why a scene can look best under Stretch even though Stretch is the heaviest of the three.",
        "The catch with Stretch is that it is one transform for the whole video, built from frames sampled across all of it. A scene that was already well exposed gets flattened slightly to make room for the one that was not. Park the Moment slider on a bright scene, flip between the three options, and look at the preview — then do the same on a normally lit scene before deciding. The numbers under \"What changed\" tell you how much moved.",
        "If you are grading this later in Resolve or similar, leave it alone. Those tools want the untouched range, and both Clamp and Stretch throw away headroom they could have used. Stretch is for the file people will actually watch.",
    ],
};

static COLOUR_TINT: ChoicePage = ChoicePage {
    title: "Color tint",
    lead: "Whether to correct an overall color cast, and how saturated the result should be.",
    options: &[
        Opt {
            label: "Leave the color alone",
            note: "",
        },
        Opt {
            label: "Remove the measured cast",
            note: "Shifts the color back to neutral by the amount measured.",
        },
        Opt {
            label: "Remove the cast and lift saturation",
            note: "For tape that has gone flat as well as off-color.",
        },
        Opt {
            label: "Just lift saturation",
            note: "No cast correction. For a dull but neutral picture.",
        },
        Opt {
            label: "Reduce saturation",
            note: "For a capture whose color is overcooked.",
        },
    ],
    get: |c| match c.color_correction {
        ColorCorrection::None => 0,
        ColorCorrection::AutoFix => 1,
        ColorCorrection::AutoFixBoost => 2,
        ColorCorrection::BoostSat => 3,
        ColorCorrection::ReduceSat => 4,
    },
    set: |c, i| {
        c.color_correction = match i {
            1 => ColorCorrection::AutoFix,
            2 => ColorCorrection::AutoFixBoost,
            3 => ColorCorrection::BoostSat,
            4 => ColorCorrection::ReduceSat,
            _ => ColorCorrection::None,
        }
    },
    details: &[
        "The cast is measured as the average U and V offset from neutral (128) across sampled frames, and the correction is a flat shift of the chroma planes by that amount. It is deliberately a constant offset, not a curve: a tape's cast comes from the chemistry and the deck, and is the same across the whole recording.",
        "Only a cast the probe judged *consistent* is offered. A large but scattered offset is scene content — a lot of sky, a lot of grass — and shifting for that would drain the color out of the scene.",
        "Saturation changes are applied in the same pass. Boosting saturation on a tape with chroma noise will boost the noise too, so if the picture is both dull and noisy, set the noise filter first and come back.",
    ],
};

static NOISE: ChoicePage = ChoicePage {
    title: "Noise",
    lead: "Tape adds a fine, restless speckle that was never in the room. Removing it makes the picture calmer and the file smaller — but push too hard and real detail goes with it.",
    options: &[
        Opt {
            label: "No noise removal",
            note: "Keeps every detail, and every bit of grain.",
        },
        Opt {
            label: "Moderate",
            note: "Cleans obvious speckle while leaving fine detail alone.",
        },
        Opt {
            label: "Heavy",
            note: "For badly degraded tape. Can soften detail and smear movement.",
        },
    ],
    get: |c| match c.noise_level {
        NoiseLevel::None => 0,
        NoiseLevel::Moderate => 1,
        NoiseLevel::Heavy => 2,
    },
    set: |c, i| {
        c.noise_level = match i {
            1 => NoiseLevel::Moderate,
            2 => NoiseLevel::Heavy,
            _ => NoiseLevel::None,
        }
    },
    details: &[
        "Denoising runs after the deinterlacer, on the progressive frames, using a temporal degrain that compares each frame with its neighbors. Noise is random between frames while picture is not, which is what makes it separable at all.",
        "The cost is motion: anything that moves fast enough to defeat the motion search gets averaged with where it was, which reads as smearing. Heavy is genuinely heavy — worth it for a tape that is mostly noise, and worth avoiding otherwise.",
        "QTGMC does no denoising at all. It is configured to deinterlace and nothing else, so grain and texture reach this step untouched and whatever you choose here is the only noise work done to the picture. It used to do some of its own, which meant a tape got partly denoised even with this set to leave it alone.",
        "The tool is BM3D — block-matching in three dimensions, which gathers patches that look alike from this frame and its neighbors, filters them together in the frequency domain, and puts them back. It is the best there is for tape grain, because tape grain is exactly the kind of noise that does not repeat from patch to patch while real detail does. It also looks at the frames either side, not just this one.",
        "BM3D used to be the slow part of an export. The generated script now picks the fastest build your machine has: the CUDA one if you have an NVIDIA card, otherwise an AVX2 build that is four times quicker than the original, and the original itself if neither is present. On the machine this was measured on, denoising went from costing two-thirds of the export to costing almost nothing — the same picture, and the same settings, just better arithmetic.",
        "Below those, the script falls back to KNLMeansCL and then to SMDegrain from havsfunc, so the same .vpy runs wherever it lands. Only the luma is denoised; chroma is left alone, since tape chroma is already soft and filtering it further costs more than it saves.",
    ],
};

static HALOS: ChoicePage = ChoicePage {
    title: "Halos",
    lead: "Whether to remove the bright outlines that tape machines and capture cards add to edges.",
    options: &[
        Opt {
            label: "Leave the edges alone",
            note: "",
        },
        Opt {
            label: "Light",
            note: "Takes the worst of the outline off without softening the edge.",
        },
        Opt {
            label: "Strong",
            note: "For heavy ringing. Watch for softened detail next to edges.",
        },
    ],
    get: |c| match c.dehalo_mode {
        DehaloMode::None => 0,
        DehaloMode::Light => 1,
        DehaloMode::Strong => 2,
    },
    set: |c, i| {
        c.dehalo_mode = match i {
            1 => DehaloMode::Light,
            2 => DehaloMode::Strong,
            _ => DehaloMode::None,
        }
    },
    details: &[
        "The tool is FineDehalo — an edge-masked DeHalo_alpha. It blurs the picture down and back up to model what the edge would look like without the halo, repairs the original against that model, then applies the result only where a Prewitt edge mask finds a strong edge, with a second shrunken mask holding fine detail out of the way. Luma only.",
        "It is written into the generated script in full rather than called from havsfunc, because the bundled havsfunc has removed FineDehalo and DeHalo_alpha — they now raise an error pointing at vs-dehalo, which is not bundled. The port needs only core std/resize operations, rgvs.Repair from RemoveGrainVS, and the mask helpers havsfunc still has, so it cannot break again when the library moves.",
        "A halo is the bright band that sits just outside a strong edge — the signature of the sharpening circuit in a VHS deck or a capture card trying to make the picture look crisper than it is. It is added detail that was never on the tape.",
        "The probe scans rows horizontally, looks at every strong luma edge, and measures whether the pixels just past the bright side overshoot the plateau further out. The score is the fraction of edges that overshoot. Horizontal-only scanning is deliberate: analog sharpening works along the scanline, and reading rows is immune to interlacing comb.",
        "Dehaloing works by darkening those overshoots back toward their surroundings, so it necessarily touches real detail that happens to sit beside an edge. Strong on a picture that only had light haloing will look worse, not better.",
    ],
};

static FILM_GRAIN: ChoicePage = ChoicePage {
    title: "Film grain",
    lead: "Whether to add a little grain back after everything else has been cleaned up. If you denoised at Heavy, adding some grain here will make the picture look more natural: Heavy strips out the fine texture along with the noise, and a picture with none left reads as waxy rather than clean.",
    options: &[
        Opt {
            label: "No grain",
            note: "",
        },
        Opt {
            label: "A little",
            note: "Takes the plastic look off a heavily denoised picture.",
        },
        Opt {
            label: "Noticeable",
            note: "A film-like texture over the whole frame.",
        },
    ],
    get: |c| {
        if c.grain_strength <= 0.0 {
            0
        } else if c.grain_strength < 2.0 {
            1
        } else {
            2
        }
    },
    set: |c, i| {
        c.grain_strength = match i {
            1 => 1.0,
            2 => 3.0,
            _ => 0.0,
        }
    },
    details: &[
        "Grain is added last, after the upscale and before the final bit-depth reduction, as gaussian noise over the luma. Adding it before an upscale would magnify the grain along with the picture and defeat the point.",
        "This exists because denoising and deinterlacing both remove high-frequency detail, and a picture with none left reads as waxy — the eye expects texture in a photographic image and notices its absence more than it notices noise. A small amount of grain restores that impression without restoring the noise.",
        "It also helps the encoder: a little dither-like texture breaks up the flat gradients that otherwise band in dark scenes. If you are seeing banding in skies or walls, a touch of grain is often a better fix than a higher bitrate.",
        "How this relates to Smooth gradient fix, the step before it: error diffusion already adds a texture of its own. That is how it works — it pushes the rounding error of each pixel into its neighbors, which breaks a band into a fine dither pattern rather than a hard step. So if you chose error diffusion and your only complaint was banding, the banding is already dealt with and you need no grain at all.",
        "Grain is the stronger medicine and it is for a different complaint. Dither moves about one level and is meant to be invisible; grain here is one to three levels of variance and is meant to be seen, because its job is to put back the texture that denoising and deinterlacing took out. A heavily denoised picture looks waxy whether or not it bands, and dither will not fix that.",
        "What to pick, by what you chose on the Noise page. Heavy is the setting that most needs this — it is the one that takes out enough texture for the result to look artificial, and A little is usually enough to undo that impression without putting any of the speckle back. Moderate benefits from A little on a soft source and often needs nothing on a sharp one. If you left the noise alone, the texture is still there and you want none: adding grain on top of grain only makes the file bigger.",
    ],
};

static DITHERING: ChoicePage = ChoicePage {
    title: "Smooth gradient fix",
    lead: "Stops large areas of near-flat color — a sky, a wall, a studio backdrop — coming out as visible bands instead of a smooth blend.",
    options: &[
        Opt {
            label: "Error diffusion",
            note: "The default. Spreads the rounding error into neighboring pixels.",
        },
        Opt {
            label: "Ordered",
            note: "A fixed pattern. Faster, and more predictable on flat areas.",
        },
        Opt {
            label: "No dithering",
            note: "Straight truncation. Expect banding in gradients.",
        },
    ],
    get: |c| {
        if !c.dither_enabled {
            2
        } else if c.dither_method == DitherMethod::Ordered {
            1
        } else {
            0
        }
    },
    set: |c, i| match i {
        1 => {
            c.dither_enabled = true;
            c.dither_method = DitherMethod::Ordered;
        }
        2 => c.dither_enabled = false,
        _ => {
            c.dither_enabled = true;
            c.dither_method = DitherMethod::ErrorDiffusion;
        }
    },
    details: &[
        "The whole pipeline runs at 16 bits so that repeated filtering does not accumulate rounding error. The finished file is 10-bit for ProRes and 8-bit for everything else, so the last step throws away the extra precision — and how it throws it away decides whether smooth gradients come out smooth.",
        "Error diffusion (fmtc dmode=3, Floyd-Steinberg) carries each pixel's rounding error into its neighbors, so the error averages out across an area instead of accumulating into visible steps. Ordered (dmode=0) uses a fixed Bayer matrix: slightly more visible as a pattern, but it does not move with the picture, which some people prefer on static shots.",
        "Turning it off truncates. On a tape capture you will see it immediately in any large area of near-flat color — a wall, a sky, a studio backdrop — as distinct bands rather than a gradient. There is no reason to choose it except to see what it looks like.",
    ],
};

static UPSCALE: ChoicePage = ChoicePage {
    title: "Upscale",
    lead: "Whether to enlarge the finished picture beyond its native size.",
    options: &[
        Opt {
            label: "Keep the native size",
            note: "Recommended. Nothing is invented.",
        },
        Opt {
            label: "Enlarge to roughly 720p",
            note: "For a player or TV that handles SD badly.",
        },
        Opt {
            label: "Enlarge to roughly 1080p",
            note: "Big files, and no more real detail than the line above.",
        },
    ],
    get: |c| {
        if !c.upscale_enabled {
            0
        } else if c
            .upscale_resolution
            .as_deref()
            .is_some_and(|r| r.contains("1440") || r.contains("1920"))
        {
            2
        } else {
            1
        }
    },
    set: |c, i| match i {
        1 => {
            c.upscale_enabled = true;
            c.upscale_resolution = None;
        }
        2 => {
            c.upscale_enabled = true;
            c.upscale_resolution = Some("1440x1080".to_string());
        }
        _ => {
            c.upscale_enabled = false;
            c.upscale_resolution = None;
        }
    },
    details: &[
        "Upscaling is done with nnedi3, a neural-network edge-directed interpolator, rather than a plain resize. It reconstructs edges rather than smearing them, so the result looks sharper than a Spline or Lanczos enlargement of the same material.",
        "It still cannot add detail that was never recorded. A VHS frame carries roughly 240 lines of real horizontal resolution; enlarging it to 1080 makes the file four times the size and the picture no more detailed. What it does buy is a cleaner result on displays that would otherwise do their own, worse, scaling.",
        "If the finished file is going into an editing timeline with HD material, upscaling here is usually better than letting the editor do it. If it is going to a TV or a modern player, leave it native — they scale well enough, and you keep the smaller file.",
    ],
};

// ── Color bleeding ─────────────────────────────────────────────────────────

/// A button that applies the recommendation, next to the slider it moves.
///
/// The advice itself is in the Analysis card above, like every other page.
/// This is only the shortcut, and it exists only when it would change
/// something — a control that does nothing when pressed is worse than no
/// control at all.
fn bleeding_button(ui: &mut egui::Ui, app: &mut App, inner: f32) {
    let Some(b) = app.report.as_ref().and_then(|r| r.bleeding) else {
        return;
    };
    let suggested: i32 = if b.recommendation { 1 } else { 0 };
    if app.config.yc_delay == suggested {
        return;
    }

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        let label = if suggested == 0 {
            "Set it back to zero".to_string()
        } else {
            format!("Set it to {suggested:+}")
        };
        if theme::secondary(ui, &label).clicked() {
            app.config.yc_delay = suggested;
        }
    });
    ui.add_space(18.0);
}

/// The Color bleeding page: a slider and a picture.
///
/// This is the one setting the app will not choose for anyone, in any mode.
/// The probe can see that the chroma and luma edges disagree, but not by how
/// much anyone will find acceptable — so the only honest control is one that
/// shows the result and lets the eye decide. A list of named amounts was the
/// first attempt and it answered the wrong question: nobody knows whether
/// they want "two pixels" until they have seen two pixels.
pub fn bleeding_page(ui: &mut egui::Ui, app: &mut App) -> PageAction {
    let action = PageAction::default();
    ui.set_max_width(COLUMN);
    let inner = COLUMN - 34.0;

    theme::page_header(
        ui,
        "Color bleeding",
        "On tape the color often lands a pixel or two to the side of the thing it belongs to. Slide until the fringes line up.",
    );
    ui.add_space(18.0);

    if let Some(found) = analysis_for(app, Step::ColorBleeding) {
        analysis_card(ui, &found);
        ui.add_space(18.0);
    }

    bleeding_button(ui, app, inner);

    // The picture, at the size the finished file will be.
    let tag = format!("yc{}", app.config.yc_delay);
    app.request_setting_preview(
        tag.clone(),
        PreviewKind::Filter(vcg_tools::preview::chroma_shift_filter(app.config.yc_delay)),
    );

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        preview_picture(ui, app, &tag, inner);

        ui.add_space(14.0);
        let mut value = app.config.yc_delay;
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = inner - 140.0;
            ui.add(
                egui::Slider::new(&mut value, -4..=4)
                    .step_by(1.0)
                    .show_value(false),
            );
            ui.add_space(12.0);
            ui.label(
                RichText::new(match value {
                    0 => "no shift".to_string(),
                    n => format!("{n:+} pixels"),
                })
                .size(theme::FONT_BODY)
                .color(if value == 0 { theme::muted() } else { theme::text() }),
            );
        });
        if value != app.config.yc_delay {
            app.config.yc_delay = value;
        }

        ui.add_space(8.0);
        wrapped(
            ui,
            "Look at a hard vertical edge with strong color — a red jumper against a pale wall. When the color fringe on one side disappears, that is the right amount.",
            theme::FONT_SMALL,
            theme::faint(),
        );
    });

    ui.add_space(16.0);
    let mut open = app.is_open("bleed-details");
    if theme::disclosure(ui, "Details", &mut open) {
        app.set_open("bleed-details", open);
    }
    if open {
        ui.add_space(8.0);
        egui::Frame::NONE
            .fill(theme::bg())
            .corner_radius(theme::RADIUS_SMALL)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(inner - 24.0);
                for para in [
                    "Luma and chroma travel separately through a tape machine and arrive at slightly different times, so the color lands beside the object it belongs to rather than on it. It shows most on hard vertical edges with strong color.",
                    "The correction is a whole-pixel horizontal shift of the chroma planes — whole pixels because that is what the fault is. The delay is in the signal path, not a resampling artefact, so a sub-pixel shift would blur the chroma to fix something that is not blurred.",
                    "The render does it in VapourSynth by shuffling the planes apart and resampling the chroma with src_left. This preview uses FFmpeg's chromashift, which moves chroma the opposite way for the same number, so the preview negates it. Measured on a frame with a hard chroma edge at x=160: src_left=+8 put it at 152, chromashift=+8 put it at 168.",
                    "Nothing here is applied for you in any mode. The probe can see that the chroma and luma edges disagree, but not by how much you will find acceptable.",
                ] {
                    wrapped(ui, para, theme::FONT_SMALL, theme::muted());
                    ui.add_space(8.0);
                }
            });
    }

    action
}

/// The preview frame, or a skeleton of the same shape while it renders.
pub fn preview_picture(ui: &mut egui::Ui, app: &App, tag: &str, width: f32) {
    match app.setting_preview_for(tag) {
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
            let height = (width * 3.0 / 4.0).floor();
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
            if ui.is_rect_visible(rect) {
                ui.painter()
                    .rect_filled(rect, theme::RADIUS_SMALL, theme::panel2());
            }
            ui.ctx().request_repaint();
        }
    }
}

// ── Audio ───────────────────────────────────────────────────────────────────

/// The Audio page.
///
/// One real decision, and only when there is one to make: a tape with sound
/// on a single side. Everything else here is an explanation, because there is
/// nothing else to change and inventing a control to fill the page would be
/// worse than saying so.
pub fn audio_page(ui: &mut egui::Ui, app: &mut App) -> PageAction {
    let action = PageAction::default();
    ui.set_max_width(COLUMN);
    let inner = COLUMN - 34.0;

    theme::page_header(ui, "Audio", "");
    ui.add_space(18.0);

    let verdict = app
        .report
        .as_ref()
        .and_then(|r| r.audio.as_ref())
        .map(|a| a.verdict);

    match verdict {
        Some(AudioVerdict::None) => {
            theme::card().show(ui, |ui| {
                ui.set_width(inner);
                theme::tag(ui, Tag::Info, Some("No sound"));
                ui.add_space(8.0);
                wrapped(ui, "This file has no audio track at all. That is common for a camera-only capture, and the finished video will have none either.", theme::FONT_BODY, theme::muted());
            });
            return action;
        }
        Some(v) if v.can_fix() => {
            theme::card().show(ui, |ui| {
                ui.set_width(inner);
                theme::tag(ui, Tag::FixRecommended, Some("Recommended"));
                ui.add_space(8.0);
                ui.label(
                    RichText::new(match v {
                        AudioVerdict::Mono => "Your sound is a single channel",
                        _ => "Your sound is only on one side",
                    })
                    .size(theme::FONT_SECTION)
                    .color(theme::text()),
                );
                ui.add_space(8.0);
                wrapped(ui, match v {
                    AudioVerdict::Mono => "The file carries one channel rather than two. Most players center it, but some put it on the left only. Copying it to both sides removes the doubt.",
                    _ => "One of the two channels is empty, so the sound comes out of one speaker. That is usually a deck with a single channel recorded, or a capture cable with a dead core. Copying the live side to both fixes it, and nothing you can hear is lost.",
                }, theme::FONT_BODY, theme::muted());

                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let mut on = app.config.fix_one_sided_audio;
                    if theme::toggle(ui, &mut on, false) {
                        app.config.fix_one_sided_audio = on;
                    }
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(if app.config.fix_one_sided_audio {
                            "Copying it to both sides."
                        } else {
                            "Leaving it as it is."
                        })
                        .size(theme::FONT_SMALL)
                        .color(if app.config.fix_one_sided_audio {
                            theme::ok()
                        } else {
                            theme::faint()
                        }),
                    );
                });
            });
            ui.add_space(18.0);
        }
        Some(_) => {
            theme::card().show(ui, |ui| {
                ui.set_width(inner);
                theme::tag(ui, Tag::LooksGood, Some("Nothing to do"));
                ui.add_space(8.0);
                wrapped(ui, "Both sides carry sound. Many tape captures come out as the same signal on both channels, which is exactly right for a mono recording and needs no fixing.", theme::FONT_BODY, theme::muted());
            });
            ui.add_space(18.0);
        }
        None => {}
    }

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        for para in [
            "Apart from that, your sound is carried through untouched. It is taken off the source uncompressed and put back without being re-encoded, except for the sharing format, which needs AAC to be a normal MP4.",
            "Nothing is filtered, levelled or resampled. A tape's audio is usually the least damaged part of it, and anything done here would have to be undone by whoever works on the result.",
            "If you trimmed the video, the sound is cut to exactly the same ranges, so the two cannot drift apart.",
        ] {
            wrapped(ui, para, theme::FONT_BODY, theme::muted());
            ui.add_space(9.0);
        }
    });

    ui.add_space(16.0);
    let mut open = app.is_open("audio-details");
    if theme::disclosure(ui, "Details", &mut open) {
        app.set_open("audio-details", open);
    }
    if open {
        ui.add_space(8.0);
        egui::Frame::NONE
            .fill(theme::bg())
            .corner_radius(theme::RADIUS_SMALL)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(inner - 24.0);
                if let Some(a) = app.report.as_ref().and_then(|r| r.audio.as_ref()) {
                    let levels = a
                        .levels
                        .iter()
                        .enumerate()
                        .map(|(i, l)| match l.rms_db {
                            Some(db) => format!("channel {}: {db:.1} dB", i + 1),
                            None => format!("channel {}: silent", i + 1),
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    wrapped(
                        ui,
                        &format!(
                            "Measured over the first 60 seconds with astats: {} channel(s), {levels}.",
                            a.channels
                        ),
                        theme::FONT_SMALL,
                        theme::muted(),
                    );
                    ui.add_space(8.0);
                }
                for para in [
                    "A side counts as missing when it is digitally silent, or more than 40 dB below the other. A real stereo mix can be 15 to 20 dB lopsided with someone speaking off to one side, and treating that as a fault would throw the other channel away.",
                    "The repair is a pan filter applied while the audio is extracted, taking the live channel to both sides. It joins the trim graph when there is one, because -af and -filter_complex cannot both apply to the same stream — adding it beside the trim made FFmpeg fail outright and the export carried on and produced a file with no sound.",
                    "Extraction is a decode to 16-bit PCM, not a stream copy: the source codec may not be legal in the output container, and a tape's audio is small next to its video.",
                    "ProRes and FFV1 keep that PCM. Without an explicit codec the container default takes over, and .mov re-encoded to 129 kbps AAC — lossy sound inside a master nobody asked to be lossy.",
                    "-shortest is passed as a backstop. A container is as long as its longest stream, so a mismatch between picture and sound would silently pad the file rather than fail.",
                ] {
                    wrapped(ui, para, theme::FONT_SMALL, theme::muted());
                    ui.add_space(8.0);
                }
            });
    }

    action
}

/// A paragraph that wraps. Stated rather than inherited from the layout,
/// which is how the Export details ended up running off the right of the
/// window.
fn wrapped(ui: &mut egui::Ui, text: &str, size: f32, color: egui::Color32) {
    theme::paragraph(ui, RichText::new(text).size(size).color(color));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact contradiction a user reported: the graph said everything
    /// was inside the legal range while the card beside it said the picture
    /// ran from 10 to 229. The sample really does dip to 9 — ffmpeg's
    /// signalstats over the whole file reports YMIN=9, YMAX=229 — so the
    /// graph was the one that was wrong, hiding it behind a 0.05% tolerance.
    #[test]
    fn the_caption_never_calls_an_out_of_range_picture_fine() {
        // The measured sample: 0.02% of pixels below the floor.
        let caption = levels_caption(9, 229, 0.0002, 0.0);
        assert!(caption.contains("9"), "{caption}");
        assert!(caption.contains("below the legal floor"), "{caption}");
        assert!(
            !caption.contains("Everything sits inside"),
            "a picture reaching 9 is not inside the legal range: {caption}"
        );
    }

    #[test]
    fn the_caption_says_so_when_the_picture_really_is_legal() {
        let caption = levels_caption(16, 235, 0.0, 0.0);
        assert!(caption.contains("Everything sits inside"), "{caption}");
    }

    #[test]
    fn the_caption_reports_both_ends_when_both_are_out() {
        let caption = levels_caption(0, 255, 0.05, 0.07);
        assert!(caption.contains("below the legal floor"), "{caption}");
        assert!(caption.contains("above the legal ceiling"), "{caption}");
    }

    /// A couple of stray pixels should be described as such rather than as
    /// "0.00% of the picture", which reads as nothing at all.
    #[test]
    fn a_tiny_share_is_worded_not_rounded_to_zero() {
        assert_eq!(share(0.00001), "a handful of pixels");
        assert_eq!(share(0.0002), "0.02% of the picture");
        assert_eq!(share(0.071), "7.1% of the picture");
    }

    /// Every page must round-trip: select option i, read it back as i. A
    /// getter and setter that disagree would silently move the user's choice.
    #[test]
    fn every_option_reads_back_as_itself() {
        for step in Step::SETTINGS {
            let Some(page) = page_for(step) else { continue };
            for i in 0..page.options.len() {
                let mut c = Config::default();
                (page.set)(&mut c, i);
                assert_eq!(
                    (page.get)(&c),
                    i,
                    "{}: option {i} ({}) did not read back",
                    page.title,
                    page.options[i].label
                );
            }
        }
    }

    /// The default config must land on a real option, or the page opens with
    /// nothing selected.
    #[test]
    fn the_default_config_selects_something() {
        for step in Step::SETTINGS {
            let Some(page) = page_for(step) else { continue };
            let i = (page.get)(&Config::default());
            assert!(
                i < page.options.len(),
                "{}: default selects {i} of {}",
                page.title,
                page.options.len()
            );
        }
    }

    /// A fresh config must select the "do nothing" option everywhere it has
    /// one, since that is the promise the modes are built on.
    #[test]
    fn a_fresh_config_changes_nothing() {
        for (step, expect) in [
            (Step::Levels, 0),
            (Step::ColorTint, 0),
            (Step::Noise, 0),
            (Step::Halos, 0),
            (Step::FilmGrain, 0),
            (Step::Upscale, 0),
            // Dithering is on by default, and should be: it is part of
            // producing a correct file, not an enhancement.
            (Step::Dithering, 0),
        ] {
            let page = page_for(step).expect("a page");
            assert_eq!((page.get)(&Config::default()), expect, "{}", page.title);
        }
    }

    /// Nothing these pages can set may count as an enhancement in
    /// `JustDeinterlace` — except that they are not reachable there at all.
    /// This checks the other half: every option except the first does change
    /// something, so no page offers a choice that does nothing.
    #[test]
    fn every_other_option_actually_changes_the_config() {
        for step in Step::SETTINGS {
            let Some(page) = page_for(step) else { continue };
            let base = Config::default();
            for i in 1..page.options.len() {
                let mut c = base.clone();
                (page.set)(&mut c, i);
                assert_ne!(
                    (page.get)(&c),
                    (page.get)(&base),
                    "{}: option {i} ({}) left the config where it was",
                    page.title,
                    page.options[i].label
                );
            }
        }
    }

    #[test]
    fn every_page_says_something_in_its_details() {
        for step in Step::SETTINGS {
            let Some(page) = page_for(step) else { continue };
            assert!(!page.details.is_empty(), "{}", page.title);
            assert!(!page.lead.is_empty(), "{}", page.title);
            for opt in page.options {
                assert!(!opt.label.is_empty(), "{}", page.title);
            }
        }
    }
}

#[cfg(test)]
mod routing {
    use super::*;

    /// Every step must lead somewhere real.
    ///
    /// The shell falls through to a "not built yet" placeholder for anything
    /// unrouted, which is the right behavior but a silent one — adding a
    /// step and forgetting its page would look finished and be empty. This
    /// lists what is routed outside this module, so the two cannot drift.
    #[test]
    fn no_step_falls_through_to_the_placeholder() {
        const ROUTED_ELSEWHERE: [Step; 9] = [
            Step::Welcome,
            Step::ChooseVideo,
            Step::CheckVideo,
            Step::Crop,
            Step::Trim,
            Step::Audio,
            // Its own page: a slider and a live picture, because the right
            // amount is a visual judgement rather than one of four names.
            Step::ColorBleeding,
            Step::Watermark,
            Step::Export,
        ];

        let missing: Vec<Step> = Step::ALL
            .iter()
            .copied()
            .filter(|s| page_for(*s).is_none() && !ROUTED_ELSEWHERE.contains(s))
            .collect();

        assert!(
            missing.is_empty(),
            "these steps would render the placeholder: {missing:?}"
        );
    }
}

#[cfg(test)]
mod comparison {
    use super::*;

    /// Smooth gradient fix is the page where "option zero does nothing" is
    /// false: its default is the correction and the do-nothing choice is
    /// last. Treating zero as neutral there showed a single large still on
    /// the default and only produced a comparison once the user picked
    /// something else, which is backwards.
    #[test]
    fn the_do_nothing_option_is_not_always_the_first_one() {
        assert!(is_no_op(Step::Levels, 0));
        assert!(!is_no_op(Step::Levels, 1));

        assert!(!is_no_op(Step::Dithering, 0), "error diffusion does something");
        assert!(!is_no_op(Step::Dithering, 1), "ordered does something");
        assert!(is_no_op(Step::Dithering, 2), "no dithering is the no-op");
    }

    /// The neutral index must match the page's own table, or the comparison
    /// and the options disagree about what "nothing" is.
    #[test]
    fn the_no_op_option_leaves_a_default_config_alone() {
        for step in Step::SETTINGS {
            let Some(page) = page_for(step) else { continue };
            let neutral = (0..page.options.len())
                .find(|&i| is_no_op(step, i))
                .unwrap_or_else(|| panic!("{}: no option does nothing", page.title));

            let mut c = Config::default();
            (page.set)(&mut c, neutral);
            let mut cleared = Config::default();
            (page.set)(&mut cleared, neutral);
            assert_eq!(
                (page.get)(&c),
                (page.get)(&cleared),
                "{}: the no-op option is not stable",
                page.title
            );
            assert_eq!(
                (page.get)(&c),
                neutral,
                "{}: is_no_op points at option {neutral}, which does not read back",
                page.title
            );
        }
    }
}
