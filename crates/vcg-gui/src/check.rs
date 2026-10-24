//! The Check video screen.
//!
//! Written for someone who has never heard of interlacing. The shape is the
//! same in every mode — a numbered run of sections, each answering one
//! question — but how much is actionable depends on the mode:
//!
//! * **Just deinterlace**: one hero card saying what will happen, then
//!   observations it will not act on, with a button to switch modes.
//! * **Auto-fix / Full control**: the same hero card, then the recommended
//!   fixes with Apply toggles, then the checks that passed.
//!
//! Every number lives behind a disclosure or the technical-details checkbox.
//! Nothing on the face of the page names a measurement.

use eframe::egui::{self, RichText};

use vcg_core::config::{Mode, QtgmcImpl, VideoFormat};
use vcg_tools::probe::Report;

use crate::findings::{self, Check, Finding};
use crate::pages::PageAction;
use crate::theme::{self, icon, Tag};
use crate::App;

/// The content column. Wide enough for the before/after pair side by side,
/// narrow enough that body text does not run to unreadable line lengths.
pub const COLUMN: f32 = 860.0;

pub fn check_video(ui: &mut egui::Ui, app: &mut App) -> PageAction {
    let mut action = PageAction::default();

    // Every card is laid out to this width, so the column cannot change shape
    // when a disclosure opens. The earlier version let an opened panel push
    // its own card wider than the ones above it.
    ui.set_max_width(COLUMN);

    if let Some(job) = &app.job
        && !job.finished {
            let (fraction, label) = (job.fraction(), job.label());
            scanning(ui, app, fraction, &label);
            ui.ctx().request_repaint();
            return action;
        }

    let Some(report) = app.report.clone() else {
        theme::page_header(ui, "Check your video", "");
        theme::card().show(ui, |ui| {
            ui.set_width(COLUMN - 34.0);
            ui.label(
                RichText::new("Not checked yet.")
                    .size(theme::FONT_BODY)
                    .color(theme::muted()),
            );
        });
        ui.add_space(16.0);
        if theme::primary(ui, "Check video", true).clicked() {
            action.start_probe = true;
        }
        return action;
    };

    let all = findings::collect(&report);

    header(ui, app);
    ui.add_space(22.0);

    // ── ① About this file ───────────────────────────────────────────────────
    //
    // First, and open. What the file *is* answers the question a reader
    // arrives with — is this the tape I think it is, and did the app read it
    // correctly — and it has to be answered before anything we propose to do
    // to it means much. It used to sit last behind a disclosure, which put
    // the one section that is pure fact below three that ask for decisions.
    section_number(ui, 1, "About this file");
    ui.add_space(10.0);
    about_file(ui, app, &report);
    ui.add_space(22.0);

    // ── ② What we'll do ─────────────────────────────────────────────────────
    let interlacing = all.iter().find(|f| f.check == Check::Interlacing);
    section_number(ui, 2, "What we'll do");
    ui.add_space(10.0);
    if let Some(f) = interlacing.cloned() {
        hero_card(ui, app, &f);
    }
    ui.add_space(22.0);

    // Full control asks about every one of these on its own page, with the
    // measurement, a recommendation and a before/after beside it. Listing
    // them here as well made the reader decide twice — once from a one-line
    // summary with no picture, and again properly a few screens later. The
    // first of those decisions could only ever be worse informed, so it goes.
    //
    // Auto-fix keeps them: in that mode this *is* the approval step, and
    // there is no later page to defer to.
    let lists_fixes = app.mode == Mode::EnhanceAutomatically;
    if lists_fixes {
        fixes_and_passes(ui, app, &all, &report, &mut action);
        ui.add_space(22.0);
    } else if app.mode == Mode::JustDeinterlace {
        // The sound repair happens in this mode too — it is structural, like
        // the crop — so it gets its own section rather than being folded into
        // "We also noticed", which is the list of things this mode will
        // deliberately leave alone. Numbering runs on from whichever of the
        // two actually appears.
        let mut number = 3;
        if sound_section(ui, app, &all, number) {
            ui.add_space(22.0);
            number += 1;
        }
        also_noticed(ui, app, &all, &report, &mut action, number);
    } else {
        // Full control used to end with a "What comes next" card here. It
        // described the settings pages the Next button leads straight to,
        // which is a paragraph explaining a thing the user is one click from
        // seeing for themselves.
        ui.add_space(4.0);
    }

    ui.add_space(20.0);
    technical_checkbox(ui, app);

    action
}

