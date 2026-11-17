//! The welcome window: five short steps shown once, the first time the app
//! starts (until `onboarded` is saved), and again from Help → Welcome… or
//! Settings → General. Every step can be skipped and every choice is the
//! same setting the Settings window changes, stored in the same place: the
//! theme in the preferences, the team's details in `report.txt` and the
//! history's "Measured by", the catalogue download through the app's own.
//!
//! Its text is the worked example of [`crate::i18n`]: every string goes
//! through `tr`.

use eframe::egui::{
    self, Align, Align2, Button, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, Label,
    Layout, Margin, Rect, RichText, Sense, Stroke, UiBuilder, pos2, vec2,
};

use super::{App, history_ui, mpc_ui, paint_logo};
use crate::capture::Capture;
use crate::i18n::{self, tr, tr_args};
use crate::known::{self, Catalogue};
use crate::prefs::{KnownSource, Prefs, ThemeChoice};
use crate::theme::{self, Palette, S1, S2, S3, S4, S5, palette};
use crate::update;

const WINDOW: [f32; 2] = [760.0, 480.0];
/// The team step's label column, and its fields (as wide as Settings draws them).
const LABEL: f32 = 128.0;
const FIELD: f32 = 360.0;
const SIDE: f32 = 184.0;
const FOOTER: f32 = 56.0;
pub const TRANSLATING_URL: &str =
    "https://github.com/alidevecioglu/OpenCeres/blob/main/TRANSLATING.md";
/// The button that writes the MPC report, as the side panel names it.
const REVIEW_BUTTON: &str = "Review & copy report";

/// A `Custom` row of the settings registry.
type DrawSetting = fn(&mut App, &mut egui::Ui);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Language,
    Appearance,
    Team,
    Known,
    Tips,
}

impl Step {
    const ALL: [Step; 5] = [
        Step::Language,
        Step::Appearance,
        Step::Team,
        Step::Known,
        Step::Tips,
    ];

    fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn title(self) -> &'static str {
        match self {
            Step::Language => tr("onboarding.language.title"),
            Step::Appearance => tr("onboarding.appearance.title"),
            Step::Team => tr("onboarding.team.title"),
            Step::Known => tr("onboarding.known.title"),
            Step::Tips => tr("onboarding.tips.title"),
        }
    }

    /// For `OPENCERES_SCREENSHOT_STATE=onboarding=<key>`: 1–5 or the name.
    fn from_key(key: &str) -> Option<Step> {
        if let Ok(n) = key.parse::<usize>() {
            return n.checked_sub(1).and_then(|i| Step::ALL.get(i).copied());
        }
        let names = ["language", "appearance", "team", "known", "tips"];
        let i = names.iter().position(|n| n.eq_ignore_ascii_case(key))?;
        Some(Step::ALL[i])
    }
}

#[derive(Default)]
pub struct Onboarding {
    /// The step shown; `None` while the window is closed.
    step: Option<Step>,
    /// A download started here: use MPCORB for known objects once it has
    /// arrived (until then, and if it fails, the online lookup stays).
    mpcorb_when_downloaded: bool,
}

impl Onboarding {
    /// Open on a first start. Screenshots show it only when asked to.
    pub fn start(prefs: &Prefs, capture: Option<&Capture>) -> Onboarding {
        let step = match capture {
            Some(c) => c
                .state
                .onboarding
                .as_deref()
                .map(|k| Step::from_key(k).unwrap_or(Step::Language)),
            None => (!prefs.onboarded).then_some(Step::Language),
        };
        Onboarding {
            step,
            mpcorb_when_downloaded: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.step.is_some()
    }
}

impl App {
    pub(super) fn show_onboarding(&mut self) {
        self.onboarding.step = Some(Step::Language);
        self.settings_open = false;
        self.searches.window_open = false;
        self.team.window_open = false;
        self.help_open = false;
    }

    fn close_onboarding(&mut self) {
        self.onboarding.step = None;
        self.prefs.onboarded = true;
    }

    fn set_language(&mut self, code: &'static str) {
        self.prefs.language = code;
        i18n::set_language(code);
    }

