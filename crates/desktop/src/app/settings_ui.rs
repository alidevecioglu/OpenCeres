//! The Settings window: a sidebar of categories, a search box and one page
//! per category, all drawn from the registry in [`crate::settings`]. Changes
//! apply as they are made; there is no Save button.
//!
//! The `Custom` rows of the registry are the `pub fn`s at the end.

use eframe::egui::{
    self, Align, Button, Color32, CursorIcon, FontId, Frame, Id, Label, Layout, Margin, Rect,
    Response, RichText, Sense, Stroke, TextEdit, UiBuilder, pos2, vec2,
};

use super::{App, hline, muted_wrapped, paint_logo, segmented, shortcut_table};
use crate::format;
use crate::icons::{self, Icon};
use crate::known::{self, Catalogue, Freshness};
use crate::settings::{self, Category, Entry, Kind};
use crate::theme::{self, S1, S2, S3, S4, S5, palette};
use crate::update;

const WINDOW: [f32; 2] = [760.0, 540.0];
const HEADER: f32 = 52.0;
/// Below this window width the sidebar shows icons only.
const NARROW: f32 = 600.0;
const CONTROL: f32 = 220.0;
const REPO: &str = "https://github.com/alidevecioglu/OpenCeres";

impl App {
    pub(super) fn settings_window(&mut self, ctx: &egui::Context, now: f64) {
        if !self.settings_open {
            return;
        }
        let before = self.prefs;
        let screen = ctx.content_rect();
        let size = vec2(
            (screen.width() - 2.0 * S5).min(WINDOW[0]),
            (screen.height() - 2.0 * S5).min(WINDOW[1]),
        )
        .max(vec2(320.0, 240.0));
        let style = ctx.global_style();
        let modal = egui::Modal::new(Id::new("settings"))
            .frame(
                Frame::popup(&style)
                    .inner_margin(Margin::ZERO)
                    .corner_radius(theme::RADIUS + 2),
            )
            .backdrop_color(Color32::from_black_alpha(70))
            .show(ctx, |ui| self.settings_body(ui, size));
        if modal.should_close() {
            self.settings_open = false;
        }
        self.apply_defaults(before, now);
    }