fn scanning(ui: &mut egui::Ui, app: &mut App, fraction: f32, label: &str) {
    // The subhead says what the scan is actually doing rather than how long
    // it takes: the measurements all come from still frames sampled across
    // the file, not from a pass over every frame, and saying so sets the
    // right expectation for the numbers that follow.
    theme::page_header(
        ui,
        "Analyzing the video file",
        "Analyzing samples from still frames to make a recommendation",
    );
    theme::card().show(ui, |ui| {
        let inner = COLUMN - 34.0;
        ui.set_width(inner);
        app.progress_bar(ui, fraction, inner, theme::BAR_HEIGHT);
        ui.add_space(10.0);
        ui.label(
            RichText::new(label)
                .size(theme::FONT_BODY)
                .color(theme::muted()),
        );
    });
}

fn header(ui: &mut egui::Ui, app: &App) {
    ui.label(
        RichText::new(if app.mode == Mode::JustDeinterlace {
            "Check video"
        } else {
            "Here's what we'll do to your video"
        })
        .size(theme::FONT_TITLE)
        .color(theme::text()),
    );

    // The line under this used to name the file and say "here's what will
    // happen when you export". Both halves are said better elsewhere: the
    // file is named in About this file, which is now the first section on
    // the page, and what will happen is the whole rest of the screen.
    ui.add_space(10.0);
    // The mode, stated and not offered. Back and the sidebar both reach
    // Choose video, so a third route to it was a button earning nothing.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(
            RichText::new("Mode: ")
                .size(theme::FONT_SMALL)
                .color(theme::faint()),
        );
        ui.label(
            RichText::new(app.mode.short_title())
                .size(theme::FONT_SMALL)
                .strong()
                .color(theme::text()),
        );
    });
}

/// A numbered section heading.
pub fn section_number(ui: &mut egui::Ui, number: usize, title: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        // The disc grows with the heading, or a 20pt title sits beside a
        // numbered dot sized for a 17pt one and the pair reads as two
        // unrelated things that happen to be adjacent.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(27.0, 27.0), egui::Sense::hover());
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            p.circle_filled(rect.center(), 13.5, theme::accent_soft());
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                number.to_string(),
                egui::FontId::proportional(13.0),
                theme::accent_hover(),
            );
        }
        ui.label(
            RichText::new(title)
                .size(theme::FONT_SUBHEAD)
                .color(theme::text()),
        );
    });
}

// ── ① The hero card ─────────────────────────────────────────────────────────

fn hero_card(ui: &mut egui::Ui, app: &mut App, f: &Finding) {
    // A progressive file has no fields to weave, and QTGMC is skipped on it.
    // Saying "Remove interlacing lines · Always on" over a file that holds
    // whole frames already promises work that will not happen, and tells
    // someone whose camera shot progressive that their file is damaged.
    let progressive = f.action.is_none();
    theme::card().show(ui, |ui| {
        let inner = COLUMN - 34.0;
        ui.set_width(inner);

        preview_row(ui, app, inner);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            theme::inline_icon(ui, 20.0, theme::ok(), icon::check);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.label(
                        RichText::new(if progressive {
                            "No interlacing to remove"
                        } else {
                            "Remove interlacing lines"
                        })
                        .size(theme::FONT_SECTION)
                        .color(theme::text()),
                    );
                    // A gray pill, not a switch that refuses to switch.
                    pill(ui, if progressive { "Already progressive" } else { "Always on" });
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(&f.explanation)
                        .size(theme::FONT_BODY)
                        .color(theme::muted()),
                );

                if let Some(meaning) = &f.meaning {
                    ui.add_space(10.0);
                    let key = "meaning-interlacing";
                    let mut open = app.is_open(key);
                    if theme::disclosure(ui, "What does this mean?", &mut open) {
                        app.queue_open(key, open);
                    }
                    if open {
                        ui.add_space(8.0);
                        egui::Frame::NONE
                            .fill(theme::bg())
                            .corner_radius(theme::RADIUS_SMALL)
                            .inner_margin(egui::Margin::same(12))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(
                                    RichText::new(meaning)
                                        .size(theme::FONT_SMALL)
                                        .color(theme::muted()),
                                );
                                if let Some(film) = &f.film_line {
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(film)
                                            .size(theme::FONT_SMALL)
                                            .color(theme::text()),
                                    );
                                }
                            });
                    }
                }

                // The raw numbers appear only when technical details are on.
                if app.show_technical {
                    ui.add_space(8.0);
                    theme::details_text(ui, &f.details);
                }
            });
        });

        // Deinterlacing is the one place the frame rate is decided, so the
        // choice belongs here rather than on a settings page the simplest
        // mode never shows. A progressive file has no fields to split, so
        // there is nothing to choose.
        if !progressive {
            ui.add_space(16.0);
            let (rule, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
            if ui.is_rect_visible(rule) {
                ui.painter().rect_filled(rule, 0.0, theme::line());
            }
            ui.add_space(14.0);
            frame_rate_choice(ui, app);

            // Expert mode only, and deliberately. Which of two codebases runs
            // the deinterlace is not a question the other two modes should
            // put to anybody: the bundled one is there, works, and is what
            // every other setting on every other page was tuned against.
            if app.mode == Mode::EnhanceManually {
                ui.add_space(16.0);
                qtgmc_choice(ui, app);
            }
        }
    });
}

