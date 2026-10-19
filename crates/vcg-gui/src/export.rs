//! The Export screen.
//!
//! The last page, and the only one that writes anything. Three states live
//! here and they are deliberately different shapes rather than one layout
//! with things grayed out: *ready* asks where it goes and what kind of file;
//! *running* shows nothing but progress, because there is nothing useful to
//! decide while it works; *finished* says where the file is.
//!
//! A tape takes longer than its own running time to rebuild — QTGMC is slow —
//! so this screen will be on display for a while. It says what it is doing in
//! words rather than leaving a bar to speak for itself.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};

use vcg_core::config::{Config, Mode, OutputFormat};

use crate::check::COLUMN;
use crate::pages::PageAction;
use crate::theme::{self, Tag};
use crate::App;

/// The formats worth offering, in the order a user would weigh them.
///
/// ProRes is first because it suits most people, not because it is the best
/// quality — it is not. FFV1 is lossless and ProRes is only *visually*
/// lossless, so labelling ProRes "best quality" was simply wrong, and an
/// archivist reading it would have picked the worse file.
///
/// The six the Python supports are mostly historical: Huffyuv, Lagarith and
/// Ut Video are lossless AVI codecs that exist for compatibility with much
/// older tools, and offering all of them here would be six ways to ask the
/// same question. These three cover archive, edit and share.
/// One output format, as offered on screen.
pub struct FormatChoice {
    pub format: OutputFormat,
    /// What it is for, in the user's terms.
    pub title: &'static str,
    /// The trade-off, in one line.
    pub note: &'static str,
    /// Roughly how much an hour of SD costs on disk.
    pub per_hour: &'static str,
    /// For someone who knows video and wants the actual settings.
    pub details: &'static [&'static str],
}

/// The formats worth offering, in the order a user would weigh them.
///
/// The Python offers six. Huffyuv, Lagarith and Ut Video are lossless AVI
/// codecs kept for compatibility with much older tools, and all three answer
/// the same question FFV1 answers better. Archive, edit and share covers it.
///
/// The `details` are deliberately specific — exact codec flags, bit depth,
/// chroma, what gets tagged — because an expert deciding between ProRes HQ
/// and FFV1 is deciding on precisely those, and a vague summary would make
/// them open the file in MediaInfo to find out.
pub const FORMATS: &[FormatChoice] = &[
    FormatChoice {
        format: OutputFormat::Prores,
        title: "Best for editing (ProRes 422 HQ)",
        note: "What editing software expects, and fast to scrub. Visually lossless, but not lossless.",
        per_hour: "about 50 GB an hour",
        details: &[
            "ProRes 422 HQ (prores_ks, profile 3) in a QuickTime .mov.",
            "10-bit 4:2:2, yuv422p10le. The pipeline processes at 16-bit and every format is dithered down to the codec's depth on the way out — 10 bits here, against 8 for the other two — with fmtc, error diffusion by default. That is what keeps banding from the deinterlacer's own maths out of the file. Ten bits leave the most room of the three, so this is the format least likely to band.",
            "Visually lossless in practice but not mathematically lossless: it is a DCT codec. Measured here at 103-110 Mbps on two SD captures at 640x480 double-rate, which is about 46-50 GB an hour. Fine as an edit master and fine to grade; if you want bit-exact, use FFV1.",
            "Color is tagged rather than converted — BT.601 primaries, smpte170m for NTSC or bt470bg for PAL, and PAL gets gamma28 as its transfer. Nothing resamples the chroma on the way out. The one exception is upscaling to HD: an HD-sized frame is read as BT.709 by players, so the pixels really are converted 601 to 709 in that case, and the tags follow them.",
            "Sound is carried through as 16-bit PCM, not re-encoded. FFmpeg would otherwise default the .mov to 128 kbps AAC, which is a strange thing to find inside a master.",
            "Generation loss is small but real. Re-encoding ProRes repeatedly will soften; keep the first export as the master.",
        ],
    },
    FormatChoice {
        format: OutputFormat::Ffv1,
        title: "Highest quality, for keeping (FFV1)",
        note: "Mathematically lossless — the only one here that throws nothing away. Smaller than the ProRes, too.",
        per_hour: "about 35 GB an hour",
        details: &[
            "FFV1 version 3 in Matroska (.mkv), 8-bit 4:2:2 yuv422p.",
            "Mathematically lossless: decode it and you get back the exact pixels the deinterlacer produced. The only format here where re-encoding costs nothing. Measured at 71-83 Mbps on two SD captures, about 32-37 GB an hour — smaller than the ProRes, which surprises people. It is lossless but 8-bit 4:2:2, against ProRes 10-bit through a DCT. How much smaller depends on the tape: FFV1 spends its bits on whatever is actually in the picture, so a noisy VHS closes the gap and a clean transfer widens it.",
            "The preservation community's choice — FFV1 in MKV is what the Library of Congress and several national archives use for video masters, and it is an open, documented format with an independent specification (RFC 9043).",
            "Slower to encode than ProRes and much slower to scrub in an editor, because every frame is entropy-coded. Many editors will want you to transcode before cutting.",
            "Written with -level 3 -coder 1 -slicecrc 1: range coder, and a CRC per slice so bit rot is detectable rather than silent. Sound is 16-bit PCM, uncompressed.",
            "Note the bit depth: this path writes 8-bit, where ProRes writes 10. If you intend to grade heavily, ProRes HQ keeps more headroom despite being lossy — which is the real trade between these two, rather than file size.",
            "Lossless describes the encode, not the whole chain. The pipeline processes at 16-bit, so the reduction to 8 happens before FFV1 ever sees the frames, and dithering is what stops that reduction banding. FFV1 then preserves the result exactly, dither included.",
        ],
    },
    FormatChoice {
        format: OutputFormat::H264,
        title: "Best for sharing (H.264)",
        note: "Plays anywhere and small enough to send. Gives up the most quality.",
        per_hour: "about 1.5 GB an hour",
        details: &[
            "H.264 High profile in .mp4, 8-bit 4:2:0 yuv420p, x264 at CRF 18 with the slow preset.",
            "CRF 18 is near-transparent for SD tape material — you are unlikely to see the difference from the ProRes on a normal screen — but it is lossy, and the 4:2:0 chroma subsampling discards half the color resolution vertically as well as horizontally.",
            "That chroma loss matters if anyone will grade or key this later. Tape chroma is already soft and smeared; throwing away more is the one step here you cannot undo.",
            "Use it for sharing and keep one of the others as the master. Exporting H.264 from your ProRes later costs nothing extra.",
            "Sound is AAC at 192 kbps — the one format here that re-encodes it, which is the right trade for something you are going to send to someone.",
            "Like FFV1, this is written at 8-bit, so the pipeline's 16-bit frames are dithered down before encoding. Leave dithering on: x264 at CRF 18 will happily preserve a band it was given, and the places banding shows — skies, walls, fades — are exactly where its rate control spends fewest bits.",
            "The slow preset trades encode time for compression efficiency, and this is the slowest of the three despite producing much the smallest file — measured on the same ten seconds, 9.0s against 6.9s for ProRes and 4.7s for FFV1. Against the deinterlacer itself that difference is noise: QTGMC is most of the wall time whichever you pick.",
        ],
    },
];

