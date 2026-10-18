//! Flags, as the fill of a progress bar.
//!
//! An export runs for hours and there is nothing to do while it does. The
//! bar is the only thing moving on the screen, so it may as well be worth
//! looking at — and a flag fills left to right on its own, which is exactly
//! the shape a progress bar already is.
//!
//! Every flag is one entry in [`FLAGS`], and adding another is adding a row.
//! There are two kinds and the difference is only in where the picture comes
//! from:
//!
//! * **Stripes** are drawn from color constants. No asset, no decoding, and
//!   they are the flags a progress bar suits best — a horizontal tricolour
//!   in a wide bar reads as itself at any size.
//! * **Emblems** come from a bundled PNG, because a maple leaf or a taegeuk
//!   is not a list of bands. They are fitted to the bar's aspect without
//!   distortion, which in a bar this wide means a centered horizontal slice:
//!   squashing a coat of arms to a 40:1 ratio would be worse than cropping
//!   it.
//!
//! The bands carry weights because not every flag divides evenly — Colombia
//! is a half and two quarters — and a Colombian flag drawn in thirds is
//! simply the wrong flag.

use std::collections::HashMap;

use eframe::egui::{self, Color32, Rect};

use crate::theme::{self, hex};

/// Which way the bands run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}

/// Where a flag's picture comes from.
pub enum Design {
    /// Bands of color, with relative weights. A weight of 1 everywhere is
    /// equal bands.
    Stripes {
        direction: Direction,
        bands: &'static [(Color32, u8)],
    },
    /// A bundled PNG, embedded in the binary.
    Emblem(&'static [u8]),
}

pub struct Flag {
    /// What the picker calls it, and what is stored in the preferences.
    pub name: &'static str,
    pub design: Design,
}

/// Equal bands, which is almost all of them.
const fn equal(colors: &'static [(Color32, u8)]) -> &'static [(Color32, u8)] {
    colors
}

macro_rules! horizontal {
    ($name:literal, $($color:literal),+ $(,)?) => {
        Flag {
            name: $name,
            design: Design::Stripes {
                direction: Direction::Horizontal,
                bands: equal(&[$((hex($color), 1)),+]),
            },
        }
    };
}

macro_rules! vertical {
    ($name:literal, $($color:literal),+ $(,)?) => {
        Flag {
            name: $name,
            design: Design::Stripes {
                direction: Direction::Vertical,
                bands: equal(&[$((hex($color), 1)),+]),
            },
        }
    };
}

/// Every flag on offer, in the order the picker lists them.
///
/// Stripes first, grouped by direction, then the emblems. Alphabetical
/// within each group would scatter the ones that look alike; grouped by how
/// they are drawn keeps the list legible while scrolling.
pub const FLAGS: &[Flag] = &[
    // ── Horizontal stripes ──────────────────────────────────────────────
    horizontal!("Germany", 0x000000, 0xDD0000, 0xFFCE00),
    horizontal!("Netherlands", 0xAE1C28, 0xFFFFFF, 0x21468B),
    horizontal!("Ukraine", 0x0057B7, 0xFFD700),
    horizontal!("Russia", 0xFFFFFF, 0x0039A6, 0xD52B1E),
    horizontal!("Austria", 0xED2939, 0xFFFFFF, 0xED2939),
    horizontal!("Hungary", 0xCD2A3E, 0xFFFFFF, 0x436F4D),
    // Colombia is the reason bands carry weights: a half and two quarters,
    // not three thirds.
    Flag {
        name: "Colombia",
        design: Design::Stripes {
            direction: Direction::Horizontal,
            bands: &[
                (hex(0xFCD116), 2),
                (hex(0x003893), 1),
                (hex(0xCE1126), 1),
            ],
        },
    },
    horizontal!("Estonia", 0x0072CE, 0x000000, 0xFFFFFF),
    horizontal!("Lithuania", 0xFDB913, 0x006A44, 0xC1272D),
    horizontal!("Bulgaria", 0xFFFFFF, 0x00966E, 0xD62612),
    horizontal!("Poland", 0xFFFFFF, 0xDC143C),
    horizontal!(
        "Rainbow",
        0xE40303,
        0xFF8C00,
        0xFFED00,
        0x008026,
        0x24408E,
        0x732982,
    ),
    // ── Vertical stripes ────────────────────────────────────────────────
    vertical!("France", 0x002395, 0xFFFFFF, 0xED2939),
    vertical!("Italy", 0x008C45, 0xF4F5F0, 0xCD212A),
    vertical!("Ireland", 0x169B62, 0xFFFFFF, 0xFF883E),
    vertical!("Belgium", 0x000000, 0xFAE042, 0xED2939),
    vertical!("Romania", 0x002B7F, 0xFCD116, 0xCE1126),
    vertical!("Nigeria", 0x008751, 0xFFFFFF, 0x008751),
    // ── Emblems ─────────────────────────────────────────────────────────
    Flag {
        name: "Canada",
        design: Design::Emblem(include_bytes!("../assets/flags/canada.png")),
    },
    Flag {
        name: "Quebec",
        design: Design::Emblem(include_bytes!("../assets/flags/quebec.png")),
    },
    Flag {
        name: "Ontario",
        design: Design::Emblem(include_bytes!("../assets/flags/ontario.png")),
    },
    // Fifty stars in the canton: a list of bands this is not.
    Flag {
        name: "United States",
        design: Design::Emblem(include_bytes!("../assets/flags/usa.png")),
    },
    Flag {
        name: "United Kingdom",
        design: Design::Emblem(include_bytes!("../assets/flags/united-kingdom.png")),
    },
    Flag {
        name: "Philippines",
        design: Design::Emblem(include_bytes!("../assets/flags/philippines.png")),
    },
    // Three horizontal bands behind a vertical one at the hoist, which is
    // neither of the directions `Stripes` can draw.
    Flag {
        name: "United Arab Emirates",
        design: Design::Emblem(include_bytes!("../assets/flags/uae.png")),
    },
    Flag {
        name: "South Korea",
        design: Design::Emblem(include_bytes!("../assets/flags/south-korea.png")),
    },
    Flag {
        name: "India",
        design: Design::Emblem(include_bytes!("../assets/flags/india.png")),
    },
    Flag {
        name: "Taiwan",
        design: Design::Emblem(include_bytes!("../assets/flags/taiwan.png")),
    },
    Flag {
        name: "Israel",
        design: Design::Emblem(include_bytes!("../assets/flags/israel.png")),
    },
    Flag {
        name: "Greece",
        design: Design::Emblem(include_bytes!("../assets/flags/greece.png")),
    },
    Flag {
        name: "Japan",
        design: Design::Emblem(include_bytes!("../assets/flags/japan.png")),
    },
    // The full flag, coat of arms and all. A Mexican flag drawn as three
    // bands is an Italian flag in the wrong colors.
    Flag {
        name: "Mexico",
        design: Design::Emblem(include_bytes!("../assets/flags/mexico.png")),
    },
    Flag {
        name: "Iran (Lion and Sun)",
        design: Design::Emblem(include_bytes!("../assets/flags/iran-lion-sun.png")),
    },
];

/// What the picker calls the plain purple bar. Not a flag, and stored as an
/// empty preference rather than as a name, so a future rename cannot orphan
/// somebody's setting.
pub const PLAIN: &str = "No flag — plain purple";

/// The flag with this name, if there is one.
pub fn find(name: &str) -> Option<&'static Flag> {
    FLAGS.iter().find(|f| f.name == name)
}

/// Decoded flag images, kept for as long as the app runs.
///
/// Lazy, because most of these are never looked at in a given session and
/// ten textures is several megabytes of video memory to upload for a
/// decoration. A failed decode is cached as `None` so a corrupt asset is
/// tried once rather than every frame.
#[derive(Default)]
pub struct Textures(HashMap<&'static str, Option<egui::TextureHandle>>);

impl Textures {
    fn get(&mut self, ctx: &egui::Context, flag: &'static Flag) -> Option<&egui::TextureHandle> {
        let Design::Emblem(bytes) = flag.design else {
            return None;
        };
        self.0
            .entry(flag.name)
            .or_insert_with(|| decode(ctx, flag.name, bytes))
            .as_ref()
    }
}

/// Decode one bundled PNG into a texture.
///
/// Returns `None` on any failure, and the caller falls back to the plain
/// purple bar without saying anything. A progress bar is not the place to
/// report that a decoration did not load.
/// Decode a bundled PNG into a texture.
///
/// Shared with the front page's workflow diagram, which is a bundled PNG
/// with the same needs — one decoder, one set of transformations, one place
/// a corrupt asset is handled.
pub fn decode_png(
    ctx: &egui::Context,
    name: &str,
    bytes: &[u8],
) -> Option<egui::TextureHandle> {
    decode(ctx, name, bytes)
}

fn decode(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<egui::TextureHandle> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes));
    // The assets are a mix of palette and RGBA, straight from Commons.
    // Normalizing here means the drawing code sees one format and the
    // assets can be replaced without touching it.
    reader.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = reader.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);

    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => buf[..info.buffer_size()]
            .iter()
            .flat_map(|v| [*v, *v, *v, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf[..info.buffer_size()]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        // normalize_to_color8 expands a palette, so this is unreachable in
        // practice and is not worth a panic if it ever is not.
        png::ColorType::Indexed => return None,
    };
    if rgba.len() != w * h * 4 {
        return None;
    }

    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
    Some(ctx.load_texture(
        format!("flag-{name}"),
        image,
        egui::TextureOptions::LINEAR,
    ))
}

