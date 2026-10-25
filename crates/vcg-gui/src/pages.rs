//! The wizard pages that exist so far: Welcome, Choose video, Check video.
//!
//! Everything else renders a placeholder, so the shape of the wizard can be
//! judged before all sixteen pages are written.

use eframe::egui::{self, Color32, RichText};

use vcg_core::config::{Mode, VideoFormat};

use crate::findings::Check;
use crate::nav::Step;
use crate::theme::{self, icon};
use crate::App;

/// What the page wants the shell to do next.
#[derive(Default)]
pub struct PageAction {
    pub go_to: Option<Step>,
    /// Start analyzing the selected file.
    pub start_probe: bool,
    /// Start writing the finished video.
    pub start_export: bool,
    /// Put the current settings on the export queue instead of starting now.
    pub add_to_queue: bool,
    /// Clear the wizard and start on another video, leaving any running
    /// export alone.
    pub another_video: bool,
    pub set_mode: Option<Mode>,
    /// A fix toggle was flipped: (check, on).
    pub set_fix: Option<(Check, bool)>,
}

// ── Welcome ─────────────────────────────────────────────────────────────────

pub fn welcome(ui: &mut egui::Ui, app: &mut App) -> PageAction {
    let mut action = PageAction::default();

    ui.add_space(28.0);
    ui.vertical_centered(|ui| {
        // The logo above the name, at a size that reads as a mark rather
        // than as a picture of one. Omitted entirely if it did not decode:
        // a placeholder box on the front page is worse than no logo.
        if let Some(logo) = app.logo() {
            ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(96.0, 96.0)));
            ui.add_space(14.0);
        }
        ui.label(
            RichText::new("VCG Deinterlacer")
                .size(theme::FONT_HERO)
                .family(theme::semibold())
                .color(theme::text()),
        );
        ui.add_space(6.0);
        // The byline, with the site behind it. Under the title rather than
        // in the footer because it is whose tool this is, which is part of
        // the name rather than an afterthought.
        ui.horizontal(|ui| {
            // `vertical_centered` lays its children out centred, but a
            // horizontal strip inside it is laid out from the left, so the
            // pair has to be centred by hand.
            let w = 230.0;
            ui.add_space((ui.available_width() - w).max(0.0) / 2.0);
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.label(
                RichText::new("By")
                    .size(theme::FONT_BODY)
                    .color(theme::muted()),
            );
            if theme::link(ui, vcg_core::AUTHOR_NAME).clicked() {
                theme::open_url(vcg_core::AUTHOR_URL);
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("Version {}", crate::VERSION))
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
        );
        ui.add_space(22.0);
        ui.label(
            RichText::new(
                "One-step deinterlacing (and more) of your analog video captures.",
            )
            .size(theme::FONT_LEAD)
            .color(theme::muted()),
        );
        ui.add_space(20.0);

        // What the tool is for, as a picture. It says in five steps what the
        // sentence above says in one line, and it is the fastest way for
        // somebody who has never deinterlaced anything to see where this sits
        // in their workflow.
        //
        // Drawn on a white plate in both palettes, because the artwork has a
        // white background of its own: floated directly on a dark page it
        // reads as a rendering fault rather than as a diagram. A plate makes
        // it deliberate — a printed card lying on the page.
        if let Some(graphic) = app.front_graphic(&ui.ctx().clone()) {
            let w = 620.0_f32.min(ui.available_width() - 40.0);
            egui::Frame::NONE
                .fill(Color32::WHITE)
                .corner_radius(theme::RADIUS)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.add(
                        egui::Image::new(&graphic)
                            .fit_to_exact_size(egui::vec2(w, w * 0.2)),
                    );
                });
            ui.add_space(26.0);
        }

        if theme::vcr_start(ui, app.deps_ready).clicked() {
            action.go_to = Some(Step::ChooseVideo);
        }

        // The components state is on its own page under About now. Only the
        // bad news surfaces here, because that is the one case where it stops
        // the user doing anything and so has to be said where they are.
        if !app.deps_ready {
            ui.add_space(14.0);
            first_run_setup(ui, app);
        }
    });

    action
}