    fn settings_body(&mut self, ui: &mut egui::Ui, size: egui::Vec2) {
        let p = palette(ui);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let narrow = size.x < NARROW;
        let side_w = if narrow { 52.0 } else { 188.0 };
        let header = Rect::from_min_size(rect.min, vec2(size.x, HEADER));
        let side = Rect::from_min_max(
            pos2(rect.min.x, header.max.y),
            pos2(rect.min.x + side_w, rect.max.y),
        );
        let page = Rect::from_min_max(pos2(side.max.x, header.max.y), rect.max);
        let painter = ui.painter();
        let r = theme::RADIUS + 2;
        painter.rect_filled(
            side,
            egui::CornerRadius {
                sw: r,
                ..Default::default()
            },
            p.inset,
        );
        painter.hline(rect.x_range(), header.max.y, Stroke::new(1.0, p.border));
        painter.vline(side.max.x, side.y_range(), Stroke::new(1.0, p.border));

        ui.scope_builder(
            UiBuilder::new()
                .max_rect(header.shrink2(vec2(S4, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
            |ui| self.settings_header(ui),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(side.shrink(S2))
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for c in Category::ALL {
                    let selected = self.settings_search.is_empty() && c == self.settings_page;
                    if nav_item(ui, c, selected, narrow).clicked() {
                        self.settings_page = c;
                        self.settings_search.clear();
                        self.confirm_reset = false;
                    }
                }
            },
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(page)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                let salt = if self.settings_search.is_empty() {
                    self.settings_page.label()
                } else {
                    "search"
                };
                egui::ScrollArea::vertical()
                    .id_salt(("settings_page", salt))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        Frame::new()
                            .inner_margin(Margin {
                                left: S5 as i8,
                                right: S5 as i8,
                                top: S4 as i8,
                                bottom: S5 as i8,
                            })
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = S1;
                                if self.settings_search.trim().is_empty() {
                                    self.settings_page_content(ui);
                                } else {
                                    self.settings_search_results(ui);
                                }
                            });
                    });
            },
        );
    }

    fn settings_header(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        ui.label(RichText::new("Settings").heading().color(p.text));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icons::small_button(ui, Icon::Close, p.muted)
                .on_hover_text("Close (Esc)")
                .clicked()
            {
                self.settings_open = false;
            }
            ui.add_space(S2);
            let w = (ui.available_width() - 120.0).clamp(140.0, 260.0);
            ui.allocate_ui_with_layout(
                vec2(w, theme::TOOL + 2.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    search_box(
                        ui,
                        &mut self.settings_search,
                        "Search settings",
                        "settings_search",
                        w,
                    );
                },
            );
        });
    }

    fn settings_page_content(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let page = self.settings_page;
        ui.horizontal(|ui| {
            ui.label(RichText::new(page.label()).heading().color(p.text));
            if settings::entries(page).any(Entry::is_value) {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let changed = settings::is_changed(&self.prefs, page);
                    let reset = ui
                        .add_enabled(changed, Button::new("Reset this section"))
                        .on_hover_text("Put this page's settings back to their defaults");
                    if reset.clicked() {
                        settings::reset(&mut self.prefs, Some(page));
                    }
                });
            }
        });
        ui.add_space(S3);
        for (i, e) in settings::entries(page).enumerate() {
            if i > 0 {
                ui.add_space(S3);
                hline(ui, p.border);
                ui.add_space(S3);
            }
            self.settings_entry(ui, e);
        }
    }

    fn settings_search_results(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let query = self.settings_search.clone();
        let mut any = false;
        for c in Category::ALL {
            let hits: Vec<&Entry> = settings::entries(c).filter(|e| e.matches(&query)).collect();
            if hits.is_empty() {
                continue;
            }
            if any {
                ui.add_space(S4);
            }
            any = true;
            let open = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = S2;
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), r, c.icon(), p.muted);
                    ui.add(
                        Label::new(RichText::new(c.label()).strong().color(p.muted))
                            .sense(Sense::click()),
                    )
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text("Open this page")
                    .clicked()
                })
                .inner;
            if open {
                self.settings_page = c;
                self.settings_search.clear();
            }
            ui.add_space(S1);
            for (i, e) in hits.into_iter().enumerate() {
                if i > 0 {
                    ui.add_space(S2);
                    hline(ui, p.border);
                    ui.add_space(S2);
                }
                self.settings_entry(ui, e);
            }
        }
        if !any {
            muted_wrapped(ui, &format!("No settings match “{}”.", query.trim()));
        }
    }

    /// One entry: label and description, with its control on the right
    /// (below, when the page is narrow); custom rows draw underneath.
    fn settings_entry(&mut self, ui: &mut egui::Ui, e: &'static Entry) {
        let p = palette(ui);
        let text = |ui: &mut egui::Ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(e.label).color(p.text));
            if !e.description.is_empty() {
                ui.add(Label::new(RichText::new(e.description).small().color(p.muted)).wrap());
            }
        };
        if let Kind::Custom(draw) = e.kind {
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                text(ui);
                ui.add_space(S2);
                ui.spacing_mut().item_spacing.y = S2;
                draw(self, ui);
            });
            return;
        }
        let wide = ui.available_width() >= 460.0;
        if wide {
            ui.horizontal_top(|ui| {
                let text_w = ui.available_width() - CONTROL - S4;
                ui.allocate_ui_with_layout(vec2(text_w, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(text_w);
                    text(ui);
                });
                ui.add_space(S4 - ui.spacing().item_spacing.x);
                ui.allocate_ui_with_layout(
                    vec2(CONTROL, theme::TOOL),
                    Layout::right_to_left(Align::Min),
                    |ui| self.value_control(ui, e),
                );
            });
        } else {
            ui.vertical(|ui| {
                text(ui);
                ui.add_space(S1);
                self.value_control(ui, e);
            });
        }
    }

    fn value_control(&mut self, ui: &mut egui::Ui, e: &Entry) {
        match e.kind {
            Kind::Toggle { get, set } => {
                let mut on = get(&self.prefs);
                if switch(ui, &mut on).changed() {
                    set(&mut self.prefs, on);
                }
            }
            Kind::Choice {
                options, get, set, ..
            } => {
                let mut i = get(&self.prefs);
                if options.len() <= 5 {
                    let labels: Vec<_> = options
                        .iter()
                        .enumerate()
                        .map(|(i, o)| (i, o.label.to_owned()))
                        .collect();
                    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                        if segmented(ui, &mut i, &labels, CONTROL) {
                            set(&mut self.prefs, i);
                        }
                    });
                } else {
                    egui::ComboBox::from_id_salt(e.id)
                        .width(CONTROL - 2.0 * S2)
                        .selected_text(options[i].label)
                        .show_ui(ui, |ui| {
                            for (k, o) in options.iter().enumerate() {
                                ui.selectable_value(&mut i, k, o.label);
                            }
                        });
                    if i != get(&self.prefs) {
                        set(&mut self.prefs, i);
                    }
                }
            }
            Kind::Custom(_) => {}
        }
    }
}

