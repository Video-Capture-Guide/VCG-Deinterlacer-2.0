//! The palette, the type scale, the drawn icons, and the shared widgets.
//!
//! Colors come from the redesign mockups. The accent moves from the Python's
//! `#7C6FF7` to `#6C5CE7`, and two semantic colors are new: amber for "a fix
//! is recommended" and blue for "an observation, no action". One meaning per
//! color, everywhere — see `Tag`.
//!
//! **Icons are drawn, never typed.** egui's bundled fonts do not carry
//! `✓ → ⓘ ↻ ·`, and a missing glyph renders as an empty box, which is exactly
//! what happened. Everything in `icon` is painted from primitives so it cannot
//! depend on font coverage.

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, RichText, Stroke, Vec2};

pub const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
    )
}

// ── The palette ─────────────────────────────────────────────────────────────

/// Every colour the interface uses, named by its role rather than its hue.
///
/// A struct rather than loose constants because there are two of these now
/// and the interface has to be able to swap between them at runtime. The
/// field names are the old constant names, so a call site reads the same as
/// it always did apart from the parentheses.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// The page.
    pub bg: Color32,
    /// Cards and raised surfaces.
    pub panel: Color32,
    /// Buttons, chips, inputs — one step above a card.
    pub panel2: Color32,
    pub sidebar: Color32,
    pub line: Color32,
    /// Border on hover.
    pub line_hover: Color32,

    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,

    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_hover: Color32,
    /// Text drawn *on* the accent — a filled primary button.
    ///
    /// Not `text`. In the dark palette `text` is near-white and happens to
    /// work; in the light one it is near-black, which put dark letters on a
    /// mid-purple button and made the most important control on the page the
    /// hardest to read. The accent is dark enough in both palettes for white
    /// to clear 4.5:1, so this is near-white in both.
    pub on_accent: Color32,
    /// A fix is recommended, or is required.
    pub fix: Color32,
    pub fix_soft: Color32,
    /// Nothing to do.
    pub ok: Color32,
    pub ok_soft: Color32,
    /// An observation, no action.
    pub info: Color32,
    pub info_soft: Color32,

    /// Shade drawn over the part of the frame a crop will remove.
    pub crop_shade: Color32,
    /// The head-switching band inside that crop.
    pub crop_band: Color32,
    /// The region that will survive as 4:3.
    pub crop_aspect: Color32,
}

/// The original dark palette, unchanged.
pub const DARK: Palette = Palette {
    bg: hex(0x1B1C22),
    panel: hex(0x24252D),
    panel2: hex(0x2B2C35),
    sidebar: hex(0x15161B),
    line: hex(0x353642),
    line_hover: hex(0x4A4B5A),

    text: hex(0xE8E8EE),
    muted: hex(0x9A9BAB),
    faint: hex(0x6D6E7C),

    accent: hex(0x6C5CE7),
    accent_soft: hex(0x2F2A52),
    accent_hover: hex(0x8574EE),
    on_accent: hex(0xFFFFFF),
    fix: hex(0xF0B429),
    fix_soft: hex(0x3A2E12),
    ok: hex(0x3ECF8E),
    ok_soft: hex(0x163327),
    info: hex(0x5AA9E6),
    info_soft: hex(0x152B3C),

    crop_shade: Color32::from_rgba_premultiplied(14, 15, 18, 190),
    crop_band: Color32::from_rgba_premultiplied(100, 75, 17, 110),
    crop_aspect: Color32::from_rgba_premultiplied(45, 45, 88, 120),
};

/// The light palette.
///
/// Not the dark one inverted. Dark interfaces carry meaning in *brightness* —
/// a lit accent on a dim ground — and light ones carry it in *depth*: the
/// page is grey, cards are white and sit above it, and the accent has to be
/// darkened or it fails against white.
///
/// The brand purple is kept, as asked, but `accent` is the darker
/// `#5B4BD6` here: `#6C5CE7` is only about 4:1 on white, which is under the
/// 4.5:1 that body-sized text needs. The three status colours move much
/// further — amber and green that read well on a near-black page are close
/// to invisible on white, so each has a darkened variant and keeps its pale
/// tint for the backing. `contrast_is_sufficient_in_both_palettes` checks
/// every one of these rather than trusting the eye.
pub const LIGHT: Palette = Palette {
    // The greys carry a little of the accent's hue rather than being
    // neutral. A light interface built from pure greys reads as unfinished
    // — there is nothing in it but the one purple, and the eye reads that as
    // a wireframe someone forgot to colour. A few points of blue-violet in
    // the surfaces tie the page to the accent without becoming a colour
    // scheme of their own, and the sidebar carries the most of it because it
    // is the one surface that is always on screen.
    bg: hex(0xF4F3F8),
    panel: hex(0xFFFFFF),
    panel2: hex(0xEEEDF5),
    sidebar: hex(0xE9E7F3),
    line: hex(0xDAD8E4),
    line_hover: hex(0xA8A6B6),

    text: hex(0x1A1A1F),
    muted: hex(0x53535E),
    faint: hex(0x6B6B76),

    accent: hex(0x5B4BD6),
    accent_soft: hex(0xE7E4FB),
    accent_hover: hex(0x4536B0),
    on_accent: hex(0xFFFFFF),
    fix: hex(0x8A5A00),
    fix_soft: hex(0xFBF0D5),
    ok: hex(0x0E7A4E),
    ok_soft: hex(0xD8F2E5),
    info: hex(0x11598F),
    info_soft: hex(0xDCEBF8),

    crop_shade: Color32::from_rgba_premultiplied(150, 150, 155, 180),
    crop_band: Color32::from_rgba_premultiplied(110, 80, 10, 120),
    crop_aspect: Color32::from_rgba_premultiplied(70, 65, 160, 120),
};