/// Which QTGMC does the work.
///
/// The algorithm is the same one either way; the difference is whose code it
/// is. havsfunc is bundled, so it is always available and is the only one the
/// parity harness measures. vs-jetpack's `QTempGaussMC` is a rewrite that is
/// still maintained, but it is a separate install. Both carry the same
/// least-destructive profile; `MatchPreset` is the one setting with no
/// counterpart in the rewrite, which is said on the page rather than left to
/// be discovered by comparing two exports.
fn qtgmc_choice(ui: &mut egui::Ui, app: &mut App) {
    ui.label(
        RichText::new("Deinterlacing tool")
            .size(theme::FONT_BODY)
            .color(theme::text()),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for choice in [QtgmcImpl::Legacy, QtgmcImpl::Modern] {
            let label = if choice == QtgmcImpl::Legacy {
                format!("{} (Recommended)", choice.label())
            } else {
                choice.label().to_string()
            };
            if theme::chip_button(ui, &label, app.config.qtgmc_impl == choice) {
                app.config.qtgmc_impl = choice;
            }
        }
    });
    ui.add_space(10.0);

    // Two sentences on what each one is, and then what we actually measured.
    // The install instructions that used to live here are gone: both ship in
    // the payload, so telling somebody how to pip-install one of them is
    // answering a question they do not have.
    theme::paragraph(ui, RichText::new(if app.config.qtgmc_impl == QtgmcImpl::Modern {
                "vs-jetpack's QTempGaussMC is a newer rewrite of the same algorithm, still actively maintained. It is configured here to match the havsfunc settings as closely as its own controls allow."
            } else {
                "havsfunc's QTGMC is the original implementation, and the one nearly every guide and forum post refers to. It is the default because it is what this app's settings were tuned against."
            })
            .size(theme::FONT_SMALL)
            .color(theme::muted()),);

    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(
            RichText::new("The exact settings are listed in")
                .size(theme::FONT_SMALL)
                .color(theme::muted()),
        );
        if theme::link(ui, "About & updates").clicked() {
            app.show_about = true;
        }
        ui.label(
            RichText::new("· more about the rewrite at")
                .size(theme::FONT_SMALL)
                .color(theme::muted()),
        );
        if theme::link(ui, "the vs-jetpack project").clicked() {
            theme::open_url("https://github.com/Jaded-Encoding-Thaumaturgy/vs-jetpack");
        }
    });

    ui.add_space(10.0);
    // What we measured, stated as what it is: two files on one machine.
    // Anybody choosing between these deserves the numbers and the caveat in
    // the same breath, rather than a recommendation with nothing behind it.
    theme::paragraph(ui, RichText::new(
                "In our tests on an uncompressed capture and a MiniDV capture, the two took about the same time. vs-jetpack produced files roughly 2-14% smaller at the same quality setting, which is the sign of a slightly softer picture rather than better compression — measured directly, it carries about 8% less vertical detail, and vertical is the direction that matters for a deinterlacer. That is why havsfunc is the default.",
            )
            .size(theme::FONT_SMALL)
            .color(theme::muted()),);
    ui.add_space(6.0);
    theme::paragraph(ui, RichText::new(
                "This is preliminary experimentation on two files and one machine. If you know this material, please run your own comparison — the export log names which tool produced each file, so two runs can be compared directly.",
            )
            .size(theme::FONT_SMALL)
            .color(theme::faint()),);
}

/// Double-rate or single-rate, in frames per second rather than in jargon.
///
/// QTGMC can emit one frame per field (59.94 or 50) or one per interlaced
/// frame (29.97 or 25). The first keeps every field the tape recorded, which
/// is every scrap of motion on it; the second throws half of them away. The
/// cost is file size, and that is the whole trade, so both sides of it are on
/// the page.
fn frame_rate_choice(ui: &mut egui::Ui, app: &mut App) {
    let pal = app.config.format == VideoFormat::Pal;
    let (double, single) = if pal { ("50", "25") } else { ("59.94", "29.97") };

    ui.label(
        RichText::new("Frames per second")
            .size(theme::FONT_BODY)
            .color(theme::text()),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if theme::chip_button(
            ui,
            &format!("{double} fps (Recommended)"),
            app.config.double_rate,
        ) {
            app.config.double_rate = true;
        }
        if theme::chip_button(ui, &format!("{single} fps"), !app.config.double_rate) {
            app.config.double_rate = false;
        }
    });
    ui.add_space(8.0);
    theme::paragraph(ui, RichText::new(format!(
                "{double} fps preserves all the data from the raw video, but makes video files \
                 about double the size. {single} fps keeps one frame for every frame on the tape."
            ))
            .size(theme::FONT_SMALL)
            .color(theme::muted()),);
}