pub fn export_page(ui: &mut egui::Ui, app: &mut App) -> PageAction {
    let mut action = PageAction::default();
    ui.set_max_width(COLUMN);

    if app.export_job.is_some() {
        running(ui, app);
        return action;
    }
    if let Some(done) = app.export_done.clone() {
        finished(ui, app, &done);
        return action;
    }

    theme::page_header(ui, "Save your video", "");
    ui.add_space(14.0);

    crate::check::section_number(ui, 1, "What will happen");
    ui.add_space(10.0);
    plan_card(ui, app);
    ui.add_space(22.0);

    crate::check::section_number(ui, 2, "What kind of file");
    ui.add_space(10.0);
    format_card(ui, app);
    ui.add_space(22.0);

    crate::check::section_number(ui, 3, "Where it goes");
    ui.add_space(10.0);
    destination_card(ui, app);
    ui.add_space(22.0);

    crate::check::section_number(ui, 4, "What's on it");
    ui.add_space(10.0);
    about_the_recording(ui, app);
    ui.add_space(22.0);

    let ready = app.file.is_some() && app.can_export();
    ui.horizontal(|ui| {
        if theme::primary(ui, "Start", ready).clicked() {
            action.start_export = true;
        }
        ui.add_space(12.0);
        if !ready {
            ui.label(
                RichText::new(app.export_blocker())
                    .size(theme::FONT_SMALL)
                    .color(theme::fix()),
            );
        }
    });

    action
}

// ── Ready ───────────────────────────────────────────────────────────────────

fn plan_card(ui: &mut egui::Ui, app: &App) {
    let inner = COLUMN - 34.0;
    theme::card().show(ui, |ui| {
        ui.set_width(inner);

        let lines = plan_lines(app);
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                ui.add_space(7.0);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                theme::inline_icon(ui, 15.0, theme::ok(), crate::theme::icon::check);
                ui.label(
                    RichText::new(line)
                        .size(theme::FONT_BODY)
                        .color(theme::text()),
                );
            });
        }

        if app.mode == Mode::JustDeinterlace {
            ui.add_space(12.0);
            ui.label(
                RichText::new("Nothing else has been changed. No sharpening, no noise removal, no color correction — the picture is as it came off the tape, minus the interlacing and the edges.")
                    .size(theme::FONT_SMALL)
                    .color(theme::faint()),
            );
        }
    });
}

