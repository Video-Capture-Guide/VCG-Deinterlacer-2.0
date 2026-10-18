//! The Watermark screen.
//!
//! Burning a name or a logo into the picture. Permanent, which is the whole
//! point and also the risk — so the page says so plainly and defaults to off.
//!
//! The logo path was, until recently, a trap: it uses `-filter_complex`, the
//! audio was deferred to a second mux, and that mux was never written. A logo
//! watermark produced a video with no sound and reported success. The encoder
//! now labels the filter output and maps the audio alongside it, so both
//! kinds work; see `encode::encode_command`.

use eframe::egui::{self, RichText};

use vcg_core::config::{Config, Watermark, WatermarkPosition};

use crate::check::COLUMN;
use crate::pages::PageAction;
use crate::theme::{self, Tag};
use crate::worker::PreviewKind;
use crate::App;

/// The three kinds, as radio rows.
const KINDS: [(&str, &str); 3] = [
    ("No watermark", "The picture is left alone."),
    ("Some text", "A name, a date, a channel — burnt into every frame."),
    ("An image", "A PNG logo, with transparency respected."),
];

const POSITIONS: [(WatermarkPosition, &str); 5] = [
    (WatermarkPosition::BottomRight, "Bottom right"),
    (WatermarkPosition::BottomLeft, "Bottom left"),
    (WatermarkPosition::TopRight, "Top right"),
    (WatermarkPosition::TopLeft, "Top left"),
    (WatermarkPosition::Center, "Middle"),
];

/// Which radio row the config is on.
fn kind_index(c: &Config) -> usize {
    match c.watermark {
        Watermark::None => 0,
        Watermark::Text { .. } => 1,
        Watermark::Logo { .. } => 2,
    }
}