/// A gray pill for something that is stated rather than chosen.
fn pill(ui: &mut egui::Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(theme::FONT_TINY),
        theme::muted(),
    );
    // Sized from the text, so it can never wrap mid-word the way the old
    // fixed-width "Required" label did.
    let size = galley.size() + egui::vec2(16.0, 6.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, size.y / 2.0, theme::panel2());
        p.rect_stroke(
            rect,
            size.y / 2.0,
            egui::Stroke::new(1.0, theme::line()),
            egui::StrokeKind::Inside,
        );
        p.galley(rect.center() - galley.size() / 2.0, galley, theme::muted());
    }
}

/// The before/after pair, a skeleton while it renders, or nothing.
fn preview_row(ui: &mut egui::Ui, app: &App, inner_width: f32) {
    let gap = 12.0;
    let each = ((inner_width - gap) / 2.0).floor();

    match (&app.preview, &app.preview_job) {
        (Some((before, after)), _) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                preview_pane(ui, Some(before), each, "Now — comb lines on movement");
                preview_pane(ui, Some(after), each, "After — smooth");
            });
            ui.add_space(16.0);
        }
        (None, Some(job)) if !job.finished => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                preview_pane(ui, None, each, "Preparing preview…");
                preview_pane(ui, None, each, "");
            });
            ui.add_space(10.0);
            // The moving bar belongs under both panes rather than inside one,
            // because the wait is for the pair.
            theme::indeterminate_bar(ui, inner_width, 5.0);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(
                        "Rendering a few seconds of the tape through the \
                         deinterlacer. This takes a minute or two \u{2014} the \
                         rest of the page is ready to read meanwhile.",
                    )
                    .size(theme::FONT_SMALL)
                    .color(theme::faint()),
                );
            });
            ui.add_space(16.0);
            ui.ctx().request_repaint();
        }
        // Finished with no picture. The reason was being recorded on the job
        // and then thrown away: the row simply vanished after a long wait,
        // which is what was reported — "it takes a long time to resolve and
        // then it doesn't load the preview. I think there is a bug."
        //
        // There usually is, and it is usually one of two, so both are named.
        // Saying nothing was the worst of the options available: the render
        // can take a couple of minutes, so an empty space is indistinguishable
        // from one that has not finished yet.
        (None, Some(job)) => {
            preview_failed(ui, app, job.error.as_deref(), inner_width);
            ui.add_space(16.0);
        }
        // No job was ever started — no file, no analysis, or no VapourSynth to
        // run one with. Nothing to report, because nothing was attempted.
        _ => {}
    }
}

/// Why there is no before-and-after, said rather than left blank.
fn preview_failed(ui: &mut egui::Ui, app: &App, error: Option<&str>, width: f32) {
    egui::Frame::NONE
        .fill(theme::panel2())
        .stroke(egui::Stroke::new(1.0, theme::line()))
        .corner_radius(theme::RADIUS)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(width - 28.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                theme::inline_icon(ui, 15.0, theme::info(), icon::info);
                ui.label(
                    RichText::new("No before-and-after for this file")
                        .size(theme::FONT_BODY)
                        .color(theme::text()),
                );
            });
            ui.add_space(6.0);
            theme::paragraph(ui, RichText::new(
                        "Everything else on this page was measured normally, and the \
                         export is unaffected — the preview is the only thing missing. \
                         It needs the bundled VapourSynth, and it needs a moment of \
                         movement in the tape to compare; a static shot gives it \
                         nothing to show.",
                    )
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),);
            // The exact reason is a tool message, so it sits behind the same
            // checkbox every other raw measurement does.
            if app.show_technical && let Some(why) = error {
                ui.add_space(8.0);
                theme::details_text(ui, why);
            }
        });
}

fn preview_pane(
    ui: &mut egui::Ui,
    texture: Option<&egui::TextureHandle>,
    width: f32,
    caption: &str,
) {
    ui.vertical(|ui| {
        ui.set_width(width);
        match texture {
            Some(t) => {
                let size = t.size_vec2();
                let scale = width / size.x;
                ui.add(
                    egui::Image::new(t)
                        .fit_to_exact_size(size * scale)
                        .corner_radius(theme::RADIUS_SMALL),
                );
            }
            None => {
                // A skeleton of the same shape, so nothing jumps when the
                // picture arrives.
                let height = (width * 2.0 / 3.0).floor();
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                if ui.is_rect_visible(rect) {
                    ui.painter()
                        .rect_filled(rect, theme::RADIUS_SMALL, theme::panel2());
                }
            }
        }
        if !caption.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(caption)
                    .size(theme::FONT_SMALL)
                    .color(theme::faint()),
            );
        }
    });
}