/// Which palette is live. 0 is dark, 1 is light.
///
/// An atomic rather than a `thread_local`, because the export worker builds
/// a watermark preview off the UI thread and would otherwise draw it in
/// whichever palette that thread had never been told about.
static ACTIVE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The palette in force.
#[inline]
pub fn palette() -> &'static Palette {
    if ACTIVE.load(std::sync::atomic::Ordering::Relaxed) == 1 {
        &LIGHT
    } else {
        &DARK
    }
}

/// Which of the two is showing.
pub fn is_light() -> bool {
    ACTIVE.load(std::sync::atomic::Ordering::Relaxed) == 1
}

fn set_palette(light: bool) {
    ACTIVE.store(
        u8::from(light),
        std::sync::atomic::Ordering::Relaxed,
    );
}

macro_rules! color_accessors {
    ($($name:ident),+ $(,)?) => {
        $(
            #[inline]
            pub fn $name() -> Color32 { palette().$name }
        )+
    };
}

color_accessors!(
    bg, panel, panel2, sidebar, line, line_hover,
    text, muted, faint,
    accent, accent_soft, accent_hover,
    // No accessor for `on_accent`: the only thing that paints on the accent
    // is `primary()`, which lives here and reads the field directly.
    fix, fix_soft, ok, ok_soft, info,
    // `info_soft` has no accessor because nothing tints an info badge yet.
    // The field stays on `Palette` so the three semantic colours keep their
    // matching pair, and `palette().info_soft` reaches it the day something
    // wants it — an accessor nobody calls is just dead code.
    crop_shade, crop_band, crop_aspect,
);

/// Corner radius for things that float above the page — dialogs, flyouts,
/// the About panel. Windows 11 uses 8px for these.
pub const RADIUS: f32 = 8.0;
/// Corner radius for things that sit *in* the page — buttons, chips, inputs,
/// list backplates, progress bars. Windows 11 uses 4px.
///
/// Both were larger (10 and 7). Fluent is specific about these two numbers
/// and a control rounder than the platform's reads as a web page embedded in
/// a window rather than as part of it.
pub const RADIUS_SMALL: f32 = 4.0;

// ── Type scale ──────────────────────────────────────────────────────────────

// The Windows 11 type ramp, which is a fixed set of size/line-height pairs
// rather than a free scale: 12/16 caption, 14/20 body, 18/24 body large,
// 20/28 subtitle, 28/36 title, 40/52 title large. Sizes that fell between
// two rungs have been moved onto the nearer one — 14.5 and 13 were not
// anything in particular, and a ramp only creates hierarchy if its steps
// are the ones the rest of the system uses.

/// Caption. The floor: Fluent calls anything smaller illegible in some
/// languages at regular weight.
pub const FONT_TINY: f32 = 12.0;
/// Caption, where it sits beside body text rather than under it.
pub const FONT_SMALL: f32 = 12.0;
/// Body. The ramp's anchor.
pub const FONT_BODY: f32 = 14.0;
/// Body large — a lead paragraph.
pub const FONT_LEAD: f32 = 18.0;
/// Body large, for a card heading that is not a section.
pub const FONT_SECTION: f32 = 18.0;
/// The numbered section headings on a wizard page.
///
/// Deliberately between `FONT_SECTION` and `FONT_TITLE`: a section heading
/// outranks the card titles under it — "Remove interlacing lines" is one of
/// those — and is outranked by the name of the page. At `FONT_SECTION` it
/// tied with its own contents and the page read as one flat list.
pub const FONT_SUBHEAD: f32 = 20.0;
/// Title.
pub const FONT_TITLE: f32 = 28.0;
/// Title large, for the front page only.
pub const FONT_HERO: f32 = 40.0;

/// How tall a progress bar is.
///
/// Taller than the ten pixels it used to be, because the fill can now be a
/// flag and a flag needs room to read as one. An emblem is fitted to this
/// without distortion, so every pixel of height is another slice of the
/// picture that survives the crop.
pub const BAR_HEIGHT: f32 = 22.0;

/// Whether "Recommended" sits on the auto-fix mode. A single constant, because
/// the brief calls it a product decision rather than a layout one.
pub const RECOMMEND_AUTO_FIX: bool = true;

/// How big the interface is drawn.
///
/// Applied as egui's zoom factor rather than by changing the type ramp, so
/// *everything* scales together — text, buttons, icons, padding, the lot.
/// Scaling only the fonts would leave 14pt text in a 30px-tall button and
/// break every fixed-width column on the settings pages.
///
/// This exists because the people most likely to be restoring forty-year-old
/// family tape are not twenty-five, and the Fluent ramp's 14px body is small
/// on a 1080p screen at arm's length. Windows has a global display scale, but
/// reaching for it means changing every application to fix one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextSize {
    Small,
    #[default]
    Medium,
    Large,
    Larger,
}

impl TextSize {
    pub const ALL: [TextSize; 4] = [
        TextSize::Small,
        TextSize::Medium,
        TextSize::Large,
        TextSize::Larger,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TextSize::Small => "Small",
            TextSize::Medium => "Medium",
            TextSize::Large => "Large",
            TextSize::Larger => "Larger",
        }
    }

    /// Medium is 1.0, which is the Fluent ramp at its stated sizes. Small is
    /// there for anyone who had the old, denser layout and wants it back.
    pub fn zoom(self) -> f32 {
        match self {
            TextSize::Small => 0.9,
            TextSize::Medium => 1.0,
            TextSize::Large => 1.15,
            TextSize::Larger => 1.3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TextSize::Small => "small",
            TextSize::Medium => "medium",
            TextSize::Large => "large",
            TextSize::Larger => "larger",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "small" => TextSize::Small,
            "large" => TextSize::Large,
            "larger" => TextSize::Larger,
            _ => TextSize::Medium,
        }
    }
}