/// What this export will actually do, read off the config rather than assumed.
fn plan_lines(app: &App) -> Vec<String> {
    let mut lines = Vec::new();
    let c = &app.config;

    lines.push(if c.ivtc_mode {
        "Put the original film frames back together".to_string()
    } else {
        "Remove the interlacing with QTGMC".to_string()
    });

    let st = crate::crop::effective_strips(c);
    if st.left + st.right + st.top + st.bottom + st.band > 0 {
        lines.push(match (st.left + st.right, st.band) {
            (0, band) if band > 0 => format!("Trim the {band}-line band of switching noise"),
            (sides, 0) => format!("Trim {sides} pixels of black border"),
            (sides, band) => {
                format!("Trim {sides} pixels of black border and the {band}-line noise band")
            }
        });
    }

    if c.par_correction {
        if let Some((w, h)) = app.source_size() {
            let (ow, oh) = crate::crop::output_size(c, w, h);
            lines.push(format!(
                "Correct the pixel shape, {w}×{h} in to {ow}×{oh} out"
            ));
        } else {
            lines.push("Correct the pixel shape to square pixels".to_string());
        }
    }

    if !c.trim_ranges.is_empty() {
        lines.push("Keep only the parts you chose".to_string());
    }

    lines
}

fn format_card(ui: &mut egui::Ui, app: &mut App) {
    let inner = COLUMN - 34.0;
    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        for (i, choice) in FORMATS.iter().enumerate() {
            if i > 0 {
                ui.add_space(4.0);
            }
            let selected = app.config.output_format == choice.format;
            if format_row(ui, inner, selected, choice) {
                app.config.output_format = choice.format;
            }

            // The disclosure sits under its own row rather than only under the
            // selected one, so the formats can be compared without clicking
            // through each in turn.
            ui.horizontal(|ui| {
                ui.add_space(34.0);
                let key = format!("fmt-{:?}", choice.format);
                let mut open = app.is_open(&key);
                if theme::disclosure(ui, "Details", &mut open) {
                    app.queue_open(&key, open);
                }
            });
            if app.is_open(&format!("fmt-{:?}", choice.format)) {
                ui.add_space(6.0);
                // Indented by the frame margin, not by a horizontal layout.
                // A horizontal layout offers its children unlimited width, so
                // labels inside one never wrap — which ran every line of this
                // panel off the right of the window.
                egui::Frame::NONE
                    .fill(theme::bg())
                    .corner_radius(theme::RADIUS_SMALL)
                    .inner_margin(egui::Margin {
                        left: 34,
                        right: 12,
                        top: 12,
                        bottom: 12,
                    })
                    .show(ui, |ui| {
                        ui.set_width(inner - 46.0);
                        for (j, para) in choice.details.iter().enumerate() {
                            if j > 0 {
                                ui.add_space(8.0);
                            }
                            // Wrapping stated rather than inherited, so this
                            // cannot silently revert if the parent layout
                            // changes again.
                            theme::paragraph(ui, RichText::new(*para)
                                        .size(theme::FONT_SMALL)
                                        .color(theme::muted()),);
                        }
                    });
                ui.add_space(8.0);
            }
        }
    });
}

fn format_row(
    ui: &mut egui::Ui,
    width: f32,
    selected: bool,
    choice: &FormatChoice,
) -> bool {
    let height = 52.0;
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
            egui::pos2(rect.left() + 34.0, rect.center().y - 9.0),
            egui::Align2::LEFT_CENTER,
            choice.title,
            egui::FontId::proportional(theme::FONT_BODY),
            if selected { theme::text() } else { theme::muted() },
        );
        painter.text(
            egui::pos2(rect.left() + 34.0, rect.center().y + 10.0),
            egui::Align2::LEFT_CENTER,
            format!("{} — {}", choice.note, choice.per_hour),
            egui::FontId::proportional(theme::FONT_SMALL),
            theme::faint(),
        );
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked() && !selected
}

/// Optional: what this recording actually is.
///
/// Written into the file as tags rather than beside it in a sidecar. A
/// restored tape outlives the folder it was restored in — filenames get
/// changed, sidecars get separated — but a tag travels with the file through
/// copying and re-muxing, and reads in any player or file browser.
///
/// Three fields, because three is what people reliably know about a tape
/// they have just found: what is on it, roughly when, roughly where. A
/// longer form starts asking questions nobody can answer, and an optional
/// field nobody can answer gets left blank, which teaches people to skip
/// the whole section.
///
/// Kept separate from the provenance note the app already writes. That one
/// says what this program did; these say what somebody pointed a camera at,
/// and searching for "Christmas 1998" should not have to wade through
/// deinterlacer settings.
fn about_the_recording(ui: &mut egui::Ui, app: &mut App) {
    theme::card().show(ui, |ui| {
        ui.set_width(COLUMN - 34.0);
        theme::paragraph(ui, RichText::new(
                    "All optional, and all written into the finished file so they travel with it. Leave any of them blank and nothing is written for that one.",
                )
                .size(theme::FONT_SMALL)
                .color(theme::muted()),);
        ui.add_space(12.0);

        field(
            ui,
            "What's on it",
            "Grandma Jones at the 1998 Christmas party",
            &mut app.config.meta_description,
            COLUMN - 80.0,
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            ui.vertical(|ui| {
                field(ui, "When", "1998-12-25", &mut app.config.meta_date, 170.0);
            });
            ui.vertical(|ui| {
                field(
                    ui,
                    "Where",
                    "Toronto, Canada",
                    &mut app.config.meta_location,
                    260.0,
                );
            });
        });

        // The date is the one field with a right answer, so say what it is
        // rather than rejecting what they typed after the fact.
        ui.add_space(8.0);
        let date = app.config.meta_date.trim();
        if !date.is_empty() && !looks_like_a_date(date) {
            ui.label(
                RichText::new(
                    "Dates go in as 1998, 1998-12 or 1998-12-25. Anything else is left out of the file.",
                )
                .size(theme::FONT_TINY)
                .color(theme::fix()),
            );
        } else {
            ui.label(
                RichText::new("Year is enough: 1998, 1998-12 or 1998-12-25.")
                    .size(theme::FONT_TINY)
                    .color(theme::faint()),
            );
        }
    });
}