// ── ② Just deinterlace: observations only ───────────────────────────────────

/// The one repair this mode makes to something other than the scan.
///
/// Reported from testing: a mono file was loaded, the sound was duplicated to
/// both channels, and no screen said so. Returns whether anything was drawn,
/// so the section after it can take the next number.
fn sound_section(ui: &mut egui::Ui, app: &App, all: &[Finding], number: usize) -> bool {
    let Some(f) = all.iter().find(|f| f.check == Check::Audio && f.is_fix()) else {
        return false;
    };

    section_number(ui, number, "The sound");
    ui.add_space(10.0);
    theme::card().show(ui, |ui| {
        ui.set_width(COLUMN - 34.0);
        notice_row(
            ui,
            app,
            Some(Tag::Required),
            &f.title,
            &f.explanation,
            Some(&f.details),
        );
        if let Some(a) = &f.action {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                theme::paragraph(ui, RichText::new(format!("We'll {a}."))
                            .size(theme::FONT_SMALL)
                            .color(theme::muted()),);
            });
        }
    });
    true
}

fn also_noticed(
    ui: &mut egui::Ui,
    app: &App,
    all: &[Finding],
    report: &Report,
    action: &mut PageAction,
    number: usize,
) {
    // Audio is excluded: it is handled above as something that will happen,
    // and listing it here as well would offer to fix what is already being
    // fixed.
    let others: Vec<&Finding> = all
        .iter()
        .filter(|f| {
            f.check != Check::Interlacing && f.check != Check::Audio && f.is_fix()
        })
        .collect();
    // This mode runs no brightness, color, noise, halo or bleeding probe at
    // all, so an empty list here means nothing was looked at — not that
    // nothing was found. Reporting the second is the one thing that would
    // make every other finding on the page untrustworthy, so the section is
    // omitted outright rather than filled with a sentence about the absence
    // of a measurement. Saying nothing is not the same as saying it is fine,
    // and a reader in the mode called "Just deinterlace" has already been
    // told, on the card they picked it from, that nothing else is touched.
    if !findings::enhancement_probes_ran(report) {
        return;
    }

    section_number(ui, number, "We also noticed");
    ui.add_space(10.0);

    theme::card().show(ui, |ui| {
        ui.set_width(COLUMN - 34.0);

        if others.is_empty() {
            notice_row(
                ui,
                app,
                None,
                "Nothing else needs fixing",
                "Brightness, color, noise and sharpness all look normal.",
                None,
            );
            return;
        }

        for (i, f) in others.iter().enumerate() {
            if i > 0 {
                ui.add_space(14.0);
            }
            notice_row(
                ui,
                app,
                Some(Tag::Info),
                &f.notice_title,
                &f.notice_text,
                Some(&f.details),
            );
        }

        ui.add_space(16.0);
        switch_mode_row(ui, action, "Fix these too  ");
    });
}

/// What Full control gets instead of a list of fixes: a count and a pointer.
///
/// The findings are not hidden — every one of them opens its own page a few
/// screens from here, with the measurement, a recommendation and a
/// before/after. This says how many there are so the reader knows what is
/// coming, without asking them to decide anything yet.
/// The rule, the sentence about what this mode leaves alone, and the button
/// that switches to the one that does not.
fn switch_mode_row(ui: &mut egui::Ui, action: &mut PageAction, button: &str) {
    let (rule, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    if ui.is_rect_visible(rule) {
        ui.painter().rect_filled(rule, 0.0, theme::line());
    }
    ui.add_space(14.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(
                RichText::new("These won't be changed in ")
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
            );
            ui.label(
                RichText::new(Mode::JustDeinterlace.short_title())
                    .size(theme::FONT_SMALL)
                    .strong()
                    .color(theme::text()),
            );
            ui.label(
                RichText::new(" mode.")
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Switches mode in place; no navigation. The analysis reruns,
            // because the new mode needs probes this one skipped.
            if theme::secondary(ui, button).clicked() {
                action.set_mode = Some(Mode::EnhanceAutomatically);
            }
        });
    });
}