/// A progress bar whose fill is a flag.
///
/// The flag is always drawn at the *whole* bar's size and then clipped to
/// the finished part. Scaling it to the finished part instead would make a
/// French tricolour at 10% a thin blue bar, at 50% a blue-white one, and at
/// no point the flag — and a bar that redraws its contents at a new size
/// every frame is a bar that never looks still.
pub fn progress_bar(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    flag: Option<&'static Flag>,
    fraction: f32,
    width: f32,
    height: f32,
) -> Rect {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return rect;
    }
    let fraction = fraction.clamp(0.0, 1.0);
    // Small, deliberately. The flag inside is clipped to a rectangle, so a
    // large radius would show square corners poking out of a round track.
    let radius = 3.0;

    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::panel2());
    if fraction <= 0.0 {
        return rect;
    }

    let filled = Rect::from_min_size(
        rect.min,
        egui::vec2((rect.width() * fraction).round(), rect.height()),
    );

    match flag.map(|f| &f.design) {
        None => {
            painter.rect_filled(filled, radius, theme::accent());
        }
        Some(Design::Stripes { direction, bands }) => {
            let clipped = painter.with_clip_rect(filled);
            paint_stripes(&clipped, rect, *direction, bands);
        }
        Some(Design::Emblem(_)) => {
            let flag = flag.expect("matched Some");
            let ctx = ui.ctx().clone();
            let texture = textures.get(&ctx, flag).cloned();
            match texture {
                Some(texture) => {
                    let clipped = ui.painter().with_clip_rect(filled);
                    paint_emblem(&clipped, rect, &texture);
                }
                // Silently plain, as specified. A missing decoration must
                // not become a message.
                None => {
                    ui.painter().rect_filled(filled, radius, theme::accent());
                }
            }
        }
    }
    rect
}