/// Which palette the user wants, independent of what the system is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    /// Follow Windows. The default, and what Fluent expects of an app.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "Match Windows",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }

    /// Stored as a short string rather than an index, so inserting a variant
    /// cannot silently change somebody's saved preference into its neighbour.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "light" => ThemeChoice::Light,
            "dark" => ThemeChoice::Dark,
            _ => ThemeChoice::System,
        }
    }
}

/// Install the fonts, once.
///
/// The app shipped in **Ubuntu-Light** — egui's bundled default, and a Linux
/// font. Nothing else about the window could make up for the text being in
/// the wrong typeface, and it is the single largest reason the interface did
/// not read as a Windows application.
///
/// Segoe UI Variable is the Windows 11 system font; Segoe UI is the Windows
/// 10 one. Both are loaded from the system font directory rather than
/// bundled, because redistributing them is not ours to do. If neither is
/// there — a Windows install stripped of its fonts, or a non-Windows host
/// running the tests — egui's default stays, which is the right failure: the
/// interface looks wrong rather than failing to start.
pub fn install_fonts(ctx: &egui::Context) {
    let dir = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("C:/Windows"))
        .join("Fonts");

    // In preference order. The variable font first: it carries its own
    // optical sizing, which is what keeps 12px legible and 40px shapely.
    let candidates = [
        "SegoeUIVariableStatic.ttf",
        "SegoeUIVF.ttf",
        "segoeui.ttf",
    ];
    let Some((name, bytes)) = candidates.iter().find_map(|f| {
        std::fs::read(dir.join(f))
            .ok()
            .map(|b| ((*f).to_string(), b))
    }) else {
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "ui".to_owned(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    // Inserted at the front, so the bundled font stays behind it as the
    // fallback for any glyph Segoe does not carry.
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "ui".to_owned());

    // The semibold face, for emphasis. Fluent excludes Bold and Italic from
    // the ramp — Bold is heavier than the system uses anywhere, and Italic
    // hurts legibility, particularly for dyslexic readers — so `.strong()`
    // is mapped onto Semibold where the face exists.
    for f in ["seguisb.ttf", "SegoeUIVariableStatic-Semibold.ttf"] {
        if let Ok(b) = std::fs::read(dir.join(f)) {
            fonts.font_data.insert(
                "ui-semibold".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(b)),
            );
            fonts
                .families
                .insert(egui::FontFamily::Name("semibold".into()), vec!["ui-semibold".to_owned()]);
            HAS_SEMIBOLD.store(true, std::sync::atomic::Ordering::Relaxed);
            break;
        }
    }

    // Segoe Fluent Icons - the Windows system icon font. Used for the few
    // places a platform glyph says it better than a drawn shape: the file
    // kinds on the Export page, where a document, a reel, a code file and a
    // gear are shapes people already read without a caption.
    //
    // Everything else in `icon` stays drawn. egui's bundled fonts carry none
    // of these and a missing glyph renders as an empty box - which is exactly
    // what happened when this app first tried to type its ticks and arrows.
    // A system font is safe here only because its presence is checked before
    // anything is asked to use it.
    if let Ok(b) = std::fs::read(dir.join("SegoeIcons.ttf")) {
        fonts.font_data.insert(
            "fluent-icons".to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(b)),
        );
        fonts.families.insert(
            egui::FontFamily::Name("icons".into()),
            vec!["fluent-icons".to_owned()],
        );
        HAS_ICONS.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    let _ = name;
    ctx.set_fonts(fonts);
}

/// Whether the Windows icon font was found.
static HAS_ICONS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// One Segoe Fluent Icons glyph, or nothing if the font is not installed.
///
/// Draws nothing rather than a placeholder box when the font is missing: the
/// label beside it already says what the thing is, so a missing decoration
/// should cost a few pixels of indent and nothing else.
pub fn fluent_icon(ui: &mut egui::Ui, glyph: &str, size: f32, color: Color32) {
    if !HAS_ICONS.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    ui.label(
        RichText::new(glyph)
            .size(size)
            .family(egui::FontFamily::Name("icons".into()))
            .color(color),
    );
}

/// The glyphs used on the Export page, by what they stand for.
///
/// Checked against the installed font's character map before being used, not
/// taken from a documentation page.
pub mod glyph {
    /// A text document.
    pub const DOCUMENT: &str = "\u{E8A5}";
    /// A strip of film.
    pub const VIDEO: &str = "\u{E714}";
    /// Source code.
    pub const CODE: &str = "\u{E943}";
    /// A settings gear.
    pub const SETTINGS: &str = "\u{E713}";
}

