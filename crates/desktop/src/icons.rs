//! Toolbar icons drawn as vector shapes, so they are crisp at any scale and
//! do not depend on which glyphs the built-in font has. Each icon is laid
//! out on a 16 × 16 point grid centred in its rect, with everything (strokes
//! included) inside the central 14 points, in 1.5-point strokes and 1.25 for
//! interior detail.

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
    Align,
    Grid,
    Known,
    Sidebar,
    Gear,
    History,
    Team,
    Close,
    Check,
    // Settings categories and search.
    Theme,
    Eye,
    Update,
    Keyboard,
    Info,
    Search,
}

pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    // 16-point grid, scaled down when the rect is smaller. Strokes scale with
    // the geometry so that an icon keeps its weight at every size.
    let s = (rect.width().min(rect.height()) / 16.0).min(1.0);
    let at = |x: f32, y: f32| c + vec2(x, y) * s;
    let stroke = Stroke::new(1.5 * s, color);
    let detail = Stroke::new(1.25 * s, color);
    match icon {
        Icon::Prev | Icon::Next => {
            let d = if icon == Icon::Prev { 1.0 } else { -1.0 };
            painter.line(
                vec![at(2.5 * d, -5.0), at(-2.5 * d, 0.0), at(2.5 * d, 5.0)],
                stroke,
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
                painter.rect_filled(bar, 1.0 * s, color);
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
            painter.circle_stroke(c, 4.0 * s, stroke);
            painter.circle_filled(c, 1.5 * s, color);
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                painter.line_segment([at(dx * 5.5, dy * 5.5), at(dx * 7.0, dy * 7.0)], detail);
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
        Icon::Align => {
            // A faint ring sliding onto a solid one: a star brought into line.
            let off = at(-2.5, -2.5);
            painter.circle_stroke(
                off,
                3.5 * s,
                Stroke::new(detail.width, color.gamma_multiply(0.45)),
            );
            let c = at(2.25, 2.25);
            painter.circle_stroke(c, 3.5 * s, stroke);
            painter.circle_filled(c, 1.25 * s, color);
        }
        Icon::Grid => {
            let r = Rect::from_center_size(c, vec2(12.0, 12.0) * s);
            painter.rect_stroke(r, 1.5 * s, stroke, egui::StrokeKind::Middle);
            for t in [1.0 / 3.0, 2.0 / 3.0] {
                painter.vline(r.min.x + r.width() * t, r.y_range(), detail);
                painter.hline(r.x_range(), r.min.y + r.height() * t, detail);
            }
        }
        Icon::Known => {
            // An open tilted orbit with one asteroid in the gap: a closed
            // ellipse with a sun inside it reads as a blob at toolbar size.
            let (cos, sin) = ((-0.45f32).cos(), (-0.45f32).sin());
            let orbit = |t: f32| {
                let (x, y) = (6.25 * t.cos(), 3.75 * t.sin());
                at(x * cos - y * sin, x * sin + y * cos)
            };
            let (from, sweep) = (0.55, TAU - 1.7);
            let arc: Vec<Pos2> = (0..=40)
                .map(|i| orbit(from + i as f32 / 40.0 * sweep))
                .collect();
            painter.add(Shape::line(arc, stroke));
            painter.circle_filled(orbit(from + sweep + 0.85), 2.0 * s, color);
        }
        Icon::Sidebar => {
            let r = Rect::from_center_size(c, vec2(12.5, 11.0) * s);
            let side = Rect::from_min_max(egui::pos2(r.max.x - 4.5 * s, r.min.y), r.max);
            painter.rect_filled(side, 1.5 * s, color.gamma_multiply(0.45));
            painter.rect_stroke(r, 2.0 * s, stroke, egui::StrokeKind::Middle);
            painter.vline(side.min.x, r.y_range(), detail);
        }
        Icon::Gear => {
            // Six teeth on one ring: the lightest form that still reads as a
            // gear, so it no longer outweighs the rest of the toolbar.
            let r = 4.25 * s;
            painter.circle_stroke(c, r, stroke);
            for i in 0..6 {
                let a = (i as f32 + 0.5) / 6.0 * TAU;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * r, c + d * 6.75 * s], stroke);
            }
        }
        Icon::Close => {
            let k = 4.0;
            painter.line_segment([at(-k, -k), at(k, k)], stroke);
            painter.line_segment([at(-k, k), at(k, -k)], stroke);
        }
        Icon::Theme => {
            // A sun: disc and rays.
            painter.circle_stroke(c, 3.25 * s, stroke);
            for i in 0..8 {
                let a = i as f32 / 8.0 * TAU;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * 5.25 * s, c + d * 7.0 * s], detail);
            }
        }
        Icon::Eye => {
            let lid = |sign: f32| -> Vec<Pos2> {
                (0..=16)
                    .map(|i| {
                        let t = i as f32 / 16.0 * 2.0 - 1.0;
                        at(t * 6.5, sign * 4.5 * (1.0 - t * t))
                    })
                    .collect()
            };
            painter.add(Shape::line(lid(1.0), stroke));
            painter.add(Shape::line(lid(-1.0), stroke));
            painter.circle_filled(c, 2.0 * s, color);
        }
        Icon::Update => {
            // A circular arrow.
            let r = 5.5 * s;
            let arc: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = -0.6 + i as f32 / 20.0 * (TAU - 1.6);
                    c + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            let end = arc[arc.len() - 1];
            painter.add(Shape::line(arc, stroke));
            let a = -0.6 + TAU - 1.6;
            let tangent = vec2(-a.sin(), a.cos());
            let normal = vec2(a.cos(), a.sin());
            painter.add(Shape::convex_polygon(
                vec![
                    end + tangent * 3.0 * s,
                    end - normal * 2.75 * s,
                    end + normal * 2.75 * s,
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Keyboard => {
            let r = Rect::from_center_size(c, vec2(12.5, 10.0) * s);
            painter.rect_stroke(r, 2.0 * s, stroke, egui::StrokeKind::Middle);
            for row in [-2.0, 0.5] {
                for col in [-3.75, -1.25, 1.25, 3.75] {
                    painter.circle_filled(at(col, row), 0.8 * s, color);
                }
            }
            painter.hline((c.x - 3.0 * s)..=(c.x + 3.0 * s), c.y + 3.0 * s, detail);
        }
        Icon::Info => {
            painter.circle_stroke(c, 6.25 * s, stroke);
            painter.circle_filled(at(0.0, -3.0), 1.0 * s, color);
            painter.vline(c.x, (c.y - 0.75 * s)..=(c.y + 3.75 * s), stroke);
        }
        Icon::Search => {
            painter.circle_stroke(at(-1.25, -1.25), 4.25 * s, stroke);
            painter.line_segment([at(1.9, 1.9), at(5.5, 5.5)], stroke);
        }
        Icon::History => {
            // A clock face.
            painter.circle_stroke(c, 6.25 * s, stroke);
            painter.line(vec![at(0.0, -3.75), at(0.0, 0.0), at(2.75, 1.75)], detail);
        }
        Icon::Team => {
            // Two people, the one behind slightly smaller.
            painter.circle_stroke(at(4.25, -3.5), 1.75 * s, detail);
            painter.add(Shape::line(
                vec![at(4.5, 1.0), at(5.75, 1.75), at(6.25, 4.0)],
                detail,
            ));
            painter.circle_stroke(at(-1.75, -2.75), 2.5 * s, stroke);
            painter.add(Shape::line(
                vec![
                    at(-6.25, 6.0),
                    at(-5.5, 3.0),
                    at(-3.75, 1.25),
                    at(0.25, 1.25),
                    at(2.0, 3.0),
                    at(2.75, 6.0),
                ],
                stroke,
            ));
        }
        Icon::Check => {
            painter.line(
                vec![at(-4.5, 0.5), at(-1.25, 3.75), at(4.75, -3.75)],
                stroke,
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
            p.icon
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