/// Bands across the whole bar, split by weight.
fn paint_stripes(
    painter: &egui::Painter,
    rect: Rect,
    direction: Direction,
    bands: &[(Color32, u8)],
) {
    let total: f32 = bands.iter().map(|(_, w)| *w as f32).sum();
    if total <= 0.0 {
        return;
    }
    // Accumulated in band space and rounded at each edge rather than
    // stepping by a rounded width, so rounding error cannot open a
    // background-colored seam between two bands.
    let span = match direction {
        Direction::Horizontal => rect.height(),
        Direction::Vertical => rect.width(),
    };
    let mut walked = 0.0f32;
    for (color, weight) in bands {
        let start = (walked / total * span).round();
        walked += *weight as f32;
        let end = (walked / total * span).round();
        let band = match direction {
            Direction::Horizontal => Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + start),
                egui::pos2(rect.right(), rect.top() + end),
            ),
            Direction::Vertical => Rect::from_min_max(
                egui::pos2(rect.left() + start, rect.top()),
                egui::pos2(rect.left() + end, rect.bottom()),
            ),
        };
        painter.rect_filled(band, 0.0, *color);
    }
}

/// The image across the whole bar, undistorted and centered.
///
/// A bar is far wider than any flag, so this crops the top and the bottom
/// and shows a band through the middle. That is the trade the brief asks
/// for: a squashed emblem is unrecognizable, a cropped one is still the
/// right colors in the right places.
fn paint_emblem(painter: &egui::Painter, rect: Rect, texture: &egui::TextureHandle) {
    let size = texture.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let (u_span, v_span) = cover_uv(size.x / size.y, rect.width() / rect.height());
    let uv = Rect::from_min_size(
        egui::pos2((1.0 - u_span) / 2.0, (1.0 - v_span) / 2.0),
        egui::vec2(u_span, v_span),
    );
    painter.image(texture.id(), rect, uv, Color32::WHITE);
}

/// How much of the image is visible when it is scaled to cover the bar.
///
/// Returned as a fraction of the image's width and height, so the caller can
/// turn it straight into texture coordinates.
fn cover_uv(image_aspect: f32, bar_aspect: f32) -> (f32, f32) {
    if image_aspect > bar_aspect {
        // Wider than the bar: the sides go.
        ((bar_aspect / image_aspect).clamp(0.0, 1.0), 1.0)
    } else {
        // Taller than the bar, which is the usual case here by a long way.
        (1.0, (image_aspect / bar_aspect).clamp(0.0, 1.0))
    }
}