/// An observation row: icon, plain title, one sentence, optional numbers.
fn notice_row(
    ui: &mut egui::Ui,
    app: &App,
    tag: Option<Tag>,
    title: &str,
    text: &str,
    details: Option<&str>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 11.0;
        let (color, is_info) = match tag {
            Some(Tag::Info) => (theme::info(), true),
            _ => (theme::ok(), false),
        };
        theme::inline_icon(ui, 17.0, color, move |p, r, c| {
            if is_info {
                icon::info(p, r, c)
            } else {
                icon::check(p, r, c)
            }
        });
        ui.vertical(|ui| {
            ui.label(
                RichText::new(title)
                    .size(theme::FONT_BODY)
                    .color(theme::text()),
            );
            ui.add_space(2.0);
            ui.label(
                RichText::new(text)
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
            );
            if let Some(d) = details
                && app.show_technical {
                    ui.add_space(6.0);
                    theme::details_text(ui, d);
                }
        });
    });
}

// ── ② / ③ Auto-fix and Full control ─────────────────────────────────────────

fn fixes_and_passes(
    ui: &mut egui::Ui,
    app: &App,
    all: &[Finding],
    report: &Report,
    action: &mut PageAction,
) {
    let fixes: Vec<&Finding> = all
        .iter()
        .filter(|f| f.is_fix() && f.check != Check::Interlacing)
        .collect();
    let passed: Vec<&Finding> = all.iter().filter(|f| !f.is_fix()).collect();

    section_number(ui, 3, "What we suggest fixing");
    ui.add_space(12.0);

    if fixes.is_empty() {
        theme::card().show(ui, |ui| {
            ui.set_width(COLUMN - 34.0);
            notice_row(
                ui,
                app,
                None,
                "Nothing else needs fixing",
                "Brightness, color, noise and sharpness all look normal.",
                None,
            );
        });
    } else {
        for (i, f) in fixes.iter().enumerate() {
            if i > 0 {
                ui.add_space(10.0);
            }
            if let Some((check, on)) = fix_card(ui, app, f, report) {
                action.set_fix = Some((check, on));
            }
        }
    }

    if !passed.is_empty() {
        ui.add_space(22.0);
        section_number(ui, 4, "Looks good — nothing to do");
        ui.add_space(10.0);
        theme::card().show(ui, |ui| {
            ui.set_width(COLUMN - 34.0);
            for (i, f) in passed.iter().enumerate() {
                if i > 0 {
                    ui.add_space(14.0);
                }
                if let Some(change) = passed_row(ui, app, f) {
                    action.set_fix = Some(change);
                }
                if let Some(note) = &f.note {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(28.0);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        theme::inline_icon(ui, 15.0, theme::info(), icon::info);
                        ui.label(
                            RichText::new(note)
                                .size(theme::FONT_SMALL)
                                .color(theme::info()),
                        );
                    });
                }
            }
        });
    }
}

/// A check that came back clean, with a switch to turn it on anyway.
///
/// "Nothing to do" is a finding, not a prohibition: a tape can measure clean
/// and still look wrong to the person who shot it, and the probes are
/// thresholds rather than oracles. Turning one on here applies the same
/// strength the fix cards would have used.
///
/// Deinterlacing is excluded: it is not optional, so a switch that cannot
/// move would only be confusing.
fn passed_row(ui: &mut egui::Ui, app: &App, f: &Finding) -> Option<(Check, bool)> {
    const RIGHT: f32 = 92.0;
    let inner = COLUMN - 34.0;
    let mut changed = None;
    let switchable = f.check != Check::Interlacing;

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(if switchable {
                inner - RIGHT - 16.0
            } else {
                inner
            });
            notice_row(ui, app, None, &f.title, &f.explanation, Some(&f.details));
        });

        if !switchable {
            return;
        }
        ui.allocate_ui_with_layout(
            egui::vec2(RIGHT, 46.0),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                let mut on = f.check.is_on(&app.config);
                if theme::toggle(ui, &mut on, false) {
                    changed = Some((f.check, on));
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(if on { "Apply" } else { "Not needed" })
                        .size(theme::FONT_TINY)
                        .color(theme::muted()),
                );
            },
        );
    });
    changed
}