/// Set once at startup, so the family lookup does not need a `Context`.
static HAS_SEMIBOLD: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The family to set headings in.
///
/// Fluent asks for Semibold on titles and Regular everywhere else, and
/// explicitly excludes Bold and Italic — Bold is heavier than the system
/// uses anywhere, and Italic hurts legibility for dyslexic readers.
///
/// This is not what `RichText::strong()` does. egui's `strong` is a *colour*
/// change — it swaps in `visuals.strong_text_color()` — and egui carries no
/// weight axis at all, so the only way to get a heavier face is to name a
/// family that is one. Hence this.
///
/// Falls back to the proportional family when Segoe's semibold face is not
/// installed, which is the right failure: a heading set at regular weight is
/// still a heading.
pub fn semibold() -> egui::FontFamily {
    if HAS_SEMIBOLD.load(std::sync::atomic::Ordering::Relaxed) {
        egui::FontFamily::Name("semibold".into())
    } else {
        egui::FontFamily::Proportional
    }
}

/// Decide the palette and apply the whole style.
///
/// Called every frame, which is cheap and means a user flipping Windows into
/// dark mode while the app is open sees it follow without restarting — which
/// is what "Personal" asks for and what every other Windows app does.
pub fn apply_with(ctx: &egui::Context, choice: ThemeChoice, size: TextSize) {
    // Set before the style, and only when it changes: egui treats a zoom
    // change as a full relayout, so writing it every frame would rebuild the
    // whole interface sixty times a second.
    if (ctx.zoom_factor() - size.zoom()).abs() > f32::EPSILON {
        ctx.set_zoom_factor(size.zoom());
    }

    let light = match choice {
        ThemeChoice::Light => true,
        ThemeChoice::Dark => false,
        // `system_theme` is None until the host reports one, which it does
        // not do on every platform. Dark is the app's own heritage, so it is
        // the answer when nobody has said otherwise.
        ThemeChoice::System => ctx.system_theme() == Some(egui::Theme::Light),
    };

    // A change has to ask for a repaint of its own. egui only redraws in
    // response to events, and Windows switching between light and dark is
    // not one it counts — so without this the palette flips in memory and
    // the window carries on showing the old one until the user happens to
    // move the mouse over it. Caught by flipping the system setting with the
    // app open and watching nothing happen.
    if light != is_light() {
        set_palette(light);
        ctx.request_repaint();
    }
    apply(ctx);
}

pub fn apply(ctx: &egui::Context) {
    let mut v = if is_light() {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };

    v.panel_fill = palette().bg;
    v.window_fill = palette().panel;
    v.extreme_bg_color = palette().bg;
    v.faint_bg_color = palette().panel2;
    v.override_text_color = Some(palette().text);

    v.widgets.noninteractive.bg_fill = palette().panel;
    v.widgets.noninteractive.weak_bg_fill = palette().panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette().line);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette().muted);

    // A secondary button must not read as disabled: solid fill, visible
    // border, full-strength label. The faded look is reserved for genuinely
    // disabled controls, which egui dims for us.
    v.widgets.inactive.bg_fill = palette().panel2;
    v.widgets.inactive.weak_bg_fill = palette().panel2;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, palette().line);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, palette().text);

    v.widgets.hovered.bg_fill = palette().panel2;
    v.widgets.hovered.weak_bg_fill = palette().panel2;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, palette().line_hover);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, palette().text);

    v.widgets.active.bg_fill = palette().accent_soft;
    v.widgets.active.weak_bg_fill = palette().accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0, palette().accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0, palette().text);

    v.selection.bg_fill = palette().accent_soft;
    v.selection.stroke = Stroke::new(1.0, palette().accent);
    v.hyperlink_color = palette().accent_hover;

    let r = CornerRadius::same(RADIUS_SMALL as u8);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    v.window_corner_radius = CornerRadius::same(RADIUS as u8);

    ctx.set_visuals(v);

    ctx.all_styles_mut(|style| {
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Heading, FontId::new(FONT_TITLE, Proportional)),
            (TextStyle::Body, FontId::new(FONT_BODY, Proportional)),
            (TextStyle::Button, FontId::new(FONT_BODY, Proportional)),
            (TextStyle::Small, FontId::new(FONT_SMALL, Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(FONT_SMALL, egui::FontFamily::Monospace),
            ),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(16.0, 9.0);
        style.spacing.interact_size.y = 30.0;
    });
}

// ── Drawn icons ─────────────────────────────────────────────────────────────

pub mod icon {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Dir {
        Right,
        Left,
        Down,
    }

    /// A tick. Two strokes, so it reads at 10px as well as 20px.
    pub fn check(painter: &egui::Painter, rect: Rect, color: Color32) {
        let w = rect.width();
        let s = Stroke::new((w * 0.16).max(1.4), color);
        let c = rect.center();
        let a = Pos2::new(c.x - w * 0.28, c.y + w * 0.02);
        let b = Pos2::new(c.x - w * 0.07, c.y + w * 0.23);
        let d = Pos2::new(c.x + w * 0.30, c.y - w * 0.24);
        painter.line_segment([a, b], s);
        painter.line_segment([b, d], s);
    }

