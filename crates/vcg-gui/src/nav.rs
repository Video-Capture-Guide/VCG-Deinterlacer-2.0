//! The wizard's steps and the sidebar that navigates them.
//!
//! A step is an enum rather than the Python's list index, so an unhandled one
//! is a compile error rather than a blank page.
//!
//! The visible step list depends on the mode. In the two simple modes it is
//! four entries; in Full control the eleven per-filter pages sit under one
//! "Settings" heading as indented sub-items. The step counter is derived from
//! that same list, so the sidebar and "Step N of M" cannot disagree.

use eframe::egui::{self, RichText};

use vcg_core::config::Mode;

use crate::theme;

/// A wizard page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    /// Hidden from the sidebar and from the step count.
    Welcome,
    ChooseVideo,
    CheckVideo,
    Crop,
    // ── The Settings group, shown only in Full control ──
    Trim,
    Levels,
    ColorTint,
    Noise,
    Halos,
    ColorBleeding,
    Upscale,
    Audio,
    Watermark,
    Dithering,
    FilmGrain,
    Export,
}

impl Step {
    /// Every step in walking order.
    pub const ALL: [Step; 16] = [
        Step::Welcome,
        Step::ChooseVideo,
        Step::CheckVideo,
        Step::Crop,
        Step::Trim,
        Step::Levels,
        Step::ColorTint,
        Step::Noise,
        Step::Halos,
        Step::ColorBleeding,
        Step::Upscale,
        Step::Audio,
        Step::Watermark,
        Step::Dithering,
        Step::FilmGrain,
        Step::Export,
    ];

    /// The eleven pages under "Settings".
    pub const SETTINGS: [Step; 11] = [
        Step::Trim,
        Step::Levels,
        Step::ColorTint,
        Step::Noise,
        Step::Halos,
        Step::ColorBleeding,
        Step::Upscale,
        Step::Audio,
        Step::Watermark,
        Step::Dithering,
        Step::FilmGrain,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Step::Welcome => "Welcome",
            Step::ChooseVideo => "Choose video",
            Step::CheckVideo => "Check video",
            Step::Crop => "Crop",
            Step::Trim => "Trim",
            Step::Levels => "Levels",
            Step::ColorTint => "Color tint",
            Step::Noise => "Noise",
            Step::Halos => "Halos",
            Step::ColorBleeding => "Color bleeding",
            Step::Upscale => "Upscale",
            Step::Audio => "Audio",
            Step::Watermark => "Watermark",
            Step::FilmGrain => "Film grain",
            Step::Dithering => "Smooth gradient fix",
            Step::Export => "Export",
        }
    }

    /// Which pages are implemented. The rest render a placeholder.
    pub fn is_built(self) -> bool {
        matches!(self, Step::Welcome | Step::ChooseVideo | Step::CheckVideo)
    }

    /// A page under "Settings", shown only in Full control.
    pub fn is_setting(self) -> bool {
        Self::SETTINGS.contains(&self)
    }

    /// Whether this step exists at all in `mode`.
    pub fn visible_in(self, mode: Mode) -> bool {
        if self == Step::Welcome {
            return false;
        }
        !self.is_setting() || mode.shows_filter_pages()
    }

    /// The steps in walking order for `mode`, excluding Welcome.
    pub fn visible(mode: Mode) -> Vec<Step> {
        Self::ALL
            .iter()
            .copied()
            .filter(|s| s.visible_in(mode))
            .collect()
    }

    pub fn next_in(self, mode: Mode) -> Option<Step> {
        if self == Step::Welcome {
            return Some(Step::ChooseVideo);
        }
        let steps = Self::visible(mode);
        let i = steps.iter().position(|s| *s == self)?;
        steps.get(i + 1).copied()
    }

    pub fn prev_in(self, mode: Mode) -> Option<Step> {
        if self == Step::Welcome {
            return None;
        }
        let steps = Self::visible(mode);
        let i = steps.iter().position(|s| *s == self)?;
        // No Back on step 1: Welcome is not somewhere the user returns to.
        if i == 0 { None } else { steps.get(i - 1).copied() }
    }

    /// The step counter, derived from the same list the sidebar draws.
    ///
    /// Full control reads "of up to N", because the Settings pages can all be
    /// skipped — and because jumping from "of 4" to "of 15" on choosing a mode
    /// reads as the app having got longer rather than more capable.
    pub fn counter_label(self, mode: Mode) -> String {
        if self == Step::Welcome {
            return String::new();
        }
        let steps = Self::visible(mode);
        let position = steps.iter().position(|s| *s == self).unwrap_or(0) + 1;
        if mode.shows_filter_pages() {
            format!("Step {position} of up to {}", steps.len())
        } else {
            format!("Step {position} of {}", steps.len())
        }
    }

    /// The next step's name, so the forward button can be labeled with it.
    pub fn next_title(self, mode: Mode) -> Option<&'static str> {
        self.next_in(mode).map(|n| n.title())
    }
}