/// The picker: one dropdown, listing the plain bar first.
///
/// Returns the new choice when it changes. Stored and compared by name
/// rather than by index so that inserting a flag in the middle of the list
/// does not silently change everybody's setting to their neighbour's.
pub fn picker(ui: &mut egui::Ui, current: &str) -> Option<String> {
    let mut chosen = None;
    let label = if current.is_empty() { PLAIN } else { current };

    egui::ComboBox::from_id_salt("flag-picker")
        .selected_text(
            egui::RichText::new(label)
                .size(theme::FONT_SMALL)
                .color(theme::text()),
        )
        .width(220.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_empty(), PLAIN).clicked() {
                chosen = Some(String::new());
            }
            for flag in FLAGS {
                if ui.selectable_label(current == flag.name, flag.name).clicked() {
                    chosen = Some(flag.name.to_string());
                }
            }
        });

    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every emblem asset is present and decodable. `include_bytes!` catches
    /// a missing file at compile time; this catches one that is there and is
    /// not a PNG, which is what a failed download leaves behind — the four
    /// that rate-limited on the first pass came back as HTML error pages of
    /// exactly the right shape to look like files.
    #[test]
    fn every_emblem_asset_is_a_real_png() {
        for flag in FLAGS {
            let Design::Emblem(bytes) = flag.design else {
                continue;
            };
            assert!(
                bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                "{} is not a PNG",
                flag.name
            );
            let info = png::Decoder::new(std::io::Cursor::new(bytes))
                .read_info()
                .unwrap_or_else(|e| panic!("{} will not decode: {e}", flag.name));
            let (w, h) = info.info().size();
            assert!(w >= 100 && h >= 50, "{} is {w}×{h}", flag.name);
        }
    }

    /// Names are the stored value, so two flags sharing one would make a
    /// preference ambiguous.
    #[test]
    fn flag_names_are_unique_and_not_the_plain_label() {
        let mut seen = std::collections::HashSet::new();
        for flag in FLAGS {
            assert!(seen.insert(flag.name), "two flags named {}", flag.name);
            assert_ne!(flag.name, PLAIN);
        }
    }

    /// A tricolour drawn in thirds is a different flag from one drawn in a
    /// half and two quarters.
    #[test]
    fn colombia_is_not_three_equal_bands() {
        let Design::Stripes { bands, .. } = &find("Colombia").expect("Colombia").design else {
            panic!("Colombia is drawn from colors");
        };
        assert_eq!(bands.iter().map(|(_, w)| *w).collect::<Vec<_>>(), [2, 1, 1]);
    }

    /// Every band has a weight, or the flag is invisible.
    #[test]
    fn no_stripe_flag_has_a_zero_weight() {
        for flag in FLAGS {
            if let Design::Stripes { bands, .. } = &flag.design {
                assert!(!bands.is_empty(), "{} has no bands", flag.name);
                assert!(
                    bands.iter().all(|(_, w)| *w > 0),
                    "{} has a zero-weight band",
                    flag.name
                );
            }
        }
    }

    /// The bands tile the bar exactly: no gap at the seams and nothing
    /// hanging off the end, at any size or weighting.
    #[test]
    fn the_bands_meet_exactly_and_fill_the_bar() {
        for span in [7.0f32, 20.0, 23.0, 64.0] {
            for weights in [vec![1u8, 1, 1], vec![2, 1, 1], vec![1, 1, 1, 1, 1, 1]] {
                let total: f32 = weights.iter().map(|w| *w as f32).sum();
                let mut walked = 0.0f32;
                let mut previous_end = 0.0f32;
                for weight in &weights {
                    let start = (walked / total * span).round();
                    walked += *weight as f32;
                    let end = (walked / total * span).round();
                    assert_eq!(start, previous_end, "seam at {span} with {weights:?}");
                    previous_end = end;
                }
                assert_eq!(previous_end, span, "short at {span} with {weights:?}");
            }
        }
    }

    /// An emblem is cropped, never squashed, and the crop is centered.
    #[test]
    fn an_emblem_keeps_its_shape_in_a_wide_bar() {
        // A 2:1 flag in a 40:1 bar shows a twentieth of its height.
        let (u, v) = cover_uv(2.0, 40.0);
        assert_eq!(u, 1.0);
        assert!((v - 0.05).abs() < 1e-6, "{v}");

        // And the other way round, which a tall bar or a wide flag reaches.
        let (u, v) = cover_uv(40.0, 2.0);
        assert!((u - 0.05).abs() < 1e-6, "{u}");
        assert_eq!(v, 1.0);

        // Never more than the whole image, whatever the numbers.
        for (ia, ba) in [(1.0, 1.0), (0.01, 100.0), (100.0, 0.01)] {
            let (u, v) = cover_uv(ia, ba);
            assert!((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v), "{u} {v}");
        }
    }
}