/// A search field with a magnifier and a clear button, `w` points wide.
pub(super) fn search_box(ui: &mut egui::Ui, text: &mut String, hint: &str, id: &str, w: f32) {
    let p = palette(ui);
    Frame::new()
        .fill(p.inset)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::RADIUS)
        .inner_margin(Margin::symmetric(S2 as i8, 3))
        .show(ui, |ui| {
            ui.set_width(w - 2.0 * S2);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = S1;
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 20.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Search, p.muted);
                let clear = !text.is_empty();
                let edit_w = ui.available_width() - if clear { 24.0 } else { 0.0 };
                ui.add(
                    TextEdit::singleline(text)
                        .id(Id::new(id))
                        .hint_text(hint)
                        .frame(Frame::NONE)
                        .margin(Margin::ZERO)
                        .desired_width(edit_w),
                );
                if clear
                    && icons::small_button(ui, Icon::Close, p.muted)
                        .on_hover_text("Clear search")
                        .clicked()
                {
                    text.clear();
                }
            });
        });
}

/// A sidebar row: icon and label (icon only when narrow).
fn nav_item(ui: &mut egui::Ui, c: Category, selected: bool, narrow: bool) -> Response {
    let p = palette(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let painter = ui.painter();
    let radius = theme::RADIUS;
    if selected {
        painter.rect_filled(rect, radius, p.accent_weak);
    } else if response.hovered() {
        painter.rect_filled(rect, radius, p.hover);
    }
    let color = if selected { p.accent } else { p.text };
    let icon_at = if narrow {
        rect.center()
    } else {
        pos2(rect.min.x + S2 + 8.0, rect.center().y)
    };
    icons::paint(
        painter,
        Rect::from_center_size(icon_at, vec2(16.0, 16.0)),
        c.icon(),
        if selected { p.accent } else { p.muted },
    );
    if !narrow {
        let galley =
            painter.layout_no_wrap(c.label().to_owned(), FontId::proportional(13.0), color);
        let at = pos2(
            icon_at.x + 8.0 + S2 + 2.0,
            rect.center().y - galley.size().y / 2.0,
        );
        painter.galley(at, galley, color);
    }
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if narrow {
        response.on_hover_text(c.label())
    } else {
        response
    }
}

/// An on/off switch; `changed()` when clicked.
fn switch(ui: &mut egui::Ui, on: &mut bool) -> Response {
    let p = palette(ui);
    let (rect, mut response) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let off = p.muted.gamma_multiply(0.55);
    let track = Color32::from_rgba_premultiplied(
        lerp(off.r(), p.accent.r(), t),
        lerp(off.g(), p.accent.g(), t),
        lerp(off.b(), p.accent.b(), t),
        lerp(off.a(), p.accent.a(), t),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 10, track);
    let x = egui::lerp((rect.min.x + 10.0)..=(rect.max.x - 10.0), t);
    painter.circle_filled(pos2(x, rect.center().y), 7.5, Color32::WHITE);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8
}

fn small(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) {
    ui.add(Label::new(RichText::new(text.into()).small().color(color)).wrap());
}

fn spinner_line(ui: &mut egui::Ui, text: &str) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(13.0).color(p.muted));
        ui.label(RichText::new(text).color(p.muted));
    });
}