    /// An arrow: shaft plus a filled head.
    pub fn arrow(painter: &egui::Painter, rect: Rect, color: Color32, dir: Dir) {
        let w = rect.width();
        let c = rect.center();
        let s = Stroke::new((w * 0.13).max(1.3), color);
        let h = w * 0.26;
        match dir {
            Dir::Right => {
                painter.line_segment(
                    [Pos2::new(c.x - w * 0.34, c.y), Pos2::new(c.x + w * 0.16, c.y)],
                    s,
                );
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(c.x + w * 0.10, c.y - h),
                        Pos2::new(c.x + w * 0.10, c.y + h),
                        Pos2::new(c.x + w * 0.38, c.y),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
            Dir::Left => {
                painter.line_segment(
                    [Pos2::new(c.x + w * 0.34, c.y), Pos2::new(c.x - w * 0.16, c.y)],
                    s,
                );
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(c.x - w * 0.10, c.y - h),
                        Pos2::new(c.x - w * 0.10, c.y + h),
                        Pos2::new(c.x - w * 0.38, c.y),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
            Dir::Down => {
                painter.line_segment(
                    [Pos2::new(c.x, c.y - w * 0.34), Pos2::new(c.x, c.y + w * 0.14)],
                    s,
                );
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(c.x - h, c.y + w * 0.08),
                        Pos2::new(c.x + h, c.y + w * 0.08),
                        Pos2::new(c.x, c.y + w * 0.38),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
        }
    }

    /// A disclosure chevron, pointing down when open and right when closed.
    pub fn chevron(painter: &egui::Painter, rect: Rect, color: Color32, open: bool) {
        let w = rect.width();
        let c = rect.center();
        let s = Stroke::new((w * 0.14).max(1.3), color);
        let r = w * 0.22;
        if open {
            painter.line_segment(
                [Pos2::new(c.x - r, c.y - r * 0.5), Pos2::new(c.x, c.y + r * 0.6)],
                s,
            );
            painter.line_segment(
                [Pos2::new(c.x, c.y + r * 0.6), Pos2::new(c.x + r, c.y - r * 0.5)],
                s,
            );
        } else {
            painter.line_segment(
                [Pos2::new(c.x - r * 0.5, c.y - r), Pos2::new(c.x + r * 0.6, c.y)],
                s,
            );
            painter.line_segment(
                [Pos2::new(c.x + r * 0.6, c.y), Pos2::new(c.x - r * 0.5, c.y + r)],
                s,
            );
        }
    }

    /// A circled lower-case i.
    pub fn info(painter: &egui::Painter, rect: Rect, color: Color32) {
        let w = rect.width();
        let c = rect.center();
        painter.circle_stroke(c, w * 0.42, Stroke::new((w * 0.09).max(1.0), color));
        painter.circle_filled(Pos2::new(c.x, c.y - w * 0.19), (w * 0.055).max(0.9), color);
        painter.line_segment(
            [
                Pos2::new(c.x, c.y - w * 0.04),
                Pos2::new(c.x, c.y + w * 0.21),
            ],
            Stroke::new((w * 0.1).max(1.0), color),
        );
    }

    /// A circular refresh arrow, for "re-check".
    pub fn refresh(painter: &egui::Painter, rect: Rect, color: Color32) {
        let w = rect.width();
        let c = rect.center();
        let r = w * 0.34;
        let s = Stroke::new((w * 0.12).max(1.2), color);
        // An open circle: a gap at the top right leaves room for the head.
        let mut pts = Vec::new();
        let steps = 22;
        for i in 0..=steps {
            let t = -0.45 + (i as f32 / steps as f32) * (std::f32::consts::TAU - 0.95);
            pts.push(Pos2::new(c.x + r * t.cos(), c.y + r * t.sin()));
        }
        painter.add(egui::Shape::line(pts, s));
        let head = Pos2::new(c.x + r * (-0.45f32).cos(), c.y + r * (-0.45f32).sin());
        painter.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(head.x - w * 0.02, head.y - w * 0.18),
                Pos2::new(head.x + w * 0.19, head.y - w * 0.02),
                Pos2::new(head.x - w * 0.14, head.y + w * 0.06),
            ],
            color,
            Stroke::NONE,
        ));
    }

    /// An en dash, for "not doing this".
    pub fn dash(painter: &egui::Painter, rect: Rect, color: Color32) {
        let w = rect.width();
        let c = rect.center();
        painter.line_segment(
            [Pos2::new(c.x - w * 0.28, c.y), Pos2::new(c.x + w * 0.28, c.y)],
            Stroke::new((w * 0.13).max(1.3), color),
        );
    }

    /// A padlock, for a toggle that cannot be turned off.
    pub fn lock(painter: &egui::Painter, rect: Rect, color: Color32) {
        let w = rect.width();
        let c = rect.center();
        let body = Rect::from_center_size(
            Pos2::new(c.x, c.y + w * 0.13),
            Vec2::new(w * 0.52, w * 0.40),
        );
        painter.rect_filled(body, CornerRadius::same((w * 0.08) as u8), color);
        // The shackle, as an arc.
        let r = w * 0.19;
        let mut pts = Vec::new();
        for i in 0..=12 {
            let t = std::f32::consts::PI + (i as f32 / 12.0) * std::f32::consts::PI;
            pts.push(Pos2::new(
                c.x + r * t.cos(),
                c.y - w * 0.09 + r * t.sin(),
            ));
        }
        painter.add(egui::Shape::line(
            pts,
            Stroke::new((w * 0.1).max(1.0), color),
        ));
    }
}

/// Allocate space inline and draw an icon in it.
pub fn inline_icon(
    ui: &mut egui::Ui,
    size: f32,
    color: Color32,
    draw: impl FnOnce(&egui::Painter, Rect, Color32),
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        draw(ui.painter(), rect, color);
    }
}

// ── Building blocks ─────────────────────────────────────────────────────────

/// A card.
pub fn card() -> egui::Frame {
    egui::Frame::NONE
        .fill(palette().panel)
        .stroke(Stroke::new(1.0, palette().line))
        .corner_radius(RADIUS)
        .inner_margin(egui::Margin::same(16))
}

/// A card whose left edge will carry a status color.
///
/// The colored bar is painted afterwards by `left_edge`, because an egui frame
/// takes one uniform stroke and cannot color a single side. The extra left
/// padding leaves room for it.
pub fn edged_card() -> egui::Frame {
    card().inner_margin(egui::Margin {
        left: 19,
        right: 16,
        top: 16,
        bottom: 16,
    })
}