    pub(super) fn onboarding_window(&mut self, ctx: &egui::Context) {
        if self.onboarding.mpcorb_when_downloaded && self.download.is_none() {
            self.onboarding.mpcorb_when_downloaded = false;
            if self.download_error.is_none() {
                self.prefs.known_source = KnownSource::Mpcorb;
            }
        }
        let Some(step) = self.onboarding.step else {
            return;
        };
        let screen = ctx.content_rect();
        let size = vec2(
            (screen.width() - 2.0 * S5).min(WINDOW[0]),
            (screen.height() - 2.0 * S5).min(WINDOW[1]),
        )
        .max(vec2(360.0, 300.0));
        let style = ctx.global_style();
        let modal = egui::Modal::new(Id::new("onboarding"))
            .frame(
                Frame::popup(&style)
                    .inner_margin(Margin::ZERO)
                    .corner_radius(theme::RADIUS + 2),
            )
            .backdrop_color(Color32::from_black_alpha(70))
            .show(ctx, |ui| self.onboarding_body(ui, step, size));
        // Escape skips; a stray click beside the window does not.
        let outside = modal.backdrop_response.clicked();
        if !outside && modal.should_close() {
            self.close_onboarding();
        }
    }

    fn onboarding_body(&mut self, ui: &mut egui::Ui, step: Step, size: egui::Vec2) {
        let p = palette(ui);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let side_w = if size.x < 560.0 { 0.0 } else { SIDE };
        let side = Rect::from_min_max(rect.min, pos2(rect.min.x + side_w, rect.max.y));
        let page = Rect::from_min_max(pos2(side.max.x, rect.min.y), rect.max);
        let footer = Rect::from_min_max(pos2(page.min.x, page.max.y - FOOTER), page.max);
        let body = Rect::from_min_max(page.min, pos2(page.max.x, footer.min.y));
        let painter = ui.painter().clone();
        let r = theme::RADIUS + 2;
        if side_w > 0.0 {
            let corners = CornerRadius {
                nw: r,
                sw: r,
                ..Default::default()
            };
            painter.rect_filled(side, corners, p.inset);
            painter.vline(side.max.x, side.y_range(), Stroke::new(1.0, p.border));
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(side.shrink(S4))
                    .layout(Layout::top_down(Align::Min)),
                |ui| self.onboarding_steps(ui, step),
            );
        }
        painter.hline(page.x_range(), footer.min.y, Stroke::new(1.0, p.border));
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(body)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("onboarding", step.index()))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        Frame::new()
                            .inner_margin(Margin {
                                left: S5 as i8,
                                right: S5 as i8,
                                top: S5 as i8,
                                bottom: S4 as i8,
                            })
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = S2;
                                self.onboarding_page(ui, step);
                            });
                    });
            },
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(footer.shrink2(vec2(S5, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| self.onboarding_footer(ui, step),
        );
    }

    /// The logo, then the steps; a step is a button back to it.
    fn onboarding_steps(&mut self, ui: &mut egui::Ui, step: Step) {
        let p = palette(ui);
        ui.horizontal(|ui| {
            let (logo, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
            paint_logo(ui, logo);
            ui.add_space(S1);
            ui.label(
                RichText::new("OpenCeres")
                    .family(theme::semibold())
                    .size(15.0)
                    .color(p.text),
            );
        });
        ui.add_space(S5);
        ui.spacing_mut().item_spacing.y = S1;
        for s in Step::ALL {
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            let current = s == step;
            if current {
                ui.painter().rect_filled(rect, theme::RADIUS, p.accent_weak);
            } else if response.hovered() {
                ui.painter().rect_filled(rect, theme::RADIUS, p.hover);
            }
            let centre = pos2(rect.min.x + S2 + 9.0, rect.center().y);
            let (fill, number) = if current || s.index() < step.index() {
                (p.accent, p.accent_text)
            } else {
                (Color32::TRANSPARENT, p.muted)
            };
            ui.painter()
                .circle(centre, 9.0, fill, Stroke::new(1.0, p.border_strong));
            ui.painter().text(
                centre,
                Align2::CENTER_CENTER,
                (s.index() + 1).to_string(),
                FontId::proportional(11.0),
                number,
            );
            ui.painter().text(
                pos2(centre.x + 9.0 + S2, rect.center().y),
                Align2::LEFT_CENTER,
                s.title(),
                FontId::proportional(13.0),
                if current { p.text } else { p.muted },
            );
            if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                self.onboarding.step = Some(s);
            }
        }
    }

    fn onboarding_page(&mut self, ui: &mut egui::Ui, step: Step) {
        let p = palette(ui);
        if step == Step::Language {
            ui.label(
                RichText::new(tr("onboarding.title"))
                    .family(theme::semibold())
                    .size(20.0)
                    .color(p.text),
            );
            wrapped(ui, tr("onboarding.subtitle"), p.muted);
            ui.add_space(S3);
        }
        ui.label(
            RichText::new(tr_args(
                "onboarding.step",
                &[
                    ("n", &(step.index() + 1).to_string()),
                    ("total", &Step::ALL.len().to_string()),
                ],
            ))
            .small()
            .color(p.muted),
        );
        ui.label(RichText::new(step.title()).heading().color(p.text));
        match step {
            Step::Language => self.language_step(ui),
            Step::Appearance => self.appearance_step(ui),
            Step::Team => self.team_step(ui),
            Step::Known => self.known_step(ui),
            Step::Tips => tips_step(ui),
        }
    }

    fn language_step(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        wrapped(ui, tr("onboarding.language.body"), p.muted);
        ui.add_space(S2);
        let width = ui.available_width().min(360.0);
        for l in i18n::LANGUAGES {
            let selected = l.code == self.prefs.language;
            let mut text = l.endonym.to_owned();
            if l.english != l.endonym {
                text += &format!(" · {}", l.english);
            }
            let row = Button::selectable(selected, RichText::new(text).color(p.text))
                .min_size(vec2(width, 30.0));
            let response = ui.add(row);
            if let Some(note) = completeness_note(l) {
                response.clone().on_hover_text(note);
            }
            if response.clicked() && !selected {
                self.set_language(l.code);
                ui.ctx().request_repaint();
            }
        }
        ui.add_space(S2);
        translate_link(ui);
    }

    fn appearance_step(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        wrapped(ui, tr("onboarding.appearance.body"), p.muted);
        ui.add_space(S3);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = S3;
            for (choice, key) in [
                (ThemeChoice::System, tr("onboarding.appearance.system")),
                (ThemeChoice::Light, tr("onboarding.appearance.light")),
                (ThemeChoice::Dark, tr("onboarding.appearance.dark")),
            ] {
                if theme_card(ui, choice, key, self.prefs.theme == choice) {
                    self.prefs.theme = choice;
                    // The theme is applied at the start of a frame.
                    ui.ctx().request_repaint();
                }
            }
        });
    }

    fn team_step(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        wrapped(ui, tr("onboarding.team.body"), p.muted);
        let fields: [(&str, &str, DrawSetting); 4] = [
            (
                tr("onboarding.team.code"),
                tr("onboarding.team.code_hint"),
                mpc_ui::code_setting,
            ),
            (
                tr("onboarding.team.observers"),
                tr("onboarding.team.observers_hint"),
                mpc_ui::observers_setting,
            ),
            (
                tr("onboarding.team.prefix"),
                tr("onboarding.team.prefix_hint"),
                mpc_ui::prefix_setting,
            ),
            (
                tr("onboarding.team.measured_by"),
                tr("onboarding.team.measured_by_hint"),
                history_ui::name_setting,
            ),
        ];
        ui.add_space(S2);
        egui::Grid::new("onboarding_team")
            .num_columns(2)
            .spacing(vec2(S4, S3))
            .min_col_width(LABEL)
            .max_col_width(FIELD)
            .show(ui, |ui| {
                for (label, hint, draw) in fields {
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.add_space(3.0);
                        ui.add(Label::new(RichText::new(label).color(p.text)).wrap());
                    });
                    ui.vertical(|ui| {
                        ui.set_max_width(FIELD);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        draw(self, ui);
                        ui.add(Label::new(RichText::new(hint).small().color(p.muted)).wrap());
                    });
                    ui.end_row();
                }
            });
    }

    fn team_is_empty(&self) -> bool {
        let c = &self.report_config;
        [&c.code, &c.observers, &c.prefix, &self.searches.config.name]
            .iter()
            .all(|s| s.trim().is_empty())
    }

    fn known_step(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        wrapped(ui, tr("onboarding.known.body"), p.muted);
        ui.add_space(S2);
        let present = matches!(
            self.catalogue.get_or_insert_with(known::catalogue_info),
            Catalogue::Present { .. }
        );
        if self.download.is_some() {
            wrapped(ui, tr("onboarding.known.downloading"), p.text);
            self.download_controls(ui);
        } else if present && self.prefs.known_source == KnownSource::Mpcorb {
            wrapped(ui, tr("onboarding.known.downloaded"), p.success);
        } else if present {
            wrapped(ui, tr("onboarding.known.present"), p.text);
            if ui.button(tr("onboarding.known.use")).clicked() {
                self.prefs.known_source = KnownSource::Mpcorb;
            }
        } else {
            if let Some(e) = &self.download_error {
                wrapped(ui, e, p.danger);
            }
            let download =
                Button::new(RichText::new(tr("onboarding.known.download")).color(p.accent_text))
                    .fill(p.accent)
                    .stroke(Stroke::NONE);
            if ui.add(download).clicked() {
                self.start_download(ui.ctx());
                self.onboarding.mpcorb_when_downloaded = true;
            }
            wrapped(ui, tr("onboarding.known.download_hint"), p.muted);
            ui.add_space(S2);
            wrapped(ui, tr("onboarding.known.later_hint"), p.muted);
        }
    }

    fn onboarding_footer(&mut self, ui: &mut egui::Ui, step: Step) {
        let p = palette(ui);
        if step != Step::Tips && ui.button(tr("onboarding.skip")).clicked() {
            self.close_onboarding();
            return;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let later = match step {
                Step::Team => self.team_is_empty(),
                Step::Known => {
                    self.download.is_none()
                        && !matches!(self.catalogue, Some(Catalogue::Present { .. }))
                }
                _ => false,
            };
            let label = match step {
                Step::Tips => tr("onboarding.finish"),
                _ if later => tr("onboarding.later"),
                _ => tr("onboarding.next"),
            };
            let primary = Button::new(RichText::new(label).color(p.accent_text))
                .fill(p.accent)
                .stroke(Stroke::NONE)
                .min_size(vec2(88.0, 0.0));
            if ui.add(primary).clicked() {
                match Step::ALL.get(step.index() + 1) {
                    Some(next) => self.onboarding.step = Some(*next),
                    None => self.close_onboarding(),
                }
            }
            if step.index() > 0 && ui.button(tr("onboarding.back")).clicked() {
                self.onboarding.step = Some(Step::ALL[step.index() - 1]);
            }
        });
    }
}