/// The first-run payload download, on the page the user is already looking at.
///
/// Not a modal dialog. The Python app put this in one that appeared only when
/// something was already broken, which meant the usual first run showed a
/// window that silently could not work. Here the Start button is already
/// disabled until the payload is ready, so this card explains the thing the
/// user can already see.
fn first_run_setup(ui: &mut egui::Ui, app: &mut App) {
    use crate::setup::Status;

    let status = app
        .deps_install
        .as_ref()
        .map(|i| i.status.clone())
        .unwrap_or(Status::Idle);

    match status {
        Status::Working { note, fraction } => {
            ui.label(
                RichText::new("Getting the components this needs")
                    .size(theme::FONT_SMALL)
                    .family(theme::semibold())
                    .color(theme::text()),
            );
            ui.add_space(6.0);
            // An unknown fraction animates rather than sitting at zero, which
            // would read as a download that has stalled.
            let shown = fraction.unwrap_or_else(|| {
                let t = ui.input(|i| i.time) as f32;
                (t * 0.5).fract()
            });
            app.progress_bar(ui, shown, 360.0, 10.0);
            ui.add_space(6.0);
            ui.label(
                RichText::new(note)
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
            );
            ui.add_space(2.0);
            ui.label(
                RichText::new(
                    "FFmpeg and VapourSynth, about 158 MiB. This happens once.",
                )
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
            );
            // Without this the window only redraws on input, so the bar would
            // freeze while the download ran.
            ui.ctx().request_repaint();
        }
        Status::Failed(message) => {
            ui.label(
                RichText::new("The components could not be downloaded")
                    .size(theme::FONT_SMALL)
                    .family(theme::semibold())
                    .color(theme::fix()),
            );
            ui.add_space(4.0);
            theme::paragraph(
                ui,
                RichText::new(message)
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
            );
            ui.add_space(8.0);
            if theme::primary(ui, "Try again", true).clicked() {
                app.start_payload_install();
            }
        }
        // Done but still not ready means the payload installed and the tools
        // are still not usable — a different problem, and the honest thing to
        // show is what the check actually says.
        Status::Idle | Status::Done => {
            ui.label(
                RichText::new(&app.deps_summary)
                    .size(theme::FONT_SMALL)
                    .color(theme::fix()),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("About & updates → Components has the detail.")
                    .size(theme::FONT_SMALL)
                    .color(theme::faint()),
            );
            ui.add_space(8.0);
            if theme::primary(ui, "Download the components", true).clicked() {
                app.start_payload_install();
            }
        }
    }
}

// ── Screen 1: Choose video ──────────────────────────────────────────────────

pub fn choose_video(ui: &mut egui::Ui, app: &mut App) -> PageAction {
    let mut action = PageAction::default();

    theme::page_header(
        ui,
        "Select a video",
        "Drop a video here or click Browse.",
    );

    if app.file.is_some() {
        file_card(ui, app);
    } else {
        drop_zone(ui, app);
    }

    ui.add_space(26.0);
    theme::section_header(ui, "Enhancement options");
    // Room for the "Recommended" badge, which straddles the top edge of its
    // card and is therefore drawn above the row. Without the gap it is
    // clipped in half by whatever sits above it.
    ui.add_space(14.0 + BADGE_OVERHANG);

    // Wider than the global 8, which is a gap between controls rather than
    // between three things being compared. `columns` divides the row by this
    // value, so setting it here sets every gap in the row at once and the
    // three cards cannot end up unevenly spaced.
    ui.spacing_mut().item_spacing.x = 16.0;
    ui.columns(3, |cols| {
        for (i, mode) in Mode::ALL.into_iter().enumerate() {
            if mode_card(&mut cols[i], mode, app.mode) {
                action.set_mode = Some(mode);
            }
        }
    });

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        theme::inline_icon(ui, 14.0, theme::faint(), icon::info);
        ui.label(
            RichText::new(
                "The video is checked automatically when you continue — it takes a few seconds.",
            )
            .size(theme::FONT_SMALL)
            .color(theme::faint()),
        );
    });

    // Full control only, and here rather than on the Export page where it
    // used to live.
    //
    // A preset replaces every setting on every page, so offering it last is
    // offering it after the reader has already made each of those choices by
    // hand — the one point in the wizard where loading one undoes work rather
    // than saving it. The point of keeping a settings file is the next tape
    // off the same deck, and that pays off at the start.
    //
    // The other two modes do not show it because they have almost nothing for
    // it to set: Just deinterlace clears every optional filter on principle,
    // and auto-fix decides from its own measurements of the tape in hand.
    if app.mode == Mode::EnhanceManually {
        ui.add_space(20.0);
        load_settings_card(ui, app);
    }

    action
}