/// One labelled single-line field.
fn field(ui: &mut egui::Ui, label: &str, hint: &str, value: &mut String, width: f32) {
    ui.label(
        RichText::new(label)
            .size(theme::FONT_SMALL)
            .color(theme::text()),
    );
    ui.add_space(3.0);
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(width),
    );
}

/// The same rule `encode::iso_date` applies, for telling the user before the
/// export rather than silently dropping the field during it.
fn looks_like_a_date(raw: &str) -> bool {
    let parts: Vec<&str> = raw.trim().split('-').collect();
    if parts.len() > 3 || !parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
        return false;
    }
    let Some(y) = parts.first().and_then(|p| p.parse::<u32>().ok()) else {
        return false;
    };
    if parts[0].len() != 4 || !(1900..=2200).contains(&y) {
        return false;
    }
    let ok = |i: usize, hi: u32| {
        parts
            .get(i)
            .is_none_or(|p| p.parse::<u32>().is_ok_and(|v| (1..=hi).contains(&v)))
    };
    ok(1, 12) && ok(2, 31)
}

fn destination_card(ui: &mut egui::Ui, app: &mut App) {
    let inner = COLUMN - 34.0;
    theme::card().show(ui, |ui| {
        ui.set_width(inner);

        let target = app.output_path();
        match &target {
            Some(path) => {
                ui.label(
                    RichText::new(file_name_of(path))
                        .size(theme::FONT_BODY)
                        .color(theme::text()),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(folder_of(path))
                        .size(theme::FONT_SMALL)
                        .color(theme::faint()),
                );
            }
            None => {
                ui.label(
                    RichText::new("No destination chosen yet.")
                        .size(theme::FONT_BODY)
                        .color(theme::muted()),
                );
            }
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if theme::secondary(ui, "Choose a different folder…").clicked() {
                app.queue_dialog(crate::dialog::Dialog::OutputFolder);
            }
        });

        // Optional companion files, beside the video and named after it.
        //
        // Off by default: most people want the video and nothing else, and a
        // folder with four files per tape is a cost paid by everybody to
        // serve the few who want the record. The settings in the video's own
        // metadata are always written and cost nothing, so the common case
        // already has provenance without any of these.
        ui.add_space(14.0);
        ui.label(
            RichText::new("Also save")
                .size(theme::FONT_SMALL)
                .strong()
                .color(theme::faint()),
        );
        ui.add_space(6.0);
        // Ordered by how often they are wanted, which is also roughly how
        // useful they are to somebody who has hit a problem: the log first,
        // the settings last.
        for (on, glyph, label, note) in [
            (
                &mut app.save_log,
                theme::glyph::DOCUMENT,
                "A log of the run",
                "What was measured, what was applied, and what the tools said while doing it. The file to send if something looks wrong.",
            ),
            (
                &mut app.save_comparison,
                theme::glyph::VIDEO,
                "A 15-second before-and-after clip",
                "The middle of the tape, the untouched capture on the left and this export on the right, both at 200% with a line down the join. No sound. It is made after the video and cannot affect it.",
            ),
            (
                &mut app.save_script,
                theme::glyph::CODE,
                "The VapourSynth script",
                "The .vpy this export actually ran, plugin loading and all. Openable in Hybrid or vsedit, comparable against a script you wrote yourself, and runnable by hand if you want to change one line.",
            ),
            (
                &mut app.save_settings,
                theme::glyph::SETTINGS,
                "The settings, as a JSON file",
                "Everything this export used, in a form the app can read back — so the next tape from the same deck can be done the same way.",
            ),
        ] {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                ui.add(egui::Checkbox::new(
                    on,
                    RichText::new("").size(theme::FONT_SMALL),
                ));
                theme::fluent_icon(ui, glyph, theme::FONT_BODY, theme::muted());
                ui.label(
                    RichText::new(label)
                        .size(theme::FONT_SMALL)
                        .color(theme::text()),
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(26.0);
                theme::paragraph(ui, RichText::new(note)
                            .size(theme::FONT_TINY)
                            .color(theme::faint()),);
            });
            ui.add_space(6.0);
        }
        theme::paragraph(ui, RichText::new(
                    "What was done is written into the video's own metadata either way, \
                     where a player or MediaInfo will show it.",
                )
                .size(theme::FONT_TINY)
                .color(theme::faint()),);

        // Nothing is ever overwritten: the name is versioned against the
        // destination folder, so a second export of the same tape becomes
        // _v01 rather than a warning the reader has to act on. This only
        // says so when the numbering has actually kicked in.
        if target
            .as_deref()
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().to_string())
            .is_some_and(|s| {
                s.rfind("_v").is_some_and(|i| {
                    s[i + 2..].len() == 2 && s[i + 2..].chars().all(|c| c.is_ascii_digit())
                })
            })
        {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 9.0;
                theme::inline_icon(ui, 15.0, theme::info(), crate::theme::icon::info);
                ui.label(
                    RichText::new("An earlier export of this video is already there, so this one is numbered. Nothing will be replaced.")
                        .size(theme::FONT_SMALL)
                        .color(theme::info()),
                );
            });
        }
    });
}