/// What the user asked of the sidebar this frame.
///
/// An enum rather than `Option<Step>` because About is not a step and never
/// can be: it has no place in the numbering, and making it one would put it
/// in "Step N of M" and in the forward/back walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarAction {
    None,
    Goto(Step),
    OpenAbout,
    OpenQueue,
}

/// Draw the sidebar and report what was clicked.
pub fn sidebar(
    ui: &mut egui::Ui,
    current: Step,
    visited: &[Step],
    reachable: bool,
    mode: Mode,
    queue: &QueueBadge,
    current_is_queue: bool,
) -> SidebarAction {
    let mut clicked = None;
    let mut about = false;
    let mut open_queue = false;

    ui.add_space(20.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.label(RichText::new("VCG Deinterlacer").size(17.0).color(theme::text()));
    });
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.label(
            RichText::new(format!("v{}", crate::VERSION))
                .size(theme::FONT_TINY)
                .color(theme::accent()),
        );
    });
    ui.add_space(24.0);

    // Numbered and flat: the circles carry the sequence, so the old
    // SELECT FILE / SOURCE / ADVANCED headings are gone.
    let mut number = 0;
    let mut settings_drawn = false;

    for step in Step::visible(mode) {
        if step.is_setting() {
            // The group gets one numbered heading, then indented sub-items.
            if !settings_drawn {
                settings_drawn = true;
                number += 1;
                nav_row(
                    ui,
                    RowSpec {
                        step: None,
                        label: "Settings",
                        number,
                        sub: false,
                    },
                    current,
                    visited,
                    reachable,
                );
                for sub in Step::SETTINGS {
                    if let Some(c) = nav_row(
                        ui,
                        RowSpec {
                            step: Some(sub),
                            label: sub.title(),
                            number: 0,
                            sub: true,
                        },
                        current,
                        visited,
                        reachable,
                    ) {
                        clicked = Some(c);
                    }
                }
            }
            continue;
        }

        number += 1;
        if let Some(c) = nav_row(
            ui,
            RowSpec {
                step: Some(step),
                label: step.title(),
                number,
                sub: false,
            },
            current,
            visited,
            reachable,
        ) {
            clicked = Some(c);
        }
    }

    // The queue and About sit at the foot of the sidebar, below everything
    // numbered, so they read as "not part of the sequence" rather than as
    // last steps.
    //
    // The card is always drawn, even with nothing in it. It used to appear
    // only when the queue had something in it, which meant the one place that
    // tells you what is waiting and how long it will take was invisible
    // exactly when you were trying to find it — and after the modal became a
    // page there was no other way in at all.
    ui.add_space(18.0);
    let card_top = ui.cursor().top();

    ui.horizontal(|ui| {
        ui.add_space(12.0);
        egui::Frame::NONE
            .fill(if current_is_queue { theme::panel() } else { theme::panel2() })
            .corner_radius(theme::RADIUS_SMALL)
            .inner_margin(egui::Margin::symmetric(11, 9))
            .stroke(egui::Stroke::new(
                1.0,
                if current_is_queue { theme::accent() } else { theme::line() },
            ))
            .show(ui, |ui| {
                // **Vertical, explicitly.** A Frame inside `ui.horizontal`
                // inherits that horizontal layout, so without this the title,
                // the badge and the status line were laid out as one row and
                // ran off the end of a 205px sidebar — which is why the status
                // read "Fi".
                ui.vertical(|ui| {
                    // 205 sidebar − 12 indent − 22 frame margins − a little
                    // slack for the stroke.
                    ui.set_width(155.0);

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Queue")
                                .size(theme::FONT_BODY)
                                .color(theme::text()),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                // What is still to come, which is what someone
                                // glancing at the sidebar wants to know — not
                                // how many have been done.
                                let (text, fill) = if queue.failed > 0 {
                                    (format!("{} failed", queue.failed), theme::fix())
                                } else if queue.left > 0 {
                                    (format!("{} left", queue.left), theme::accent())
                                } else {
                                    (String::new(), theme::accent())
                                };
                                if !text.is_empty() {
                                    egui::Frame::NONE
                                        .fill(fill)
                                        .corner_radius(9.0)
                                        .inner_margin(egui::Margin::symmetric(7, 2))
                                        .show(ui, |ui| {
                                            ui.label(
                                                RichText::new(text)
                                                    .size(theme::FONT_TINY)
                                                    .color(egui::Color32::WHITE),
                                            );
                                        });
                                }
                            },
                        );
                    });

                    if queue.running {
                        ui.add_space(7.0);
                        ui.add(
                            egui::ProgressBar::new(queue.fraction)
                                .desired_height(4.0)
                                .corner_radius(2.0),
                        );
                    }
                    ui.add_space(5.0);
                    // Wrapped, not truncated: "Finished · needs attention" is
                    // two lines in this width and both of them matter.
                    ui.label(
                        RichText::new(&queue.status)
                            .size(theme::FONT_TINY)
                            .color(theme::faint()),
                    );
                });
            });
    });

    // The whole card is the target, which is a kinder hit area than a line of
    // link text for the older users the brief has in mind. Measured from the
    // cursor rather than guessed, so it tracks however tall the card grows.
    let card_rect = egui::Rect::from_min_max(
        egui::pos2(ui.min_rect().left() + 12.0, card_top),
        egui::pos2(ui.min_rect().left() + 193.0, ui.cursor().top()),
    );
    let response = ui.interact(
        card_rect,
        ui.id().with("queue-card"),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        open_queue = true;
    }

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        if theme::link(ui, "About & updates").clicked() {
            about = true;
        }
    });

    if open_queue {
        SidebarAction::OpenQueue
    } else if about {
        SidebarAction::OpenAbout
    } else if let Some(step) = clicked {
        SidebarAction::Goto(step)
    } else {
        SidebarAction::None
    }
}