/// Loading a saved settings file, at the point in the wizard where it helps.
fn load_settings_card(ui: &mut egui::Ui, app: &mut App) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new("Already have a settings file?")
                .size(theme::FONT_BODY)
                .color(theme::text()),
        );
        ui.add_space(8.0);
        if theme::secondary(ui, "Load settings from a JSON file…").clicked() {
            app.queue_dialog(crate::dialog::Dialog::LoadSettings);
        }
        ui.add_space(8.0);
        theme::paragraph(ui, RichText::new(
                    "Replaces every setting at once, on every page. What is measured \
                     from the file you are working on — the field order above all — \
                     wins over what the preset says, because that describes the tape \
                     in hand rather than the one the preset came from.",
                )
                .size(theme::FONT_SMALL)
                .color(theme::muted()),);
        if let Some(result) = &app.settings_loaded {
            ui.add_space(8.0);
            let (text, color) = match result {
                Ok(message) => (message.clone(), theme::ok()),
                Err(why) => (format!("Nothing was changed — {why}"), theme::fix()),
            };
            theme::paragraph(ui, RichText::new(text).size(theme::FONT_SMALL).color(color));
        }
    });
}

/// The empty state: a dashed target with Browse inside it.
fn drop_zone(ui: &mut egui::Ui, app: &mut App) {
    let hovering = !ui.ctx().input(|i| i.raw.hovered_files.is_empty());
    let width = ui.available_width();

    let response = egui::Frame::NONE
        .fill(theme::panel())
        .corner_radius(theme::RADIUS + 2.0)
        .inner_margin(egui::Margin::same(32))
        .show(ui, |ui| {
            ui.set_width(width - 64.0);
            ui.vertical_centered(|ui| {
                theme::inline_icon(
                    ui,
                    34.0,
                    if hovering { theme::accent() } else { theme::faint() },
                    |p, r, c| icon::arrow(p, r, c, icon::Dir::Down),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(if hovering {
                        "Release to load"
                    } else {
                        "Drop a video file here"
                    })
                    .size(theme::FONT_LEAD)
                    .color(theme::muted()),
                );
                ui.add_space(12.0);
                if theme::primary(ui, "Browse…", true).clicked() {
                    browse(app);
                }
                ui.add_space(12.0);
                // What a tape capture actually arrives as. MPG belongs here —
                // DVD recorders and standalone capture boxes write MPEG-2, and
                // it was missing. MP4 and MOV are gone: they are delivery
                // formats, and they are this app's *own* output, so listing
                // them invites someone to feed a finished export back in and
                // deinterlace it twice.
                //
                // This is a hint, not a gate. `dialog::VIDEO_EXTENSIONS` still
                // offers mp4 and mov in the picker, and a dragged file is not
                // filtered at all — somebody with a genuinely interlaced MP4
                // off a DVR must still be able to open it.
                ui.label(
                    RichText::new("AVI · MPG · MKV · MTS · DV · VOB")
                        .size(theme::FONT_SMALL)
                        .color(theme::faint()),
                );
            });
        })
        .response;

    // A dashed border, painted over the frame: egui strokes are solid.
    if ui.is_rect_visible(response.rect) {
        dashed_rect(
            ui,
            response.rect,
            if hovering { theme::accent() } else { theme::line() },
            2.0,
        );
    }
}

