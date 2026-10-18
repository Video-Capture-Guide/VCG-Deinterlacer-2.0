//! The About panel: what the app is, and what it does to a tape.
//!
//! A modal rather than a wizard step. It is not part of the walk from a file
//! to an export — a step would put it in the sidebar's numbering and in
//! "Step N of M", which would be a lie about the shape of the job — and it
//! has to be reachable from anywhere, including mid-export.
//!
//! Everything factual on this page is read from the code that actually does
//! the work: the QTGMC profile comes from `vcg_core::vpy::qtgmc_profile()`,
//! the version from `vcg_core::VERSION`. The Python original kept its About
//! text as a hand-written block beside the settings dict, and the two drifted
//! apart almost immediately — by the end it described `NoiseProcess=2` on a
//! build that had been shipping something else for months. A page that
//! documents settings must read them, not restate them.

use eframe::egui::{self, RichText};

use vcg_deps::update::{Outcome, Release};

use crate::theme;
use crate::App;

/// Width of the panel's content column.
const WIDTH: f32 = 720.0;

/// What the update check is doing right now.
#[derive(Debug, Default)]
pub enum UpdateState {
    /// Nothing asked for yet.
    #[default]
    Idle,
    Checking,
    Done(Outcome),
}

/// Draw the About panel if it is open. Returns false when it should close.
pub fn about_modal(ctx: &egui::Context, app: &mut App) {
    if !app.show_about {
        return;
    }

    // Measured out here, not inside the closure: within the modal's own area
    // `content_rect` reports that area rather than the window, which capped
    // the panel at a few hundred pixels and hid most of it behind a scroll
    // nobody could see the need for.
    let max_height = (ctx.content_rect().height() - 120.0).max(320.0);

    let response = egui::Modal::new(egui::Id::new("about"))
        .frame(
            egui::Frame::NONE
                .fill(theme::panel())
                .corner_radius(theme::RADIUS)
                .inner_margin(egui::Margin::symmetric(28, 24))
                .stroke(egui::Stroke::new(1.0, theme::line())),
        )
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            // The height has to be *reserved*, not just capped. A modal's
            // Area sizes itself to its content, so a ScrollArea inside one
            // has no available height to expand into and collapses to a few
            // hundred pixels however high its `max_height` is — which hid
            // most of this panel behind a scrollbar on a window with room to
            // show it.
            ui.set_min_height(max_height);
            // The window is not always tall enough for this, and a modal that
            // runs off the bottom of the screen has no way to be read.
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .auto_shrink([false, false])
                .show(ui, |ui| body(ui, app));
        });

    // Click the backdrop or press Escape to dismiss, which is what every
    // other modal on the platform does.
    if response.should_close() {
        app.show_about = false;
    }
}