/// What the sidebar needs to draw its queue card.
///
/// Passed in rather than read from `App` because `nav` draws the sidebar
/// before the page and must not depend on the rest of the interface.
#[derive(Debug, Clone, Default)]
pub struct QueueBadge {
    /// Still to come — the running job plus whatever is behind it.
    pub left: usize,
    pub failed: usize,
    pub running: bool,
    /// Progress across the whole batch, 0..1.
    pub fraction: f32,
    /// One short line: "Running · 3 of 7" or "Finished".
    pub status: String,
}

struct RowSpec<'a> {
    /// `None` for the Settings heading, which is not itself a page.
    step: Option<Step>,
    label: &'a str,
    number: usize,
    sub: bool,
}

fn nav_row(
    ui: &mut egui::Ui,
    spec: RowSpec<'_>,
    current: Step,
    visited: &[Step],
    reachable: bool,
) -> Option<Step> {
    let is_current = spec.step == Some(current);
    let is_visited = spec.step.is_some_and(|s| visited.contains(&s));
    let enabled = spec.step.is_some() && (reachable || spec.step == Some(Step::ChooseVideo));

    let height = if spec.sub { 26.0 } else { 36.0 };
    let full_width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(full_width, height),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );

    let text_color = if is_current {
        theme::text()
    } else if !enabled || spec.sub {
        theme::faint()
    } else {
        theme::muted()
    };

    if ui.is_rect_visible(rect) {
        if is_current {
            ui.painter().rect_filled(rect, 0.0, theme::accent_soft());
            let bar = egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height()));
            ui.painter().rect_filled(bar, 0.0, theme::accent());
        } else if response.hovered() && enabled {
            ui.painter().rect_filled(rect, 0.0, theme::panel());
        }

        if !spec.sub {
            step_circle(
                ui,
                egui::Rect::from_center_size(
                    egui::pos2(rect.left() + 27.0, rect.center().y),
                    egui::vec2(22.0, 22.0),
                ),
                spec.number,
                is_current,
                is_visited,
                enabled,
            );
        }

        ui.painter().text(
            egui::pos2(rect.left() + 48.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            spec.label,
            egui::FontId::proportional(if spec.sub {
                theme::FONT_TINY
            } else {
                theme::FONT_BODY
            }),
            text_color,
        );
    }

    theme::focus_ring(ui, &response, theme::RADIUS_SMALL);
    if response.clicked() { spec.step } else { None }
}

/// A numbered circle, showing a tick once the step has been visited.
fn step_circle(
    ui: &egui::Ui,
    rect: egui::Rect,
    number: usize,
    is_current: bool,
    is_visited: bool,
    enabled: bool,
) {
    let outline = if is_current {
        theme::accent()
    } else if is_visited {
        theme::ok()
    } else {
        theme::line()
    };
    let p = ui.painter();
    p.circle_stroke(rect.center(), 11.0, egui::Stroke::new(1.0, outline));

    if is_visited && !is_current {
        // Drawn, not typed: the bundled fonts carry no tick, and a missing
        // glyph renders as an empty box.
        theme::icon::check(
            p,
            egui::Rect::from_center_size(rect.center(), egui::vec2(13.0, 13.0)),
            theme::ok(),
        );
    } else {
        let color = if is_current {
            theme::accent()
        } else if enabled {
            theme::muted()
        } else {
            theme::faint()
        };
        p.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            number.to_string(),
            egui::FontId::proportional(11.0),
            color,
        );
    }
}