/// A dashed rounded rectangle, drawn as segments along each side.
fn dashed_rect(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32, width: f32) {
    let p = ui.painter();
    let stroke = egui::Stroke::new(width, color);
    let (dash, gap) = (9.0, 6.0);
    let inset = 1.0;
    let r = rect.shrink(inset);

    let run = |from: egui::Pos2, to: egui::Pos2| {
        let total = (to - from).length();
        let dir = (to - from).normalized();
        let mut t = 0.0;
        while t < total {
            let end = (t + dash).min(total);
            p.line_segment([from + dir * t, from + dir * end], stroke);
            t = end + gap;
        }
    };
    // Corners are left square; at this radius the gap is not noticeable and it
    // avoids approximating arcs with dashes.
    let c = theme::RADIUS;
    run(
        egui::pos2(r.left() + c, r.top()),
        egui::pos2(r.right() - c, r.top()),
    );
    run(
        egui::pos2(r.right() - c, r.bottom()),
        egui::pos2(r.left() + c, r.bottom()),
    );
    run(
        egui::pos2(r.left(), r.top() + c),
        egui::pos2(r.left(), r.bottom() - c),
    );
    run(
        egui::pos2(r.right(), r.top() + c),
        egui::pos2(r.right(), r.bottom() - c),
    );
}

/// The loaded state: thumbnail, name, path, metadata chips, and the actions.
fn file_card(ui: &mut egui::Ui, app: &mut App) {
    let Some(file) = app.file.clone() else { return };
    let mut browse_clicked = false;
    let mut remove_clicked = false;

    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;

            // The thumbnail column is omitted entirely when there is no frame,
            // rather than showing a placeholder box.
            if let Some(texture) = &app.thumbnail {
                let size = texture.size_vec2();
                let scale = 160.0 / size.x;
                ui.add(
                    egui::Image::new(texture)
                        .fit_to_exact_size(size * scale)
                        .corner_radius(theme::RADIUS_SMALL),
                );
            }

            // The middle column is given an explicit width rather than being
            // allowed to take what is left. A long path is one label that
            // claims the whole row, and the action column — which is laid out
            // last — then gets whatever remains, which on a deep path is a few
            // pixels: "Change file…" was reported stacked one letter per line
            // against the right edge. Reserving the actions first means the
            // name and the path truncate instead, which is what truncation is
            // for.
            const ACTIONS_WIDTH: f32 = 150.0;
            let middle = (ui.available_width() - ACTIONS_WIDTH - 16.0).max(160.0);

            ui.vertical(|ui| {
                ui.set_max_width(middle);
                let name = file
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                ui.add(
                    egui::Label::new(
                        RichText::new(&name).size(theme::FONT_SECTION).color(theme::text()),
                    )
                    .truncate(),
                )
                .on_hover_text(&name);
                ui.add_space(2.0);

                // One line, truncated, with the whole path on hover.
                let path = file.display().to_string();
                ui.add(
                    egui::Label::new(
                        RichText::new(&path).size(theme::FONT_SMALL).color(theme::faint()),
                    )
                    .truncate(),
                )
                .on_hover_text(&path);

                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    for text in file_chips(app, &file) {
                        theme::chip(ui, &text);
                    }
                });
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.vertical(|ui| {
                    if theme::secondary(ui, "Change file…").clicked() {
                        browse_clicked = true;
                    }
                    ui.add_space(4.0);
                    if theme::link(ui, "Remove").clicked() {
                        remove_clicked = true;
                    }
                });
            });
        });
    });

    if browse_clicked {
        browse(app);
    }
    if remove_clicked {
        app.clear_file();
    }
}