/// One recommended fix, with an Apply toggle.
fn fix_card(
    ui: &mut egui::Ui,
    app: &App,
    f: &Finding,
    _report: &Report,
) -> Option<(Check, bool)> {
    let mut changed = None;
    // A fixed right-hand column, so the toggle and its label always have room
    // and the card width never depends on what is open.
    const RIGHT: f32 = 92.0;
    let inner = COLUMN - 36.0;

    let on = f.check.is_on(&app.config);

    let frame = theme::edged_card().show(ui, |ui| {
        ui.set_width(inner);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(inner - RIGHT - 16.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.label(
                        RichText::new(&f.notice_title)
                            .size(theme::FONT_SECTION)
                            .color(theme::text()),
                    );
                    theme::tag(ui, f.tag, f.tag_text.as_deref());
                });
                ui.add_space(7.0);
                ui.label(
                    RichText::new(&f.notice_text)
                        .size(theme::FONT_SMALL)
                        .color(theme::muted()),
                );
                // What this card will do, in the tense the switch beside it
                // is actually in. "We'll bring blacks and whites into the
                // standard range" over a switch reading Skip is a promise
                // the export will not keep, and it was read as a bug in the
                // switch rather than as a sentence written for the other
                // state. Brightness is the one that starts off — see its
                // note below — so this was the common case, not a corner.
                if let Some(act) = &f.action {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(if on {
                            format!("We'll {act}.")
                        } else {
                            format!("Switch this on to {act}.")
                        })
                        .size(theme::FONT_SMALL)
                        .color(if on { theme::text() } else { theme::muted() }),
                    );
                }

                // Why a recommended fix might still start switched off.
                if let Some(note) = &f.note {
                    ui.add_space(8.0);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        theme::inline_icon(ui, 15.0, theme::info(), icon::info);
                        theme::paragraph(ui, RichText::new(note)
                                    .size(theme::FONT_SMALL)
                                    .color(theme::info()),);
                    });
                }

                ui.add_space(9.0);
                let key = format!("why-{}", f.check.id());
                let mut open = app.is_open(&key);
                if theme::disclosure(ui, "Why?", &mut open) {
                    app.queue_open(&key, open);
                }
                if open || app.show_technical {
                    ui.add_space(6.0);
                    theme::details_text(ui, &f.details);
                }

                if findings::shows_fine_tune(app.mode)
                    && let Some(step) = f.check.settings_step() {
                        ui.add_space(6.0);
                        if theme::link(ui, &format!("Fine-tune in {}", step.title())).clicked()
                        {
                            app.queue_goto(step);
                        }
                    }
            });

            ui.allocate_ui_with_layout(
                egui::vec2(RIGHT, 60.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    let mut switch = on;
                    if theme::toggle(ui, &mut switch, false) {
                        changed = Some((f.check, switch));
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(if switch { "Apply" } else { "Skip" })
                            .size(theme::FONT_TINY)
                            .color(theme::muted()),
                    );
                },
            );
        });
    });

    theme::left_edge(ui, frame.response.rect, f.tag.color());
    changed
}

// ── ③ About this file ───────────────────────────────────────────────────────

fn about_file(ui: &mut egui::Ui, app: &mut App, report: &Report) {
    theme::card().show(ui, |ui| {
        ui.set_width(COLUMN - 34.0);

        // Open, with no disclosure to click. This is the first thing on the
        // page now, and a closed panel at the top of a screen is a page that
        // appears to begin with nothing on it.
        if let Some(path) = &app.file {
            ui.label(
                RichText::new(
                    path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
                .size(theme::FONT_SECTION)
                .color(theme::text()),
            );
            ui.add_space(10.0);
        }

        if let Some(i) = &report.info {
            about_row(
                ui,
                "Picture size",
                &format!("{} × {}", i.width, i.height),
                picture_size_note(i.width, i.height),
            );
            about_row(ui, "Length", &crate::pages::format_duration(i.duration), "");
        }
        if let Some(p) = &report.par {
            let (name, note) = match p.format {
                Some(VideoFormat::Ntsc) => ("NTSC", "North American video"),
                Some(VideoFormat::Pal) => ("PAL", "European/international video"),
                None => ("Unknown", ""),
            };
            about_row(ui, "TV system", name, note);
        }
        if let Some(i) = &report.info {
            about_row(ui, "Frames per second", &format!("{:.2}", i.fps), "");
        }
        if let Some(c) = &report.classification {
            about_row(ui, "Format", &format_name(c), format_plain(&c.codec).1);
        }

        // Field order, on the face of the panel rather than behind the
        // technical checkbox.
        //
        // It is the single most consequential fact about an interlaced file:
        // deinterlacing with the wrong one interpolates each field against
        // the neighbor on the wrong side of it in time, and the result looks
        // worse than the source. Most tape files declare nothing, so what the
        // app decided is not guessable from anywhere else, and a user who
        // knows their own deck is the one person who can catch us being
        // wrong. That is only possible if the decision is visible.
        {
            use vcg_core::config::FieldOrder as F;
            let declared = report.classification.as_ref().is_some_and(|c| {
                c.field_order != vcg_core::analysis::probe::DetectedFieldOrder::Unknown
            });
            let measured = report
                .telecine
                .as_ref()
                .and_then(|t| t.measured_field_order())
                .is_some();
            let (name, note) = match app.config.field_order {
                F::Tff => (
                    "Top field first",
                    if declared {
                        "the file says so"
                    } else if measured {
                        "measured from the picture"
                    } else {
                        "usual for an analog capture card"
                    },
                ),
                F::Bff => (
                    "Bottom field first",
                    if declared {
                        "the file says so"
                    } else if measured {
                        "measured from the picture"
                    } else {
                        "usual for DV camcorders"
                    },
                ),
                F::Progressive => (
                    "Progressive",
                    "whole frames already — nothing to deinterlace",
                ),
            };
            about_row(ui, "Field order", name, note);
        }

        // The raw identifiers are for someone who asked for them.
        if app.show_technical {
            ui.add_space(12.0);
            let mut bits: Vec<String> = Vec::new();
            if let Some(c) = &report.classification {
                bits.push(c.codec.clone());
                bits.push(c.pix_fmt.clone());
                bits.push(format!("described as {}", c.display_name));
                use vcg_core::analysis::probe::DetectedFieldOrder as F;
                bits.push(
                    match c.field_order {
                        F::Tff => "field order top first".to_string(),
                        F::Bff => "field order bottom first".to_string(),
                        F::Progressive => "field order progressive".to_string(),
                        F::Unknown => "field order not stored (measured)".to_string(),
                    },
                );
            }
            let frames = findings::frames_measured(report);
            if frames > 0 {
                bits.push(format!("{frames} frames analyzed"));
            }
            theme::details_text(ui, &bits.join(" · "));
        }
    });
}

fn about_row(ui: &mut egui::Ui, label: &str, value: &str, note: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            egui::vec2(170.0, 21.0),
            egui::Label::new(
                RichText::new(label)
                    .size(theme::FONT_SMALL)
                    .color(theme::faint()),
            )
            .halign(egui::Align::LEFT),
        );
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(
            RichText::new(value)
                .size(theme::FONT_SMALL)
                .color(theme::text()),
        );
        if !note.is_empty() {
            ui.label(
                RichText::new(format!("— {note}"))
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
            );
        }
    });
    ui.add_space(3.0);
}