fn tips_step(ui: &mut egui::Ui) {
    let p = palette(ui);
    let tips = [
        tr_args("onboarding.tips.blink", &[("key", "Space")]),
        tr("onboarding.tips.mark").to_owned(),
        tr_args("onboarding.tips.report", &[("button", REVIEW_BUTTON)]),
    ];
    ui.add_space(S2);
    for (i, tip) in tips.iter().enumerate() {
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), 12.0, p.accent_weak);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                (i + 1).to_string(),
                FontId::new(13.0, theme::semibold()),
                p.accent,
            );
            ui.add_space(S2);
            ui.vertical(|ui| {
                ui.add_space(3.0);
                ui.add(Label::new(RichText::new(tip).color(p.text)).wrap());
            });
        });
        ui.add_space(S2);
    }
    ui.add_space(S2);
    wrapped(
        ui,
        &tr_args("onboarding.tips.shortcuts", &[("key", "?")]),
        p.muted,
    );
}

fn wrapped(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add(Label::new(RichText::new(text).color(color)).wrap());
}

/// "73 % translated", for a language that is not complete.
fn completeness_note(l: &i18n::Language) -> Option<String> {
    let c = i18n::completeness(l);
    (c < 1.0).then(|| {
        tr_args(
            "language.complete",
            &[("percent", &format!("{:.0}", c * 100.0))],
        )
    })
}

