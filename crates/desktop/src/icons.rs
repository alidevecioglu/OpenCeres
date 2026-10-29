//! Toolbar icons drawn as vector shapes, so they are crisp at any scale and
//! do not depend on which glyphs the built-in font has. Each icon is laid
//! out on a 16 × 16 point grid centred in its rect.

use std::f32::consts::{FRAC_PI_2, TAU};

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Response, Sense, Shape, Stroke, vec2};

use crate::theme::{self, palette};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Prev,
    Next,
    Play,
    Pause,
    Minus,
    Plus,
    Stars,
    Invert,
    Grid,
    Sidebar,
    Gear,
    Close,
    Check,
}

pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    // 16-point grid, scaled down when the rect is smaller.
    let s = (rect.width().min(rect.height()) / 16.0).min(1.0);
    let at = |x: f32, y: f32| c + vec2(x, y) * s;
    let stroke = Stroke::new(1.5, color);
    match icon {
        Icon::Prev | Icon::Next => {
            let d = if icon == Icon::Prev { 1.0 } else { -1.0 };
            painter.line(
                vec![at(2.5 * d, -5.0), at(-2.5 * d, 0.0), at(2.5 * d, 5.0)],
                Stroke::new(1.75, color),
            );
        }
        Icon::Play => {
            painter.add(Shape::convex_polygon(
                vec![at(-3.5, -5.5), at(5.5, 0.0), at(-3.5, 5.5)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            for dx in [-2.75, 2.75] {
                let bar = Rect::from_center_size(at(dx, 0.0), vec2(3.0, 11.0) * s);
                painter.rect_filled(bar, 1.0, color);
            }
        }
        Icon::Minus | Icon::Plus => {
            painter.hline((c.x - 4.5 * s)..=(c.x + 4.5 * s), c.y, stroke);
            if icon == Icon::Plus {
                painter.vline(c.x, (c.y - 4.5 * s)..=(c.y + 4.5 * s), stroke);
            }
        }
        Icon::Stars => {
            // A detection ring around a star, with reticle ticks.
            painter.circle_stroke(c, 4.5 * s, stroke);
            painter.circle_filled(c, 1.5 * s, color);
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                painter.line_segment([at(dx * 6.0, dy * 6.0), at(dx * 7.5, dy * 7.5)], stroke);
            }
        }
        Icon::Invert => {
            let r = 6.0 * s;
            let half: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = FRAC_PI_2 + i as f32 / 24.0 * std::f32::consts::PI;
                    c + vec2(a.cos(), -a.sin()) * r
                })
                .collect();
            painter.add(Shape::convex_polygon(half, color, Stroke::NONE));
            painter.circle_stroke(c, r, stroke);
        }
        Icon::Grid => {
            let r = Rect::from_center_size(c, vec2(12.0, 12.0) * s);
            painter.rect_stroke(r, 1.5, stroke, egui::StrokeKind::Middle);
            for t in [1.0 / 3.0, 2.0 / 3.0] {
                painter.vline(
                    r.min.x + r.width() * t,
                    r.y_range(),
                    Stroke::new(1.0, color),
                );
                painter.hline(
                    r.x_range(),
                    r.min.y + r.height() * t,
                    Stroke::new(1.0, color),
                );
            }
        }
        Icon::Sidebar => {
            let r = Rect::from_center_size(c, vec2(14.0, 12.0) * s);
            let side = Rect::from_min_max(egui::pos2(r.max.x - 5.0 * s, r.min.y), r.max);
            painter.rect_filled(side, 1.5, color.gamma_multiply(0.45));
            painter.rect_stroke(r, 2.0, stroke, egui::StrokeKind::Middle);
            painter.vline(side.min.x, r.y_range(), stroke);
        }
        Icon::Gear => {
            let teeth = 8;
            for i in 0..teeth {
                let a = i as f32 / teeth as f32 * TAU;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment(
                    [c + d * 4.5 * s, c + d * 7.0 * s],
                    Stroke::new(2.25 * s.max(0.8), color),
                );
            }
            painter.circle_stroke(c, 4.75 * s, Stroke::new(1.75, color));
            painter.circle_stroke(c, 1.75 * s, Stroke::new(1.5, color));
        }
        Icon::Close => {
            let k = 4.0;
            painter.line_segment([at(-k, -k), at(k, k)], stroke);
            painter.line_segment([at(-k, k), at(k, -k)], stroke);
        }
        Icon::Check => {
            painter.line(
                vec![at(-4.5, 0.5), at(-1.25, 3.75), at(4.75, -3.75)],
                Stroke::new(1.75, color),
            );
        }
    }
}

/// A square, frameless toolbar button. `selected` gives it the accent
/// background, for toggles that are on.
pub fn button(ui: &mut egui::Ui, icon: Icon, selected: bool, enabled: bool) -> Response {
    let p = palette(ui);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(theme::TOOL, theme::TOOL), sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = f32::from(theme::RADIUS);
        let color = if !enabled {
            p.muted.gamma_multiply(0.5)
        } else if selected {
            p.accent
        } else {
            p.text
        };
        if selected {
            painter.rect_filled(rect, radius, p.accent_weak);
        } else if enabled && (response.hovered() || response.has_focus()) {
            painter.rect_filled(rect, radius, p.hover);
        }
        if enabled && response.is_pointer_button_down_on() {
            painter.rect_filled(rect, radius, p.hover.gamma_multiply(0.6));
        }
        paint(painter, rect, icon, color);
    }
    if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    }
}

/// A smaller frameless icon button (dismiss buttons in banners and windows).
pub fn small_button(ui: &mut egui::Ui, icon: Icon, color: Color32) -> Response {
    let p = palette(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, p.hover.gamma_multiply(0.8));
    }
    paint(ui.painter(), rect.shrink(3.0), icon, color);
    response.on_hover_cursor(CursorIcon::PointingHand)
}