/// A plain gloss for a frame size.
fn picture_size_note(width: u32, height: u32) -> &'static str {
    match (width, height) {
        (_, 480) | (_, 576) => "standard definition",
        (_, 720) => "high definition",
        (_, 1080) => "full high definition",
        (_, h) if h >= 2000 => "ultra high definition",
        _ => "",
    }
}

/// What to show on the Format row: the plain name, and the FourCC behind it.
///
/// The FourCC is shown because the plain name alone can be surprising and
/// there is otherwise nothing on screen to check it against. An uncompressed
/// AVI is the case that prompted this: a file named for HuffYUV decoded as
/// `rawvideo`, the row said "Uncompressed", and nothing on the page showed
/// that the container really does say `YUY2` — the file had been captured
/// without compression and only its name claimed otherwise. Printing the tag
/// turns a bare verdict into something the user can confirm against what
/// their capture program was set to.
///
/// A codec with no name of its own falls back to the codec's own name, which
/// is what the match arm below always meant to do.
fn format_name(c: &vcg_core::analysis::probe::Classification) -> String {
    let (name, _) = format_plain(&c.codec);
    let name = if name.is_empty() { c.codec.as_str() } else { name };
    match (name, c.codec_tag.as_str()) {
        ("", "") => "Unknown".to_string(),
        ("", tag) => tag.to_string(),
        (name, "") => name.to_string(),
        // No point printing "HuffYUV (HFYU)" twice over when the tag just
        // restates the name.
        (name, tag) if name.eq_ignore_ascii_case(tag) => name.to_string(),
        (name, tag) => format!("{name} ({tag})"),
    }
}

/// A plain name and gloss for a codec.
fn format_plain(codec: &str) -> (&'static str, &'static str) {
    match codec {
        "rawvideo" => ("Uncompressed", "full quality, large file"),
        "dvvideo" => ("DV", "digital camcorder tape"),
        "mpeg2video" => ("MPEG-2", "DVD or HDV compression"),
        "h264" => ("H.264", "modern compression, small file"),
        "hevc" => ("HEVC", "newer compression, small file"),
        "ffv1" => ("FFV1", "lossless — full quality, smaller than uncompressed"),
        "huffyuv" | "ffvhuff" => ("HuffYUV", "lossless — full quality"),
        "utvideo" => ("Ut Video", "lossless — full quality"),
        "prores" => ("ProRes", "editing format, high quality"),
        "lagarith" => ("Lagarith", "lossless — full quality"),
        // An unrecognized codec keeps its own name rather than being guessed at.
        _ => ("", ""),
    }
}

/// The one control for technical detail, at the bottom where it belongs.
fn technical_checkbox(ui: &mut egui::Ui, app: &mut App) {
    let mut show = app.show_technical;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if ui
            .add(egui::Checkbox::new(
                &mut show,
                RichText::new("Show technical details")
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),
            ))
            .changed()
        {
            app.set_show_technical(show);
        }
    });
}