// ── Running ─────────────────────────────────────────────────────────────────

fn running(ui: &mut egui::Ui, app: &mut App) {
    let Some(job) = &app.export_job else { return };
    let inner = COLUMN - 34.0;
    let elapsed = job.started.elapsed().as_secs_f64();
    let stopping = job.stopping();
    let label = job.label();
    let fraction = job.fraction();

    theme::page_header(ui, "Working on it", "");
    ui.add_space(20.0);

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        ui.label(
            RichText::new(if stopping { "Stopping…".to_string() } else { label })
                .size(theme::FONT_SECTION)
                .color(theme::text()),
        );
        ui.add_space(14.0);
        app.progress_bar(ui, fraction, inner, theme::BAR_HEIGHT);
        ui.add_space(10.0);
        flag_control(ui, app, inner);
        ui.add_space(12.0);
        ui.label(
            RichText::new(if stopping {
                "Waiting for the encoder to let go of the file.".to_string()
            } else {
                format!(
                    "Running for {}. Enhancing a tape takes longer than the tape itself — that is QTGMC looking at every frame and its neighbors. You can leave this running.",
                    crate::worker::format_clock(elapsed)
                )
            })
            .size(theme::FONT_SMALL)
            .color(theme::faint()),
        );
    });

    ui.add_space(22.0);
    support_card(ui, inner);

    if !stopping {
        ui.add_space(22.0);
        stop_control(ui, app, elapsed);
    }

    // The bar only moves if something asks for a repaint.
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
}

/// The flag picker, folded away behind a link.
///
/// It sits under the bar it changes, where the reader is already looking and
/// has nothing else to do for the next hour — but it starts closed. The bar
/// is what the screen is for, and a dropdown parked under it reads as another
/// export setting rather than as a toy. The link says plainly that this is a
/// way to pass the time, and the panel says plainly that it changes nothing
/// about the video, because a control on an export screen is otherwise
/// assumed to.
fn flag_control(ui: &mut egui::Ui, app: &mut App, inner: f32) {
    if theme::link(
        ui,
        if app.flag_picker_open {
            "Hide the progress bar themes"
        } else {
            "Customize your progress while you wait"
        },
    )
    .clicked()
    {
        app.flag_picker_open = !app.flag_picker_open;
    }

    if !app.flag_picker_open {
        return;
    }

    ui.add_space(8.0);
    egui::Frame::NONE
        .fill(theme::panel2())
        .stroke(egui::Stroke::new(1.0, theme::line()))
        .corner_radius(theme::RADIUS)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(inner - 28.0);
            theme::paragraph(ui, RichText::new(
                        "Pick a flag theme for your progress bar. Has no effect on \
                         your final video. We think software should be useful and silly.",
                    )
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),);
            ui.add_space(10.0);
            if let Some(chosen) = crate::flags::picker(ui, &app.flag) {
                app.flag = chosen;
            }
        });
}

/// Three ways to support the project, on the one screen where the reader has
/// nothing to do and several minutes in which to do it.
///
/// Asked for by name, and it belongs here rather than on the finished page:
/// once the file exists the reader's attention is on the file.
fn support_card(ui: &mut egui::Ui, inner: f32) {
    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        ui.label(
            RichText::new("How you can support the project")
                .size(theme::FONT_SECTION)
                .color(theme::text()),
        );
        ui.add_space(10.0);

        const WAYS: [(&str, Option<&str>); 3] = [
            (
                "Write a mini-review about the software on the forums, YouTube, \
                 Facebook, or Instagram",
                None,
            ),
            (
                "Buy a cool mug or other merch",
                Some("https://my-store-1107f57.creator-spring.com/"),
            ),
            ("Leave a tip", Some("https://videocaptureguide.com/")),
        ];

        for (i, (text, link)) in WAYS.iter().enumerate() {
            if i > 0 {
                ui.add_space(8.0);
            }
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    RichText::new(format!("{}.", i + 1))
                        .size(theme::FONT_BODY)
                        .color(theme::faint()),
                );
                ui.vertical(|ui| {
                    theme::paragraph(ui, RichText::new(*text)
                                .size(theme::FONT_BODY)
                                .color(theme::text()),);
                    if let Some(url) = link {
                        ui.add_space(2.0);
                        // A real hyperlink, so it opens in the browser
                        // rather than being a URL the reader has to retype.
                        ui.hyperlink_to(
                            RichText::new(*url)
                                .size(theme::FONT_SMALL)
                                .color(theme::accent_hover()),
                            *url,
                        );
                    }
                });
            });
        }
    });
}