/// Size, duration, resolution and scan type, from the quick container read.
fn file_chips(app: &App, file: &std::path::Path) -> Vec<String> {
    let mut chips = Vec::new();

    if let Ok(meta) = std::fs::metadata(file) {
        let mb = meta.len() as f64 / 1_048_576.0;
        chips.push(if mb >= 1024.0 {
            format!("{:.1} GB", mb / 1024.0)
        } else {
            format!("{mb:.1} MB")
        });
    }

    if let Some(report) = &app.file_info {
        if let Some(i) = &report.info {
            chips.push(format_duration(i.duration));
            let standard = report
                .par
                .as_ref()
                .and_then(|p| p.format)
                .map(|f| match f {
                    VideoFormat::Ntsc => " NTSC",
                    VideoFormat::Pal => " PAL",
                })
                .unwrap_or("");
            chips.push(format!("{}×{}{standard}", i.width, i.height));
        }
        if let Some(c) = &report.classification {
            use vcg_core::analysis::probe::DetectedFieldOrder as F;
            chips.push(
                match c.field_order {
                    F::Progressive => "Progressive",
                    // The container usually does not say; the real answer comes
                    // from the Check video step.
                    F::Unknown => "Interlaced (assumed)",
                    _ => "Interlaced",
                }
                .to_string(),
            );
            if let Some(order) = field_order_chip(app, c.field_order) {
                chips.push(order);
            }
        }
    }

    chips
}

/// The field order, beside "Interlaced", in the three letters a user who
/// knows their own deck will recognise.
///
/// It is the single most consequential fact about an interlaced file — fed
/// the wrong dominance, QTGMC interpolates each field against the neighbor
/// on the wrong side of it in time — and most tape captures declare nothing,
/// so what the app decided is not guessable from anywhere else.
///
/// Three states, and the difference between them matters. The container's
/// own declaration is used when it makes one. Once the check has run, the
/// measured answer is better than the declaration would have been, and is
/// marked as measured. Before either, the chip says the question is still
/// open rather than naming a default that may not survive the measurement.
fn field_order_chip(
    app: &App,
    declared: vcg_core::analysis::probe::DetectedFieldOrder,
) -> Option<String> {
    use vcg_core::analysis::probe::DetectedFieldOrder as F;
    use vcg_core::config::FieldOrder;

    match declared {
        F::Progressive => None,
        F::Tff => Some("TFF".to_string()),
        F::Bff => Some("BFF".to_string()),
        F::Unknown => match app.report.as_ref().map(|_| app.config.field_order) {
            Some(FieldOrder::Tff) => Some("TFF (measured)".to_string()),
            Some(FieldOrder::Bff) => Some("BFF (measured)".to_string()),
            Some(FieldOrder::Progressive) => None,
            None => Some("Field order not declared".to_string()),
        },
    }
}

/// Ask the shell to open the file picker after this frame.
///
/// Not opened here: a native dialog with no owner window can come up behind
/// the app, which blocks the whole window with nothing on screen explaining
/// why. See `dialog`.
fn browse(app: &App) {
    app.queue_dialog(crate::dialog::Dialog::Source);
}