pub fn watermark_page(ui: &mut egui::Ui, app: &mut App) -> PageAction {
    let action = PageAction::default();
    ui.set_max_width(COLUMN);
    let inner = COLUMN - 34.0;

    theme::page_header(
        ui,
        "Watermark",
        "Burn a name or a logo into the picture. This cannot be removed afterwards.",
    );
    ui.add_space(20.0);

    let kind = kind_index(&app.config);
    // Set inside the borrow of `app.config.watermark` below and acted on
    // after it, because asking for a dialog needs `app` itself.
    let mut pick_logo = false;

    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        for (i, (title, note)) in KINDS.iter().enumerate() {
            if crate::settings::radio(ui, inner, i == kind, title, note) {
                app.config.watermark = match i {
                    1 => Watermark::Text {
                        text: String::new(),
                        fontsize: 24,
                    },
                    2 => Watermark::Logo {
                        logo_path: String::new(),
                        logo_size: 0.15,
                    },
                    _ => Watermark::None,
                };
            }
        }
    });

    if kind == 0 {
        return action;
    }

    ui.add_space(18.0);
    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        match &mut app.config.watermark {
            Watermark::Text { text, fontsize } => {
                ui.label(
                    RichText::new("What it should say")
                        .size(theme::FONT_BODY)
                        .color(theme::text()),
                );
                ui.add_space(6.0);
                let edit = egui::TextEdit::singleline(text)
                    .desired_width(inner - 20.0)
                    .hint_text("Smith family, 1998")
                    .show(ui);
                ui.add_space(8.0);
                accent_picker(ui, text, &edit, inner);
                ui.add_space(14.0);
                ui.label(
                    RichText::new("Size")
                        .size(theme::FONT_BODY)
                        .color(theme::text()),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for (label, size) in [("Small", 16u32), ("Medium", 24), ("Large", 36)] {
                        let on = *fontsize == size;
                        if theme::chip_button(ui, label, on) {
                            *fontsize = size;
                        }
                    }
                });
            }
            Watermark::Logo {
                logo_path,
                logo_size,
            } => {
                let chosen = !logo_path.is_empty();
                ui.label(
                    RichText::new(if chosen {
                        logo_path.as_str()
                    } else {
                        "No image chosen yet."
                    })
                    .size(theme::FONT_BODY)
                    .color(if chosen { theme::text() } else { theme::muted() }),
                );
                ui.add_space(10.0);
                if theme::secondary(ui, "Choose an image…").clicked() {
                    // Queued, not opened: a dialog needs the window as its
                    // owner or it can come up behind the app. See `dialog`.
                    pick_logo = true;
                }
                if chosen && !std::path::Path::new(logo_path.as_str()).exists() {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("That file is not there any more. The watermark will be skipped.")
                            .size(theme::FONT_SMALL)
                            .color(theme::fix()),
                    );
                }
                ui.add_space(14.0);
                ui.label(
                    RichText::new("Size, as a share of the picture width")
                        .size(theme::FONT_BODY)
                        .color(theme::text()),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for (label, size) in [("Small", 0.08f64), ("Medium", 0.15), ("Large", 0.25)] {
                        let on = (*logo_size - size).abs() < 0.001;
                        if theme::chip_button(ui, label, on) {
                            *logo_size = size;
                        }
                    }
                });
            }
            Watermark::None => {}
        }
    });

    // ── What it looks like ──────────────────────────────────────────────────
    //
    // Rendered at the finished file size, because that is what decides where
    // a corner lands and how big the text is. Everything that can change the
    // picture is in the tag, so moving a control re-renders and a late
    // arrival for a setting already changed is dropped.
    let tag = preview_tag(&app.config);
    app.request_setting_preview(tag.clone(), PreviewKind::Watermark);

    ui.add_space(18.0);
    crate::check::section_number(ui, 1, "What it will look like");
    ui.add_space(10.0);
    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        crate::settings::preview_picture(ui, app, &tag, inner);
    });

    // ── Where, and how strong ───────────────────────────────────────────────
    ui.add_space(18.0);
    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        ui.label(
            RichText::new("Where it goes")
                .size(theme::FONT_BODY)
                .color(theme::text()),
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for (pos, label) in POSITIONS {
                let on = app.config.wm_position == pos;
                if theme::chip_button(ui, label, on) {
                    app.config.wm_position = pos;
                }
            }
        });

        ui.add_space(16.0);
        ui.label(
            RichText::new("How strong")
                .size(theme::FONT_BODY)
                .color(theme::text()),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (label, value) in [("Faint", 0.3f64), ("Normal", 0.6), ("Solid", 1.0)] {
                let on = (app.config.wm_opacity - value).abs() < 0.001;
                if theme::chip_button(ui, label, on) {
                    app.config.wm_opacity = value;
                }
            }
        });
    });

    ui.add_space(18.0);
    theme::card().show(ui, |ui| {
        ui.set_width(inner);
        theme::tag(ui, Tag::Info, Some("Worth knowing"));
        ui.add_space(8.0);
        theme::paragraph(ui, RichText::new("A watermark is burnt into the pixels. There is no later step that removes it, so if this is the only copy you are keeping, consider exporting a clean master first and a watermarked version second — the second export costs only the encode, since the analysis is already done.")
                    .size(theme::FONT_BODY)
                    .color(theme::muted()),);
    });

    ui.add_space(16.0);
    details(ui, app);

    if pick_logo {
        app.queue_dialog(crate::dialog::Dialog::Logo);
    }

    action
}

/// Characters that cannot be typed, offered as buttons.
///
/// Accented letters do not reach the field on a keyboard that enters them
/// with AltGr. Windows reports AltGr as Ctrl+Alt, and the layer below this
/// one discards typed text whenever Ctrl is held — it cannot tell an AltGr
/// composition from someone pressing Ctrl+S. Dead-key layouts fare no better.
///
/// Nothing is wrong with the font or with the burn-in. The bundled font
/// carries every one of these, and FFmpeg renders them correctly with the
/// font already chosen — checked by rendering "José Müller 1998" through the
/// real filter, which is how the fault was narrowed to the keyboard.
fn accent_picker(
    ui: &mut egui::Ui,
    text: &mut String,
    edit: &egui::text_edit::TextEditOutput,
    inner: f32,
) {
    const ROWS: [&str; 2] = [
        "àáâäãåæçèéêëìíîïñòóôöõøùúûüß",
        "ÀÁÂÄÃÅÆÇÈÉÊËÌÍÎÏÑÒÓÔÖÕØÙÚÛÜ",
    ];

    // Where the caret is, in characters. egui counts characters and the
    // string is indexed in bytes; mixing the two splits a multi-byte
    // character and panics — and an accented name is made of those.
    let caret = edit
        .cursor_range
        .map(|c| c.primary.index.0)
        .unwrap_or_else(|| text.chars().count());

    let mut insert: Option<char> = None;
    ui.vertical(|ui| {
        ui.set_width(inner - 20.0);
        for row in ROWS {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
                for ch in row.chars() {
                    if theme::chip_button(ui, &ch.to_string(), false) {
                        insert = Some(ch);
                    }
                }
            });
        }
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "Accented letters often cannot be typed into a window like this one, so they are here instead. Pasting works too.",
        )
        .size(theme::FONT_TINY)
        .color(theme::faint()),
    );

    if let Some(ch) = insert {
        let at = text
            .char_indices()
            .nth(caret)
            .map(|(b, _)| b)
            .unwrap_or(text.len());
        text.insert(at, ch);

        // The caret goes after what was just inserted. Without this every
        // character lands in front of the last one and the name comes out
        // backwards.
        let mut state = edit.state.clone();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(caret + 1),
        )));
        state.store(ui.ctx(), edit.response.id);
        edit.response.request_focus();
    }
}