/// Stopping a render, behind two deliberate acts.
///
/// An export is the one thing here that takes hours, and the one thing that
/// cannot be undone by waiting: a stop throws away everything rendered so
/// far. So the control is not a button on the page — it is a quiet line of
/// text well below the progress card, and pressing it only *reveals* the
/// decision. The confirmation then puts the weight on the wrong side on
/// purpose: "Keep going" is the primary, drawn where the eye and the mouse
/// already are, and the destructive answer is the quiet one beside it.
///
/// Two separate controls, in two places, neither of them primary. A hand
/// that slips reaches a link that reveals a question, which is recoverable;
/// nothing on this page cancels an hour's work in one click.
fn stop_control(ui: &mut egui::Ui, app: &mut App, elapsed: f64) {
    if !app.export_stop_armed {
        ui.vertical_centered(|ui| {
            if theme::link(ui, "Stop this export").clicked() {
                app.export_stop_armed = true;
            }
        });
        return;
    }

    let inner = COLUMN - 34.0;
    egui::Frame::NONE
        .fill(theme::fix_soft())
        .stroke(egui::Stroke::new(1.0, theme::fix()))
        .corner_radius(theme::RADIUS)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(inner);
            ui.label(
                RichText::new("Stop and throw away what has been rendered?")
                    .size(theme::FONT_BODY)
                    .color(theme::text()),
            );
            ui.add_space(6.0);
            theme::paragraph(ui, RichText::new(format!(
                        "This export has been running for {}. Stopping deletes the \
                         part-finished file — there is no way to carry on from here, \
                         and starting again starts from the beginning.",
                        crate::worker::format_clock(elapsed)
                    ))
                    .size(theme::FONT_SMALL)
                    .color(theme::muted()),);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                // Deliberately the primary, and deliberately first: the safe
                // answer is the one a reflex click lands on.
                if theme::primary(ui, "Keep going", true).clicked() {
                    app.export_stop_armed = false;
                }
                ui.add_space(12.0);
                if theme::ghost(ui, "Yes, stop the export").clicked() {
                    if let Some(job) = &app.export_job {
                        job.stop();
                    }
                    app.export_stop_armed = false;
                }
            });
        });
}

// ── Finished ────────────────────────────────────────────────────────────────

fn finished(ui: &mut egui::Ui, app: &mut App, done: &Result<ExportSummary, String>) {
    let inner = COLUMN - 34.0;

    match done {
        Ok(summary) => {
            theme::page_header(ui, "Done", "");
            ui.add_space(20.0);
            theme::card().show(ui, |ui| {
                ui.set_width(inner);
                theme::tag(ui, Tag::LooksGood, Some("Finished"));
                ui.add_space(10.0);
                ui.label(
                    RichText::new(file_name_of(&summary.output))
                        .size(theme::FONT_SECTION)
                        .color(theme::text()),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(folder_of(&summary.output))
                        .size(theme::FONT_SMALL)
                        .color(theme::faint()),
                );
                ui.add_space(12.0);
                ui.label(
                    RichText::new(summary.describe())
                        .size(theme::FONT_BODY)
                        .color(theme::muted()),
                );

                // Named, because a file written beside the video without
                // being mentioned is one the user finds by accident later
                // and wonders about.
                if !summary.companions.is_empty() {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("Saved alongside it:")
                            .size(theme::FONT_SMALL)
                            .color(theme::faint()),
                    );
                    for p in &summary.companions {
                        ui.label(
                            RichText::new(format!("  {}", file_name_of(p)))
                                .size(theme::FONT_SMALL)
                                .color(theme::muted()),
                        );
                    }
                }
            });
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                if theme::primary(ui, "Show me the file", true).clicked() {
                    reveal(&summary.output);
                }
                ui.add_space(10.0);
                if theme::secondary(ui, "Deinterlace another video").clicked() {
                    app.reset_for_another();
                }
            });
        }
        // Stopping is not a failure, and the page that says so must not look
        // like the one that reports a crash. No error text, no "quote this
        // to support" — the reader did this on purpose and knows why.
        Err(why) if why == vcg_tools::encode::STOPPED => {
            theme::page_header(ui, "Stopped", "");
            ui.add_space(20.0);
            theme::card().show(ui, |ui| {
                ui.set_width(inner);
                theme::tag(ui, Tag::Info, Some("Stopped"));
                ui.add_space(10.0);
                ui.label(
                    RichText::new("You stopped this export")
                        .size(theme::FONT_SECTION)
                        .color(theme::text()),
                );
                ui.add_space(8.0);
                theme::paragraph(ui, RichText::new(
                            "The part-finished file has been removed, so there is \
                             nothing half-done left on disk. Your settings are all \
                             still here — starting again picks up exactly where you \
                             left off, from the beginning of the render.",
                        )
                        .size(theme::FONT_BODY)
                        .color(theme::muted()),);
            });
            ui.add_space(18.0);
            if theme::primary(ui, "Start again", true).clicked() {
                app.export_done = None;
            }
        }
        Err(why) => {
            theme::page_header(ui, "It did not finish", "");
            ui.add_space(20.0);
            theme::card().show(ui, |ui| {
                ui.set_width(inner);
                theme::tag(ui, Tag::Info, Some("Stopped"));
                ui.add_space(10.0);
                ui.label(
                    RichText::new("Your video was not saved")
                        .size(theme::FONT_SECTION)
                        .color(theme::text()),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Nothing on your computer has been changed. The reason is below — it is written for a developer rather than for you, but it is the useful thing to quote if you ask for help.")
                        .size(theme::FONT_BODY)
                        .color(theme::muted()),
                );
                ui.add_space(10.0);
                theme::details_text(ui, why);
            });
            ui.add_space(18.0);
            if theme::primary(ui, "Try again", true).clicked() {
                app.export_done = None;
            }
        }
    }
}