/// One mode card. Returns true when clicked.
fn mode_card(ui: &mut egui::Ui, mode: Mode, selected: Mode) -> bool {
    let is_selected = mode == selected;
    let recommended = theme::RECOMMEND_AUTO_FIX && mode == Mode::EnhanceAutomatically;

    let (fill, stroke) = if is_selected {
        (theme::accent_soft(), egui::Stroke::new(1.5, theme::accent()))
    } else {
        (theme::panel(), egui::Stroke::new(1.0, theme::line()))
    };

    let inner = egui::Frame::NONE
        .fill(fill)
        .stroke(stroke)
        .corner_radius(theme::RADIUS)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 9.0;
                    radio_dot(ui, is_selected);
                    ui.label(
                        RichText::new(mode.short_title())
                            .size(theme::FONT_LEAD)
                            .color(if is_selected { theme::text() } else { theme::muted() }),
                    );
                });
                ui.add_space(9.0);
                ui.label(
                    RichText::new(mode.lead())
                        .size(theme::FONT_SMALL)
                        .color(theme::text()),
                );
                ui.add_space(10.0);

                for (positive, text) in mode.includes() {
                    // Top-aligned and wrapped. A plain label in a horizontal
                    // layout does not wrap: the longest bullet ran straight
                    // out of its card and under the next one, which read as
                    // two cards with no gap between them and a sentence
                    // chopped off mid-word.
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 7.0;
                        if *positive {
                            theme::inline_icon(ui, 13.0, theme::ok(), icon::check);
                        } else {
                            theme::inline_icon(ui, 13.0, theme::faint(), icon::dash);
                        }
                        theme::paragraph(
                            ui,
                            RichText::new(*text)
                                .size(theme::FONT_SMALL)
                                .color(theme::muted()),
                        );
                    });
                    ui.add_space(2.0);
                }

                // The footer sits at the bottom so it lines up across cards
                // whose bullet lists differ in length.
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(mode.footer())
                            .size(theme::FONT_TINY)
                            .color(theme::faint()),
                    );
                    ui.add_space(8.0);
                    let w = ui.available_width();
                    let (rule, _) =
                        ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
                    if ui.is_rect_visible(rule) {
                        ui.painter().rect_filled(rule, 0.0, theme::line());
                    }
                });
            });
        });

    let response = inner.response.interact(egui::Sense::click());

    if recommended && ui.is_rect_visible(response.rect) {
        badge(ui, response.rect, "Recommended");
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    theme::focus_ring(ui, &response, theme::RADIUS);
    response.clicked()
}

/// How far a badge rises above the card it sits on. The row above has to
/// leave this much clear or the badge is drawn into it and clipped.
const BADGE_OVERHANG: f32 = 11.0;

/// A small label straddling the top border of a card.
///
/// Centered, not tucked into a corner: at the right edge it sat in the seam
/// between two cards, and the neighboring card — drawn afterwards — painted
/// over half of it.
fn badge(ui: &egui::Ui, card: egui::Rect, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(theme::FONT_TINY),
        theme::ok(),
    );
    let size = galley.size() + egui::vec2(16.0, 4.0);
    let rect = egui::Rect::from_min_size(
        egui::pos2(card.center().x - size.x / 2.0, card.top() - size.y / 2.0),
        size,
    );
    let p = ui.painter();
    p.rect_filled(rect, size.y / 2.0, theme::ok_soft());
    p.rect_stroke(
        rect,
        size.y / 2.0,
        egui::Stroke::new(1.0, theme::ok()),
        egui::StrokeKind::Inside,
    );
    p.galley(rect.center() - galley.size() / 2.0, galley, theme::ok());
}

fn radio_dot(ui: &mut egui::Ui, selected: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if selected {
            p.circle_filled(rect.center(), 9.0, theme::accent());
            p.circle_filled(rect.center(), 3.5, theme::accent_soft());
        } else {
            p.circle_stroke(
                rect.center(),
                8.0,
                egui::Stroke::new(2.0, theme::faint()),
            );
        }
    }
}

/// A duration in words rather than digits.
pub fn format_duration(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m} min {s:02} sec")
    } else {
        format!("{s} sec")
    }
}

// ── Placeholder ─────────────────────────────────────────────────────────────

pub fn placeholder(ui: &mut egui::Ui, step: Step) -> PageAction {
    theme::page_header(ui, step.title(), "This page is not built yet.");
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(
                "The engine behind this page is ported and verified — only the \
                 interface is outstanding.",
            )
            .size(theme::FONT_BODY)
            .color(theme::muted()),
        );
    });
    PageAction::default()
}