/// Everything that changes what the watermark looks like, as one string.
///
/// The preview is keyed on this, so any control that moves invalidates it.
/// Missing a field here would leave a stale picture on screen claiming to be
/// the current setting.
fn preview_tag(c: &Config) -> String {
    match &c.watermark {
        Watermark::None => "wm-none".to_string(),
        Watermark::Text { text, fontsize } => format!(
            "wm-text-{text}-{fontsize}-{:?}-{:.2}",
            c.wm_position, c.wm_opacity
        ),
        Watermark::Logo {
            logo_path,
            logo_size,
        } => format!(
            "wm-logo-{logo_path}-{logo_size:.3}-{:?}-{:.2}",
            c.wm_position, c.wm_opacity
        ),
    }
}

fn details(ui: &mut egui::Ui, app: &mut App) {
    let mut open = app.is_open("wm-details");
    if theme::disclosure(ui, "Details", &mut open) {
        app.set_open("wm-details", open);
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
            ui.set_width(COLUMN - 58.0);
            for para in [
                "Text is drawn with FFmpeg's drawtext at the chosen point size, white on a black box at 30% opacity, inset 20 pixels from the edges. The font is found on the system rather than bundled; if none can be found the watermark is skipped rather than guessed at.",
                "An image goes through -filter_complex: scaled to the chosen share of the output width, converted to RGBA, its alpha multiplied by the strength setting, then overlaid. Transparency in the PNG is respected, so a logo with a soft edge keeps it.",
                "Both are applied after everything else — after the deinterlace, the crop, the PAR correction and any upscale — so the watermark is drawn at output resolution and stays sharp rather than being scaled up with the picture.",
                "The image path is read at export time, not copied. Moving or deleting the file between setting it here and exporting means the watermark is skipped; the page says so if the file is already gone.",
                "Until recently a logo watermark produced a file with no sound: the overlay graph left its output unlabeled, the audio was deferred to a second mux, and that mux was never written. The graph output is now labeled and the audio mapped alongside it.",
            ] {
                theme::paragraph(ui, RichText::new(para)
                            .size(theme::FONT_SMALL)
                            .color(theme::muted()),);
                ui.add_space(8.0);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_reads_back_from_the_config() {
        assert_eq!(kind_index(&Config::default()), 0);
        let text = Config {
            watermark: Watermark::Text {
                text: "hi".into(),
                fontsize: 24,
            },
            ..Config::default()
        };
        assert_eq!(kind_index(&text), 1);
        let logo = Config {
            watermark: Watermark::Logo {
                logo_path: "a.png".into(),
                logo_size: 0.15,
            },
            ..Config::default()
        };
        assert_eq!(kind_index(&logo), 2);
    }

    /// Off by default, and the strength is visible rather than zero — an
    /// opacity of 0 would draw a watermark nobody can see and leave the user
    /// hunting for why.
    #[test]
    fn the_defaults_are_off_but_usable() {
        let c = Config::default();
        assert_eq!(c.watermark, Watermark::None);
        assert!(c.wm_opacity > 0.1, "{}", c.wm_opacity);
        assert_eq!(c.wm_position, WatermarkPosition::BottomRight);
    }

    /// Every strength the page offers must be one the config can hold and
    /// the page can read back, or a button would never light up.
    #[test]
    fn every_offered_strength_is_selectable() {
        for value in [0.3f64, 0.6, 1.0] {
            let c = Config {
                wm_opacity: value,
                ..Config::default()
            };
            assert!((c.wm_opacity - value).abs() < 0.001);
        }
    }
}