/// What the finished export produced, kept so the page can describe it after
/// the job itself is gone.
#[derive(Debug, Clone)]
pub struct ExportSummary {
    pub output: PathBuf,
    pub duration: f64,
    pub had_audio: bool,
    pub took: f64,
    pub bytes: u64,
    /// The log and settings files written beside it, if either was asked for.
    /// What was actually written, not what was requested — an unwritable
    /// folder must not produce a page claiming files that are not there.
    pub companions: Vec<PathBuf>,
}

impl ExportSummary {
    fn describe(&self) -> String {
        let sound = if self.had_audio {
            "with its sound"
        } else {
            "with no sound track (the source had none)"
        };
        format!(
            "{} long, {sound}. {} on disk. Took {}.",
            crate::worker::format_clock(self.duration),
            human_size(self.bytes),
            crate::worker::format_rough(self.took)
        )
    }
}

fn human_size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", b / MB)
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn folder_of(path: &Path) -> String {
    path.parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// Open the containing folder with the file selected.
///
/// `/select,` has to be passed as one raw argument. Rust quotes any argument
/// containing a space, which yields `explorer "/select,D:\My Tapes\out.mov"`
/// — Explorer cannot parse that, gives up, and opens Documents instead. That
/// is what it did. `raw_arg` hands the command line over verbatim, with the
/// path quoted *inside* the switch where Explorer expects it.
///
/// Failure is silent on purpose: the file is saved and its location is on
/// screen, so a message about the file manager would be noise.
fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("explorer");
        cmd.raw_arg(format!("/select,\"{}\"", path.display()));
        let _ = cmd.spawn();
    }
    #[cfg(not(windows))]
    {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

/// The default output path for a source file: alongside it, renamed, never
/// overwriting the source itself.
#[cfg(test)]
pub fn default_output(source: &Path, config: &Config) -> PathBuf {
    let dir = source.parent().map(Path::to_path_buf).unwrap_or_default();
    output_in(&dir, source, config)
}

/// The output path inside `dir`, versioned against what is already there.
///
/// Takes the directory explicitly because the collision has to be checked
/// where the file will actually be written. Checking it beside the source and
/// then moving the name to a folder the user chose would hand back a name
/// that is free in the wrong place.
pub fn output_in(dir: &Path, source: &Path, config: &Config) -> PathBuf {
    let ext = vcg_core::ffmpeg::output_args(config).extension;
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "video".to_string());
    next_free(dir, &format!("{stem}{}", version_suffix()), ext)
}