/// Paint a colored bar down the left edge of `rect`.
pub fn left_edge(ui: &egui::Ui, rect: Rect, color: Color32) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let bar = Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height()));
    ui.painter().rect_filled(
        bar,
        CornerRadius {
            nw: RADIUS as u8,
            sw: RADIUS as u8,
            ne: 0,
            se: 0,
        },
        color,
    );
}

/// A small outlined label, as used for file metadata.
pub fn chip(ui: &mut egui::Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(FONT_SMALL),
        palette().muted,
    );
    let size = galley.size() + Vec2::new(16.0, 7.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, RADIUS_SMALL * 0.7, palette().panel2);
        p.rect_stroke(
            rect,
            RADIUS_SMALL * 0.7,
            Stroke::new(1.0, palette().line),
            egui::StrokeKind::Inside,
        );
        p.galley(rect.center() - galley.size() / 2.0, galley, palette().muted);
    }
}

/// One meaning per color, used for every status tag in the app.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    /// Applied, cannot be turned off.
    Required,
    /// On by default, the user may turn it off.
    FixRecommended,
    /// Nothing to do.
    LooksGood,
    /// An observation, no action.
    Info,
}

impl Tag {
    pub fn color(self) -> Color32 {
        match self {
            Tag::Required | Tag::FixRecommended => palette().fix,
            Tag::LooksGood => palette().ok,
            Tag::Info => palette().info,
        }
    }

    fn soft(self) -> Color32 {
        match self {
            Tag::Required | Tag::FixRecommended => palette().fix_soft,
            Tag::LooksGood => palette().ok_soft,
            Tag::Info => palette().info_soft,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tag::Required => "Required",
            Tag::FixRecommended => "Fix recommended",
            Tag::LooksGood => "Looks good",
            Tag::Info => "Info",
        }
    }
}

/// A status tag. `text` overrides the default label, so severity can be kept
/// ("Light — fix recommended") without inventing a new color.
pub fn tag(ui: &mut egui::Ui, kind: Tag, text: Option<&str>) {
    let label = text.unwrap_or_else(|| kind.label());
    let color = kind.color();
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(FONT_TINY),
        color,
    );
    let size = galley.size() + Vec2::new(16.0, 6.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, size.y / 2.0, kind.soft());
        p.rect_stroke(
            rect,
            size.y / 2.0,
            Stroke::new(1.0, color.gamma_multiply(0.55)),
            egui::StrokeKind::Inside,
        );
        p.galley(rect.center() - galley.size() / 2.0, galley, color);
    }
}

// ── Text helpers ────────────────────────────────────────────────────────────

pub fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(
        RichText::new(title)
            .size(FONT_TITLE)
            .family(semibold())
            .color(palette().text),
    );
    if !subtitle.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new(subtitle).size(FONT_BODY).color(palette().muted));
    }
}

pub fn section_header(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .size(FONT_SECTION)
            .family(semibold())
            .color(palette().text),
    );
}

// ── Buttons ─────────────────────────────────────────────────────────────────

/// The single forward action on a screen.
pub fn primary(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(text)
                .size(FONT_BODY)
                .color(palette().on_accent),
        )
            .fill(palette().accent)
            .stroke(Stroke::new(1.0, palette().accent))
            .corner_radius(RADIUS_SMALL)
            .min_size(Vec2::new(0.0, 38.0)),
    )
}

/// A normal action. Deliberately not faded — that look is for disabled only.
pub fn secondary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(FONT_BODY).color(palette().text))
            .fill(palette().panel2)
            .stroke(Stroke::new(1.0, palette().line))
            .corner_radius(RADIUS_SMALL)
            .min_size(Vec2::new(0.0, 36.0)),
    )
}

/// A borderless action, for things like "Re-check video".
pub fn ghost(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(FONT_BODY).color(palette().muted))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(RADIUS_SMALL)
            .min_size(Vec2::new(0.0, 36.0)),
    )
}