// ---------------------------------------------------------------------------
// Custom rows of the registry

pub fn data_folder(app: &mut App, ui: &mut egui::Ui) {
    let _ = app;
    let p = palette(ui);
    let Some(dir) = crate::prefs::data_dir() else {
        small(
            ui,
            "No data folder could be found on this computer.",
            p.danger,
        );
        return;
    };
    ui.add(
        Label::new(
            RichText::new(dir.display().to_string())
                .monospace()
                .size(11.5)
                .color(p.text),
        )
        .wrap()
        .selectable(true),
    );
    let label = if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(windows) {
        "Show in Explorer"
    } else {
        "Open folder"
    };
    if ui.button(label).clicked() {
        let _ = std::fs::create_dir_all(&dir);
        update::open_url(&dir.to_string_lossy());
    }
}

pub fn reset_all(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    if !app.confirm_reset {
        if ui.button("Reset all…").clicked() {
            app.confirm_reset = true;
        }
        return;
    }
    ui.horizontal(|ui| {
        let reset = Button::new(RichText::new("Reset all settings").color(p.accent_text))
            .fill(p.accent)
            .stroke(Stroke::NONE);
        if ui.add(reset).clicked() {
            settings::reset(&mut app.prefs, None);
            app.confirm_reset = false;
        }
        if ui.button("Cancel").clicked() {
            app.confirm_reset = false;
        }
    });
}

/// A small 3 × 3 grid with its box names.
pub fn grid_names(app: &mut App, ui: &mut egui::Ui) {
    let _ = app;
    let p = palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(168.0, 96.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, theme::RADIUS, theme::STAGE);
    let cell = rect.size() / 3.0;
    for r in 1..=3 {
        for c in 1..=3 {
            let box_rect = Rect::from_min_size(
                rect.min + vec2((c - 1) as f32 * cell.x, (r - 1) as f32 * cell.y),
                cell,
            );
            if (r, c) == (1, 2) {
                painter.rect_filled(box_rect, 0.0, theme::GRID_FILL);
            }
            painter.text(
                box_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("{r}x{c}"),
                FontId::proportional(11.5),
                theme::GRID_TEXT,
            );
        }
    }
    let stroke = Stroke::new(1.0, theme::GRID_LINE);
    for i in 1..3 {
        painter.vline(rect.min.x + i as f32 * cell.x, rect.y_range(), stroke);
        painter.hline(rect.x_range(), rect.min.y + i as f32 * cell.y, stroke);
    }
    let _ = p;
}

pub fn mpcorb(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let info = app.catalogue.get_or_insert_with(known::catalogue_info);
    let present = matches!(info, Catalogue::Present { .. });
    let status = match info {
        Catalogue::Missing => "Not downloaded yet".to_owned(),
        Catalogue::Present { date, bytes, .. } => format!(
            "{} · {:.0} MB",
            date.as_deref()
                .map_or_else(|| "Downloaded".to_owned(), |d| format!("Catalogue of {d}")),
            *bytes as f64 / 1e6
        ),
    };
    ui.label(RichText::new(status).color(p.text));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = S2;
        app.download_controls(ui);
        let checking = matches!(app.mpcorb_check, Some((_, true)));
        if present && app.download.is_none() && !checking {
            let check = ui
                .button("Check for update")
                .on_hover_text("Ask the Minor Planet Center whether there is a newer catalogue");
            if check.clicked() {
                app.start_mpcorb_check(ui.ctx(), true);
            }
        }
    });
    if matches!(app.mpcorb_check, Some((_, true))) {
        spinner_line(ui, "Checking for a newer catalogue…");
    } else if let Some(result) = &app.mpcorb_fresh {
        let when = |v: &known::Version| v.modified.map(format::unix_utc);
        match result {
            Ok(Freshness::Current(v)) => small(
                ui,
                match when(v) {
                    Some(t) => format!("Up to date — MPC catalogue of {t}"),
                    None => "Up to date".to_owned(),
                },
                p.muted,
            ),
            Ok(Freshness::Newer(v)) => small(
                ui,
                match when(v) {
                    Some(t) => format!("A newer catalogue is available ({t})"),
                    None => "A newer catalogue is available".to_owned(),
                },
                p.accent,
            ),
            Err(e) => small(ui, e.clone(), p.danger),
        }
    }
}