/// The first name in the series that is not taken.
///
/// `name.mov`, then `name_v01.mov`, `name_v02.mov` and so on. A second export
/// of the same tape is the normal case, not the exception — a clean master
/// and then a watermarked copy, or the same tape again with the noise setting
/// changed — and the old behavior was to offer to overwrite the first one
/// and warn about it. A warning puts the work of not losing a render onto the
/// reader; a free filename removes the question.
///
/// Capped: at a hundred the numbering gives up and returns the plain name
/// rather than searching forever on a directory that cannot be written to.
fn next_free(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let plain = dir.join(format!("{stem}{ext}"));
    if !plain.exists() {
        return plain;
    }
    for n in 1..100 {
        let candidate = dir.join(format!("{stem}_v{n:02}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    plain
}

/// The suffix stamped on every output: the app's initials and the version
/// digits.
///
/// Worth having in the filename rather than only in metadata. These files
/// outlive the tape and often the machine, and a year later the one thing
/// you want to know about a restored capture is what made it and which
/// build — particularly if a release turns out to have had a bug in it.
///
/// Pre-release tags are dropped and the dots removed: 2.0.0-dev becomes
/// _VCGD200.
pub fn version_suffix() -> String {
    let digits: String = crate::VERSION
        .split('-')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    format!("_VCGD{digits}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_output_sits_beside_the_source_and_never_on_it() {
        let source = Path::new(r"D:\tapes\Tape 01.avi");
        let config = Config {
            output_format: OutputFormat::Prores,
            ..Config::default()
        };
        let out = default_output(source, &config);
        assert_eq!(out.parent(), source.parent());
        assert_ne!(out, source);
        assert_eq!(
            out.file_name().unwrap().to_string_lossy(),
            format!("Tape 01{}.mov", version_suffix())
        );
    }

    /// The extension has to follow the chosen format, or the container and the
    /// codec disagree and ffmpeg writes something unplayable.
    #[test]
    fn the_extension_follows_the_format() {
        let source = Path::new("tape.avi");
        for (format, want) in [
            (OutputFormat::Prores, "mov"),
            (OutputFormat::H264, "mp4"),
            (OutputFormat::Ffv1, "mkv"),
        ] {
            let config = Config {
                output_format: format,
                ..Config::default()
            };
            let out = default_output(source, &config);
            assert_eq!(
                out.extension().unwrap().to_string_lossy(),
                want,
                "{format:?}"
            );
        }
    }

    #[test]
    fn a_source_with_no_folder_still_produces_a_path() {
        let out = default_output(Path::new("clip.avi"), &Config::default());
        assert_eq!(
            out.file_name().unwrap().to_string_lossy(),
            format!("clip{}.mov", version_suffix())
        );
    }

    /// The suffix is part of how a finished file is identified a year later,
    /// so its shape is pinned rather than left to whatever the version
    /// string happens to look like.
    #[test]
    fn the_version_suffix_is_vcgd_plus_digits() {
        let s = version_suffix();
        assert!(s.starts_with("_VCGD"), "{s}");
        assert!(
            s[5..].chars().all(|c| c.is_ascii_digit()),
            "no dots or pre-release tags: {s}"
        );
        assert!(s.len() > 5, "there has to be a version in it: {s}");
    }

    #[test]
    fn sizes_read_as_people_write_them() {
        assert_eq!(human_size(900 * 1024 * 1024), "900 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024 + 512 * 1024 * 1024), "3.5 GB");
    }

    #[test]
    fn the_finished_summary_says_whether_there_was_sound() {
        let mut s = ExportSummary {
            companions: Vec::new(),
            output: PathBuf::from("out.mov"),
            duration: 125.0,
            had_audio: true,
            took: 300.0,
            bytes: 1024 * 1024 * 1024,
        };
        assert!(s.describe().contains("2:05"));
        assert!(s.describe().contains("with its sound"));
        s.had_audio = false;
        assert!(s.describe().contains("no sound"));
    }
}

#[cfg(test)]
mod versioning_tests {
    use super::*;
    use vcg_core::config::{Config, OutputFormat};

    /// A second export of the same tape must not offer to overwrite the
    /// first. The user asked for this directly: number it instead of warning
    /// about it, so there is no question to answer and no render to lose.
    #[test]
    fn a_second_export_is_numbered_rather_than_overwriting() {
        let dir = std::env::temp_dir().join(format!("vcg-ver-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let source = dir.join("Tape 01.avi");
        let config = Config {
            output_format: OutputFormat::Prores,
            ..Config::default()
        };

        // Nothing there yet: the plain name.
        let first = output_in(&dir, &source, &config);
        assert_eq!(
            first.file_name().unwrap().to_string_lossy(),
            format!("Tape 01{}.mov", version_suffix())
        );

        // With that file present, the next one is _v01, then _v02.
        std::fs::write(&first, b"").expect("write");
        let second = output_in(&dir, &source, &config);
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            format!("Tape 01{}_v01.mov", version_suffix())
        );

        std::fs::write(&second, b"").expect("write");
        let third = output_in(&dir, &source, &config);
        assert_eq!(
            third.file_name().unwrap().to_string_lossy(),
            format!("Tape 01{}_v02.mov", version_suffix())
        );

        // And it never hands back a name that is already taken.
        for p in [&first, &second, &third] {
            assert!(p != &third || !p.exists(), "the chosen name must be free");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The numbering is checked in the folder the file will land in, not
    /// beside the source. A name that is free in the wrong place is no use.
    #[test]
    fn the_collision_is_checked_where_the_file_will_be_written() {
        let base = std::env::temp_dir().join(format!("vcg-ver2-{}", std::process::id()));
        let from = base.join("from");
        let to = base.join("to");
        std::fs::create_dir_all(&from).expect("dir");
        std::fs::create_dir_all(&to).expect("dir");
        let source = from.join("clip.avi");
        let config = Config {
            output_format: OutputFormat::Prores,
            ..Config::default()
        };

        // Taken in the destination, free beside the source.
        std::fs::write(to.join(format!("clip{}.mov", version_suffix())), b"").expect("write");
        let out = output_in(&to, &source, &config);
        assert_eq!(
            out.file_name().unwrap().to_string_lossy(),
            format!("clip{}_v01.mov", version_suffix())
        );
        assert_eq!(out.parent(), Some(to.as_path()));

        let _ = std::fs::remove_dir_all(&base);
    }
}