fn body(ui: &mut egui::Ui, app: &mut App) {
    ui.set_width(WIDTH - 16.0);

    heading(ui);
    ui.add_space(18.0);

    section(ui, "What this does");
    paragraph(
        ui,
        "A Windows tool for deinterlacing VHS, Hi8, MiniDV and other tape \
         captures using QTGMC, the highest-quality software deinterlacer \
         there is. It measures your capture, recommends what to do about \
         what it finds, and writes an archival master. Every recommendation \
         can be overridden, and every number behind one can be read.",
    );
    ui.add_space(16.0);

    section(ui, "QTGMC settings");
    paragraph(
        ui,
        "VCG uses a least-destructive QTGMC profile whose only job is to \
         deinterlace. It does not denoise — that is the separate Noise step — \
         applies minimal sharpening, and maximizes fidelity to the original \
         fields.",
    );
    ui.add_space(10.0);
    settings_table(ui);
    ui.add_space(10.0);
    paragraph_small(
        ui,
        "All other parameters use QTGMC's Slower-preset defaults. Both \
         deinterlacer backends — the bundled havsfunc and the optional \
         vs-jetpack rewrite — carry this same profile.",
    );
    ui.add_space(16.0);

    section(ui, "Key design choices");
    paragraph(
        ui,
        "SourceMatch=3 together with Lossless=2 keeps the output as close to \
         the captured fields as possible: the first corrects the result back \
         toward the source, the second puts the exact source field lines back \
         in. Sharpness=0.1 is far below the typical 0.2–1.0, so the analog \
         look survives and no halos are added for the halo step to undo. \
         NoiseProcess is off, so grain and texture pass through untouched and \
         the separate Noise step is the only thing that removes them.",
    );
    ui.add_space(8.0);
    paragraph(
        ui,
        "The trade-off: slightly more residual shimmer and aliasing than an \
         aggressive preset would leave, in exchange for fidelity to the tape. \
         That is the right way round for an archival tool, where the source \
         cannot be recaptured and a destroyed detail is gone for good.",
    );
    ui.add_space(16.0);

    section(ui, "Denoising");
    paragraph(
        ui,
        "QTGMC itself performs no denoising. When the Noise step is enabled \
         it runs after the deinterlacer, on the progressive frames, and is \
         the only thing that removes noise. BM3D is the tool — block-matching \
         in three dimensions — and the script picks the fastest build present: \
         CUDA on an NVIDIA card, otherwise an AVX2 build four times quicker \
         than the original, then the original, then KNLMeansCL, then \
         SMDegrain. Only the luma is denoised; \
         tape chroma is already soft and filtering it further costs more than \
         it saves.",
    );
    ui.add_space(16.0);

    section(ui, "Pixel aspect and cropping");
    paragraph(
        ui,
        "An analog capture card digitises a 13.5 MHz line into 720 samples of \
         which 704 are active, so 704 is 4:3 and the pixel aspect is 10/11. \
         DV uses the whole 720 as active, giving 8/9 — which is why DV \
         defaults to no crop. Both resize to 640×480 square pixels.",
    );
    ui.add_space(8.0);
    paragraph(
        ui,
        "The crop itself is measured off your own capture rather than taken \
         from a preset, because real capture-card blanking is rarely \
         symmetric. Three different things sit at the frame edge and each is \
         tested for separately: flat black blanking, the analog edge ramp, \
         and the torn head-switching band along the bottom.",
    );
    ui.add_space(16.0);

    section(ui, "Field order");
    paragraph(
        ui,
        "Field order is the single most consequential fact about an \
         interlaced file. Fed the wrong one, QTGMC interpolates each field \
         against the neighbour on the wrong side of it in time, and the \
         result looks worse than the source. Most tape captures declare \
         nothing, so it is measured from the picture — and what the app \
         decided is shown on the Check video page so you can overrule it.",
    );
    ui.add_space(16.0);

    section(ui, "Export formats");
    for (name, line) in [
        (
            "ProRes 422 HQ",
            "10-bit 4:2:2 in .mov, about 50 GB an hour. What editing software expects.",
        ),
        (
            "FFV1",
            "Mathematically lossless, 8-bit 4:2:2 in .mkv, about 35 GB an hour. The preservation choice, and smaller than the ProRes.",
        ),
        (
            "H.264",
            "8-bit 4:2:0 in .mp4 at CRF 18, about 1.5 GB an hour. For sharing, not for keeping.",
        ),
    ] {
        bullet(ui, name, line);
    }
    ui.add_space(8.0);
    paragraph_small(
        ui,
        "Colour is tagged, not converted. Sound is carried through as 16-bit \
         PCM except in H.264, which re-encodes to AAC.",
    );
    ui.add_space(16.0);

    section(ui, "Components");
    components(ui, app);
    ui.add_space(16.0);

    section(ui, "Appearance");
    appearance(ui, app);
    ui.add_space(16.0);

    section(ui, "Updates");
    updates(ui, app);
    ui.add_space(18.0);

    ui.separator();
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if theme::primary(ui, "Close", true).clicked() {
            app.show_about = false;
        }
        ui.add_space(8.0);
        if theme::link(ui, "Project page on GitHub").clicked() {
            theme::open_url(vcg_core::REPO_URL);
        }
    });
}

fn heading(ui: &mut egui::Ui) {
    ui.label(
        RichText::new("VCG Deinterlacer")
            .size(theme::FONT_TITLE)
            .family(theme::semibold())
            .color(theme::text()),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!(
            "Version {}  ·  built {}  ·  {}",
            vcg_core::VERSION,
            vcg_core::BUILD_DATE,
            vcg_core::AUTHOR_HANDLE
        ))
        .size(theme::FONT_SMALL)
        .color(theme::muted()),
    );
}

/// The QTGMC profile, read from the generator rather than retyped.
fn settings_table(ui: &mut egui::Ui) {
    theme::card().show(ui, |ui| {
        ui.set_width(WIDTH - 60.0);
        for (key, value, why) in vcg_core::vpy::qtgmc_profile() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // `add_sized` centres its content in the cell, which right-
                // ragged the key column and made the table hard to scan. An
                // explicit left-to-right child of fixed width lines them up.
                ui.allocate_ui_with_layout(
                    egui::vec2(190.0, theme::FONT_SMALL + 4.0),
                    egui::Layout::left_to_right(egui::Align::TOP),
                    |ui| {
                        ui.label(
                            RichText::new(format!("{key}={value}"))
                                .size(theme::FONT_SMALL)
                                .monospace()
                                .color(theme::accent_hover()),
                        );
                    },
                );
                theme::paragraph(ui, RichText::new(why).size(theme::FONT_SMALL).color(theme::muted()),);
            });
            ui.add_space(5.0);
        }
    });
}