fn translate_link(ui: &mut egui::Ui) {
    let p = palette(ui);
    let link = ui.link(RichText::new(tr("settings.help_translate")).color(p.accent));
    if link.on_hover_text(TRANSLATING_URL).clicked() {
        update::open_url(TRANSLATING_URL);
    }
}

/// A small picture of the window in that theme (System: half of each);
/// true when clicked.
fn theme_card(ui: &mut egui::Ui, choice: ThemeChoice, label: &str, selected: bool) -> bool {
    let p = palette(ui);
    let size = vec2(132.0, 86.0);
    let (rect, response) = ui.allocate_exact_size(size + vec2(0.0, S5), Sense::click());
    let card = Rect::from_min_size(rect.min, size);
    let painter = ui.painter();
    let draw = |painter: &egui::Painter, clip: Rect, t: &Palette| {
        let painter = painter.with_clip_rect(clip);
        painter.rect_filled(card, theme::RADIUS, t.panel);
        let stage = Rect::from_min_max(card.min + vec2(S2, 18.0), card.max - vec2(S2, S2));
        painter.rect_filled(stage, 3, theme::STAGE);
        for (i, x) in [0.3, 0.5, 0.7].into_iter().enumerate() {
            let at = pos2(
                stage.min.x + stage.width() * x,
                stage.max.y - stage.height() * (0.25 + 0.25 * i as f32),
            );
            painter.circle_filled(at, 3.0, theme::STREAK);
        }
        for (i, w) in [22.0, 14.0, 14.0].into_iter().enumerate() {
            let x = card.min.x + S2 + i as f32 * 26.0;
            let bar = Rect::from_min_size(pos2(x, card.min.y + 7.0), vec2(w, 5.0));
            painter.rect_filled(bar, 2, t.muted);
        }
    };
    match choice {
        ThemeChoice::Light => draw(painter, card, &theme::LIGHT),
        ThemeChoice::Dark => draw(painter, card, &theme::DARK),
        ThemeChoice::System => {
            let mid = card.center().x;
            draw(painter, card.with_max_x(mid), &theme::LIGHT);
            draw(painter, card.with_min_x(mid), &theme::DARK);
        }
    }
    let stroke = if selected {
        Stroke::new(2.0, p.accent)
    } else if response.hovered() {
        Stroke::new(1.0, p.border_strong)
    } else {
        Stroke::new(1.0, p.border)
    };
    painter.rect_stroke(card, theme::RADIUS, stroke, egui::StrokeKind::Outside);
    painter.text(
        pos2(card.center().x, card.max.y + S2),
        Align2::CENTER_TOP,
        label,
        FontId::proportional(13.0),
        if selected { p.text } else { p.muted },
    );
    response.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

// ---------------------------------------------------------------------------
// Settings → General

pub fn language_setting(app: &mut App, ui: &mut egui::Ui) {
    let current = i18n::current();
    let mut code = current.code;
    egui::ComboBox::from_id_salt("language")
        .width(200.0)
        .selected_text(current.endonym)
        .show_ui(ui, |ui| {
            for l in i18n::LANGUAGES {
                let mut text = l.endonym.to_owned();
                if let Some(note) = completeness_note(l) {
                    text += &format!(" · {note}");
                }
                ui.selectable_value(&mut code, l.code, text);
            }
        });
    if code != current.code {
        app.set_language(code);
    }
    translate_link(ui);
}

pub fn welcome_setting(app: &mut App, ui: &mut egui::Ui) {
    if ui.button(tr("settings.show_welcome")).clicked() {
        app.show_onboarding();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_by_number_or_name() {
        assert_eq!(Step::from_key("1"), Some(Step::Language));
        assert_eq!(Step::from_key("5"), Some(Step::Tips));
        assert_eq!(Step::from_key("known"), Some(Step::Known));
        assert_eq!(Step::from_key("0"), None);
        assert_eq!(Step::from_key("6"), None);
        assert_eq!(Step::from_key("bogus"), None);
    }

    #[test]
    fn shown_once_and_never_in_screenshots_unless_asked() {
        let fresh = Prefs::default();
        assert!(Onboarding::start(&fresh, None).is_open());
        let seen = Prefs {
            onboarded: true,
            ..Prefs::default()
        };
        assert!(!Onboarding::start(&seen, None).is_open());
        let shot = |state: &str| Capture {
            path: String::new(),
            state: crate::capture::State::parse(state),
            frames: 0,
            applied_at: None,
            requested: false,
            focus: None,
        };
        assert!(!Onboarding::start(&fresh, Some(&shot("dark"))).is_open());
        let o = Onboarding::start(&seen, Some(&shot("onboarding=team")));
        assert_eq!(o.step, Some(Step::Team));
        let o = Onboarding::start(&seen, Some(&shot("onboarding")));
        assert_eq!(o.step, Some(Step::Language));
    }
}