/// An inline textual link.
pub fn link(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let response = ui.add(
        egui::Button::new(RichText::new(text).size(FONT_SMALL).color(palette().accent_hover))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(0.0)
            .min_size(Vec2::ZERO),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}


/// An indeterminate progress bar: a band that slides along a track.
///
/// For waits with no measurable progress — the preview render is the one that
/// prompted it. A line of grey text saying "Preparing preview…" is easy to
/// read as a dead end, and this one can take a couple of minutes, so people
/// concluded it had hung. Something moving says the machine is still working
/// in a way no wording can.
///
/// Drives its own repaints, so a caller cannot forget to animate it.
pub fn indeterminate_bar(ui: &mut egui::Ui, width: f32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let radius = height / 2.0;
    ui.painter().rect_filled(rect, radius, palette().panel2);

    // A band a third of the width, sweeping across and easing at the turns so
    // it reads as deliberate motion rather than a glitch.
    let period = 1.6_f64;
    let t = (ui.input(|i| i.time) % period) / period;
    let eased = (1.0 - (t * std::f64::consts::TAU).cos()) / 2.0;
    let band = width * 0.34;
    let x = rect.left() + (rect.width() + band) * eased as f32 - band;

    let band_rect = egui::Rect::from_min_max(
        Pos2::new(x.max(rect.left()), rect.top()),
        Pos2::new((x + band).min(rect.right()), rect.bottom()),
    );
    if band_rect.width() > 0.5 {
        ui.painter().rect_filled(band_rect, radius, palette().accent);
    }

    // 60fps while it is on screen; the bar is the only thing moving.
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
}

/// Draw a focus ring around a widget that has keyboard focus.
///
/// egui puts anything sensing clicks into the tab order and activates it on
/// Space or Enter, so the custom-drawn controls here were already reachable —
/// they just gave no sign of it, which makes keyboard navigation useless in
/// practice. You cannot use what you cannot see.
///
/// Drawn outside the widget rather than on its border, so it never covers a
/// control's own outline, and in the accent colour so it reads as "here" on
/// a dark theme without looking like a selected state.
pub fn focus_ring(ui: &egui::Ui, response: &egui::Response, radius: f32) {
    if !response.has_focus() {
        return;
    }
    let rect = response.rect.expand(3.0);
    ui.painter().rect_stroke(
        rect,
        radius + 3.0,
        Stroke::new(2.0, palette().accent_hover),
        egui::StrokeKind::Outside,
    );
}

/// An on/off switch. Returns true when toggled.
///
/// `locked` draws a padlock and refuses the click, for a fix that is required.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool, locked: bool) -> bool {
    let size = Vec2::new(42.0, 23.0);
    let (rect, response) = ui.allocate_exact_size(
        size,
        if locked {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        },
    );

    let changed = if response.clicked() {
        *on = !*on;
        true
    } else {
        false
    };
    focus_ring(ui, &response, size.y / 2.0);

    if ui.is_rect_visible(rect) {
        let track = if *on { palette().accent } else { palette().panel2 };
        let p = ui.painter();
        p.rect_filled(rect, size.y / 2.0, track);
        p.rect_stroke(
            rect,
            size.y / 2.0,
            Stroke::new(1.0, if *on { palette().accent } else { palette().line }),
            egui::StrokeKind::Inside,
        );
        let knob_x = if *on {
            rect.right() - size.y / 2.0
        } else {
            rect.left() + size.y / 2.0
        };
        let knob = Pos2::new(knob_x, rect.center().y);
        p.circle_filled(knob, size.y / 2.0 - 3.0, palette().text);
        if locked {
            icon::lock(
                p,
                Rect::from_center_size(knob, Vec2::splat(size.y - 10.0)),
                palette().accent,
            );
        }
    }

    if !locked && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    changed
}

/// A disclosure row: a chevron, a label, and whether it is open.
pub fn disclosure(ui: &mut egui::Ui, label: &str, open: &mut bool) -> bool {
    let mut changed = false;
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            inline_icon(ui, 13.0, palette().accent_hover, |p, r, c| {
                icon::chevron(p, r, c, *open)
            });
            ui.label(
                RichText::new(label)
                    .size(FONT_SMALL)
                    .color(palette().accent_hover),
            );
        })
        .response
        .interact(egui::Sense::click());

    if response.clicked() {
        *open = !*open;
        changed = true;
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    focus_ring(ui, &response, RADIUS_SMALL);
    changed
}

/// Monospace technical detail, shown inside a disclosure.
pub fn details_text(ui: &mut egui::Ui, text: &str) {
    egui::Frame::NONE
        .fill(palette().bg)
        .corner_radius(RADIUS_SMALL)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(text)
                    .size(FONT_SMALL)
                    .monospace()
                    .color(palette().muted),
            );
        });
}

/// A small pill-shaped toggle button, for picking one of a short row.
///
/// Distinct from `chip`, which is a static label: this one is pressable and
/// shows which of a set is active. Used where a radio list would be three
/// nearly empty rows — a size, a corner, a strength.
pub fn chip_button(ui: &mut egui::Ui, text: &str, selected: bool) -> bool {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(FONT_SMALL),
        if selected { palette().text } else { palette().muted },
    );
    let size = Vec2::new(galley.size().x + 26.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(
            rect,
            15.0,
            if selected {
                palette().accent_soft
            } else if response.hovered() {
                palette().panel2
            } else {
                palette().bg
            },
        );
        p.rect_stroke(
            rect,
            15.0,
            Stroke::new(1.0, if selected { palette().accent } else { palette().line }),
            egui::StrokeKind::Inside,
        );
        p.galley(
            egui::pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            if selected { palette().text } else { palette().muted },
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    focus_ring(ui, &response, size.y / 2.0);
    response.clicked() && !selected
}

/// The front page's Start button, drawn as a VCR transport key.
///
/// Modelled on the real thing: a recessed bezel, a key sitting in it, a play
/// triangle and the word START. Drawn from primitives rather than built from
/// the reference photograph — a 4.6 MB picture of a plastic button would not
/// scale with the window, would not follow the light and dark palettes, and
/// would sit on the page as a photograph of a control rather than as one.
///
/// Deliberately the only control in the app that looks like this. It is the
/// one button on the one screen where the app should feel like the machine
/// the tape came out of; everywhere else, Fluent.
pub fn vcr_start(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    let size = Vec2::new(188.0, 92.0);
    let (rect, response) = ui.allocate_exact_size(
        size,
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let pressed = enabled && response.is_pointer_button_down_on();
        let hovered = enabled && response.hovered();

        // The bezel the key sits in: darker than the page in either palette,
        // so the key reads as recessed rather than as a floating rectangle.
        let bezel = if is_light() {
            hex(0xC9C9CF)
        } else {
            hex(0x101015)
        };
        p.rect_filled(rect, RADIUS, bezel);

        // The key itself, inset, and pushed down a pixel while held.
        let mut key = rect.shrink(9.0);
        if pressed {
            key = key.translate(egui::vec2(0.0, 1.5));
        }
        let face = if !enabled {
            if is_light() { hex(0xB4B4BC) } else { hex(0x2A2A32) }
        } else if hovered {
            palette().accent_hover
        } else {
            palette().accent
        };
        p.rect_filled(key, RADIUS_SMALL, face);

        // A highlight along the top edge, which is what makes a moulded key
        // look moulded rather than flat.
        if enabled && !pressed {
            p.line_segment(
                [
                    egui::pos2(key.left() + 6.0, key.top() + 1.0),
                    egui::pos2(key.right() - 6.0, key.top() + 1.0),
                ],
                Stroke::new(1.0, Color32::from_white_alpha(38)),
            );
        }

        let ink = if enabled {
            palette().on_accent
        } else {
            palette().faint
        };

        // The play triangle, above the word, as on the real key.
        let tri_c = egui::pos2(key.center().x, key.top() + key.height() * 0.36);
        let r = 13.0;
        p.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(tri_c.x - r * 0.55, tri_c.y - r * 0.82),
                egui::pos2(tri_c.x - r * 0.55, tri_c.y + r * 0.82),
                egui::pos2(tri_c.x + r * 0.95, tri_c.y),
            ],
            ink,
            Stroke::NONE,
        ));

        p.text(
            egui::pos2(key.center().x, key.bottom() - key.height() * 0.26),
            egui::Align2::CENTER_CENTER,
            "START",
            egui::FontId::new(15.0, semibold()),
            ink,
        );
    }

    if enabled {
        focus_ring(ui, &response, RADIUS);
    }
    response
}