/// What the app found to work with, and whether it is complete.
///
/// This used to sit on the front page, where it was the largest thing on
/// screen and said "everything is fine" to somebody who had not yet asked.
/// It belongs here: it is reference material, consulted when a support
/// question needs answering or a render has failed. The front page keeps
/// only the bad news, because that is the case that stops the user working
/// and so has to be said where they are standing.
fn components(ui: &mut egui::Ui, app: &App) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(
            RichText::new(if app.deps_ready {
                "Everything needed is installed."
            } else {
                "Something is missing."
            })
            .size(theme::FONT_BODY)
            .color(theme::muted()),
        );
        ui.label(
            RichText::new(&app.deps_summary)
                .size(theme::FONT_SMALL)
                .color(if app.deps_ready { theme::ok() } else { theme::fix() }),
        );
    });
    ui.add_space(10.0);
    theme::card().show(ui, |ui| {
        ui.set_width(WIDTH - 60.0);
        // Which engine is in use, so a support question is answerable. A
        // report of "the colours are wrong" is unanswerable without knowing
        // which of the three VapourSynth installs on a machine actually ran.
        for line in &app.tool_lines {
            ui.label(
                RichText::new(line)
                    .size(theme::FONT_SMALL)
                    .color(theme::faint()),
            );
        }
    });
    if !app.deps_ready {
        ui.add_space(10.0);
        paragraph_small(
            ui,
            "The components download on first run. If that did not happen,              close the app and run vcg-deps install beside the executable.",
        );
    }
}

/// Light, dark, or whatever Windows is doing.
///
/// "Match Windows" is the default and the first option, because following
/// the system is what the platform expects of an app and what most people
/// will want. The other two exist because a colourist working in a dark room
/// and somebody editing beside a window are both right, and neither of them
/// wants to change a system setting to get it.
fn appearance(ui: &mut egui::Ui, app: &mut App) {
    paragraph(
        ui,
        "The window follows your Windows light or dark setting, and changes          with it while the app is open. Override it here if you would rather          it did not.",
    );
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for choice in [
            theme::ThemeChoice::System,
            theme::ThemeChoice::Light,
            theme::ThemeChoice::Dark,
        ] {
            if theme::chip_button(ui, choice.label(), app.theme_choice == choice) {
                app.theme_choice = choice;
            }
        }
    });

    ui.add_space(16.0);
    paragraph(
        ui,
        "Text size scales the whole interface, not just the words, so          nothing crowds or overlaps at the larger settings. It takes effect          as soon as you choose it.",
    );
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for size in theme::TextSize::ALL {
            if theme::chip_button(ui, size.label(), app.text_size == size) {
                app.text_size = size;
            }
        }
    });
}

fn updates(ui: &mut egui::Ui, app: &mut App) {
    paragraph(
        ui,
        "This checks GitHub for a newer release. It does not install \
         anything: the app cannot safely replace its own running .exe, so an \
         update means downloading the new one and putting it where this one \
         is. Nothing is sent to GitHub but the request itself.",
    );
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        let busy = matches!(app.update_state, UpdateState::Checking);
        if theme::secondary(ui, if busy { "Checking…" } else { "Check for updates" })
            .clicked()
            && !busy
        {
            app.start_update_check();
        }
        ui.add_space(10.0);
        match &app.update_state {
            UpdateState::Idle => {}
            UpdateState::Checking => {
                ui.add(egui::Spinner::new().size(14.0));
            }
            UpdateState::Done(Outcome::UpToDate) => {
                ui.label(
                    RichText::new("This is the latest version.")
                        .size(theme::FONT_SMALL)
                        .color(theme::ok()),
                );
            }
            UpdateState::Done(Outcome::Failed(why)) => {
                // Not a warning colour. Being offline is not a fault in the
                // app, and an amber badge would read as one.
                ui.label(
                    RichText::new(format!("Could not check: {why}"))
                        .size(theme::FONT_SMALL)
                        .color(theme::muted()),
                );
            }
            UpdateState::Done(Outcome::Available(r)) => {
                let r: Release = r.clone();
                ui.label(
                    RichText::new(format!("Version {} is available.", r.version))
                        .size(theme::FONT_SMALL)
                        .color(theme::accent_hover()),
                );
                if theme::link(ui, "Open the download page").clicked() {
                    theme::open_url(&r.page_url);
                }
            }
        }
    });
}

// ── Small text helpers ──────────────────────────────────────────────────────

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .size(theme::FONT_SECTION)
            .family(theme::semibold())
            .color(theme::text()),
    );
    ui.add_space(7.0);
}

fn paragraph(ui: &mut egui::Ui, text: &str) {
    theme::paragraph(ui, RichText::new(text)
                .size(theme::FONT_BODY)
                .color(theme::muted()),);
}

fn paragraph_small(ui: &mut egui::Ui, text: &str) {
    theme::paragraph(ui, RichText::new(text).size(theme::FONT_SMALL).color(theme::faint()),);
}

fn bullet(ui: &mut egui::Ui, name: &str, text: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(
            egui::vec2(140.0, theme::FONT_BODY + 4.0),
            egui::Layout::left_to_right(egui::Align::TOP),
            |ui| {
                ui.label(RichText::new(name).size(theme::FONT_SMALL).color(theme::text()));
            },
        );
        theme::paragraph(ui, RichText::new(text).size(theme::FONT_SMALL).color(theme::muted()),);
    });
    ui.add_space(4.0);
}