pub fn update_status(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    ui.label(RichText::new(format!("Version {}", update::CURRENT_VERSION)).color(p.text));
    let u = &app.updates;
    if u.pending.is_some() && u.manual {
        spinner_line(ui, "Checking for updates…");
    } else if let Some(r) = &u.found {
        small(ui, format!("Version {} is available", r.version), p.accent);
    } else if let Some(e) = &u.error {
        small(ui, e.clone(), p.danger);
    } else if let Some(t) = u.checked_at {
        small(
            ui,
            format!("Up to date · checked {}", format::unix_clock(t)),
            p.muted,
        );
    } else if app.prefs.update_last_check > 0 {
        small(
            ui,
            format!(
                "Up to date · last checked {}",
                format::unix_utc(app.prefs.update_last_check)
            ),
            p.muted,
        );
    } else {
        small(ui, "Not checked yet", p.muted);
    }
    let mut check = false;
    let mut download = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = S2;
        if let Some(r) = &u.found {
            if let Some(asset) = &r.asset
                && u.downloading.is_none()
            {
                let label = format!("Download ({:.1} MB)", asset.size as f64 / 1e6);
                let b = Button::new(RichText::new(label).color(p.accent_text))
                    .fill(p.accent)
                    .stroke(Stroke::NONE);
                download = ui
                    .add(b)
                    .on_hover_text(format!("Save {} to your Downloads folder", asset.name))
                    .clicked();
            }
            if ui.button("What's new").clicked() {
                let page = if r.page.is_empty() { REPO } else { &r.page };
                update::open_url(page);
            }
        }
        check = ui
            .add_enabled(u.pending.is_none(), Button::new("Check now"))
            .clicked();
    });
    if u.downloading.is_some() {
        spinner_line(ui, "Downloading the update…");
    } else if let Some(path) = &u.downloaded {
        ui.horizontal(|ui| {
            small(ui, format!("Saved to {}", path.display()), p.muted);
            if ui.small_button("Show").clicked() {
                update::reveal(path);
            }
        });
    }
    if let Some(e) = &u.download_error {
        small(ui, e.clone(), p.danger);
    }
    if download {
        app.start_update_download(ui.ctx());
    }
    if check {
        app.start_update_check(ui.ctx(), true);
    }
}

pub fn shortcuts(app: &mut App, ui: &mut egui::Ui) {
    let _ = app;
    shortcut_table(ui, "settings_keys");
}

pub fn about(app: &mut App, ui: &mut egui::Ui) {
    let _ = app;
    let p = palette(ui);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
        paint_logo(ui, r);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(format!("Version {}", update::CURRENT_VERSION)).color(p.text));
            ui.label(
                RichText::new("Licence: MIT OR Apache-2.0")
                    .small()
                    .color(p.muted),
            );
        });
    });
    muted_wrapped(
        ui,
        "Open-source asteroid search for students and teachers in IASC campaigns: blink \
         the images, find what moves, report it.",
    );
    if ui
        .link(RichText::new("github.com/alidevecioglu/OpenCeres").color(p.accent))
        .on_hover_text(REPO)
        .clicked()
    {
        update::open_url(REPO);
    }
    ui.add_space(S2);
    ui.label(RichText::new("Credits").strong().color(p.text));
    for line in [
        "SkyBoT — IMCCE, Observatoire de Paris",
        "MPCORB orbit catalogue — Minor Planet Center",
        "VSOP87 planetary theory — P. Bretagnon & G. Francou",
        "Planetary data — NASA JPL",
    ] {
        small(ui, format!("·  {line}"), p.muted);
    }
    ui.add_space(S2);
    small(
        ui,
        "OpenCeres is an independent project, not affiliated with Astrometrica or IASC.",
        p.muted,
    );
}