/// Hand a URL to the system browser.
///
/// Failure is silent on purpose: there is nothing the user can do about a
/// machine with no browser association, and an error box over a link they
/// can read and type themselves would be worse than the link not opening.
pub fn open_url(url: &str) {
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

/// A wrapped paragraph, laid out ragged-right.
///
/// egui justifies a wrapping label whenever the enclosing layout has
/// `horizontal_justify()` set — `Label` copies it straight out of the layout
/// and there is no way to say "no" on the widget. In a justified layout that
/// stretches the spaces of every wrapped line to the full column width, which
/// on a short line with few spaces opens gaps wide enough to read as a bug.
/// It was reported as one.
///
/// Forcing a `top_down(LEFT)` child turns it off, because that layout's
/// `cross_justify` is false. Everything that renders a sentence should come
/// through here rather than calling `Label::wrap` directly.
pub fn paragraph(ui: &mut egui::Ui, text: RichText) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        Vec2::new(width, 0.0),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.set_width(width);
            ui.add(egui::Label::new(text).wrap());
        },
    );
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    /// WCAG relative luminance.
    fn luminance(c: Color32) -> f64 {
        let ch = |v: u8| {
            let s = v as f64 / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Text has to be readable on the surface it sits on, in both palettes.
    ///
    /// This is the check that stopped the light palette from being the dark
    /// one inverted. The brand purple is `#6C5CE7`, which measures about 4:1
    /// on white — under the 4.5:1 that body text needs — so light mode uses a
    /// darkened `#5B4BD6`. Amber and green went further still: both read
    /// clearly on a near-black page and are close to invisible on white.
    #[test]
    fn contrast_is_sufficient_in_both_palettes() {
        // 4.5:1 is the WCAG AA bar for body text. `faint` is deliberately the
        // quietest thing on the page and is held to the 3:1 large-text bar,
        // which is what it is used for.
        for (name, p) in [("dark", &DARK), ("light", &LIGHT)] {
            for (label, fg, bg, min) in [
                ("text on page", p.text, p.bg, 4.5),
                ("text on card", p.text, p.panel, 4.5),
                ("text on control", p.text, p.panel2, 4.5),
                ("text on sidebar", p.text, p.sidebar, 4.5),
                ("muted on page", p.muted, p.bg, 4.5),
                ("muted on card", p.muted, p.panel, 4.5),
                ("faint on page", p.faint, p.bg, 3.0),
                ("accent on page", p.accent, p.bg, 3.0),
                ("accent on card", p.accent, p.panel, 3.0),
                // The primary button: the most important control on any page.
                ("on_accent on accent", p.on_accent, p.accent, 4.5),
                ("fix on its tint", p.fix, p.fix_soft, 4.5),
                ("ok on its tint", p.ok, p.ok_soft, 4.5),
                ("info on its tint", p.info, p.info_soft, 4.5),
            ] {
                let got = contrast(fg, bg);
                assert!(
                    got >= min,
                    "{name}: {label} is {got:.2}:1, needs {min}:1"
                );
            }
        }
    }

    /// The two palettes must actually differ, in the direction they claim.
    #[test]
    fn light_is_light_and_dark_is_dark() {
        assert!(
            luminance(LIGHT.bg) > 0.6,
            "the light page is not light: {:.3}",
            luminance(LIGHT.bg)
        );
        assert!(
            luminance(DARK.bg) < 0.1,
            "the dark page is not dark: {:.3}",
            luminance(DARK.bg)
        );
        // Cards sit above the page in light mode and the page is grey so they
        // can; inverting that would flatten the whole hierarchy.
        assert!(luminance(LIGHT.panel) > luminance(LIGHT.bg));
        assert!(luminance(DARK.panel) > luminance(DARK.bg));
    }

    /// Switching is what every colour accessor reads, so it has to stick.
    #[test]
    fn the_active_palette_switches() {
        set_palette(true);
        assert!(is_light());
        assert_eq!(text(), LIGHT.text);
        set_palette(false);
        assert!(!is_light());
        assert_eq!(text(), DARK.text);
    }
}
