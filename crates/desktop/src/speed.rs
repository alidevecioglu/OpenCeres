//! Blink speed: seconds per frame, any value from [`MIN`] to [`MAX`].
//!
//! Fine steps grow with the value (0.05 s below 1 s, 0.1 s up to 2 s, 0.25 s
//! above), so each press changes the blink by a similar fraction; Shift
//! jumps between round [`COARSE`] values. Everywhere on screen slower is on
//! the left and faster on the right, marked by a tortoise and a hare.

use eframe::egui::{
    self, Button, CursorIcon, DragValue, Id, Popup, PopupCloseBehavior, RectAlign, Response,
    RichText, Sense, Slider, Ui, vec2,
};

use crate::icons::{self, Icon};
use crate::theme::{self, S1, S2, palette};

pub const MIN: f32 = 0.1;
pub const MAX: f32 = 5.0;
/// Shift + key (or Shift + click on the toolbar arrows) moves between these.
pub const COARSE: [f32; 11] = [0.1, 0.2, 0.3, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0];
/// One-click choices in the speed popover, slow to fast.
const PRESETS: [f32; 4] = [2.0, 1.0, 0.5, 0.2];
/// Dragging this far across the value halves (or doubles) the time per frame,
/// so the whole range is about 340 points of drag.
const DRAG_DOUBLING: f32 = 60.0;
/// Scrolling this far does the same.
const SCROLL_DOUBLING: f32 = 120.0;

/// The fine steps, in seconds, ascending.
fn fine() -> impl DoubleEndedIterator<Item = f32> {
    (10u16..=100)
        .step_by(5)
        .chain((110u16..=200).step_by(10))
        .chain((225u16..=500).step_by(25))
        .map(|hundredths| f32::from(hundredths) / 100.0)
}

/// The nearest stop beyond `secs` in the given direction (a value between
/// stops goes to its neighbour), clamped to the range.
fn next(mut stops: impl DoubleEndedIterator<Item = f32>, secs: f32, slower: bool) -> f32 {
    const EPS: f32 = 1e-3;
    if slower {
        stops.find(|&s| s > secs + EPS).unwrap_or(MAX)
    } else {
        stops.rev().find(|&s| s < secs - EPS).unwrap_or(MIN)
    }
}

/// One fine step slower (longer per frame) or faster.
pub fn step(secs: f32, slower: bool) -> f32 {
    next(fine(), secs, slower)
}

/// To the next [`COARSE`] value slower or faster.
pub fn jump(secs: f32, slower: bool) -> f32 {
    next(COARSE.into_iter(), secs, slower)
}

/// In range and rounded to what the display shows (0.01 s).
pub fn tidy(secs: f32) -> f32 {
    (secs.clamp(MIN, MAX) * 100.0).round() / 100.0
}

/// `secs` scaled by 2^(-amount): positive amounts make the blink faster.
fn scaled(secs: f32, amount: f32) -> f32 {
    (secs * (-amount).exp2()).clamp(MIN, MAX)
}

/// "0.50 s".
pub fn label(secs: f32) -> String {
    format!("{secs:.2} s")
}

/// The toolbar control, added right to left (it sits in the toolbar's
/// right-to-left run): hare (faster), the value, tortoise (slower). The
/// value can be dragged sideways or scrolled, and a click opens a popover
/// with a slider, presets and an exact field. Returns true if `secs` changed.
pub fn toolbar(ui: &mut Ui, secs: &mut f32, popover: &mut bool) -> bool {
    let before = *secs;
    let shift = ui.input(|i| i.modifiers.shift);
    let arrow = |ui: &mut Ui, icon: Icon, enabled: bool, tip: &str| {
        icons::button(ui, icon, false, enabled)
            .on_hover_text(tip)
            .clicked()
    };
    if arrow(
        ui,
        Icon::Faster,
        *secs > MIN,
        "Faster: less time on each frame (.)\nShift: bigger step",
    ) {
        *secs = if shift {
            jump(*secs, false)
        } else {
            step(*secs, false)
        };
    }
    let value = value_button(ui, secs, *popover);
    if arrow(
        ui,
        Icon::Slower,
        *secs < MAX,
        "Slower: more time on each frame (,)\nShift: bigger step",
    ) {
        *secs = if shift {
            jump(*secs, true)
        } else {
            step(*secs, true)
        };
    }
    if value.clicked() {
        *popover = !*popover;
    }
    Popup::from_response(&value)
        .id(Id::new("blink_speed_popover"))
        .open_bool(popover)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .align(RectAlign::BOTTOM)
        .gap(S1)
        .show(|ui| {
            ui.set_width(236.0);
            editor(ui, secs);
        });
    let changed = *secs != before;
    if changed {
        // The blink timer was scheduled before the toolbar ran.
        ui.ctx().request_repaint();
    }
    changed
}

/// The value, as a flat button that also drags and scrolls.
fn value_button(ui: &mut Ui, secs: &mut f32, open: bool) -> Response {
    let p = palette(ui);
    // Wide enough for "5.00 s", and fixed so the toolbar never shifts.
    let (rect, response) = ui.allocate_exact_size(vec2(48.0, theme::TOOL), Sense::click_and_drag());
    if response.drag_started() {
        ui.data_mut(|d| d.insert_temp(response.id, *secs));
    }
    if response.dragged() {
        let start = ui.data(|d| d.get_temp(response.id)).unwrap_or(*secs);
        let dx = response.total_drag_delta().map_or(0.0, |d| d.x);
        *secs = scaled(start, dx / DRAG_DOUBLING);
    }
    if response.drag_stopped() {
        *secs = tidy(*secs);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        let amount = scroll.x + scroll.y;
        if amount != 0.0 {
            *secs = scaled(*secs, amount / SCROLL_DOUBLING);
        }
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = f32::from(theme::RADIUS);
        if open || response.dragged() {
            painter.rect_filled(rect, radius, p.accent_weak);
        } else if response.hovered() || response.has_focus() {
            painter.rect_filled(rect, radius, p.hover);
        }
        let color = if open { p.accent } else { p.text };
        let galley =
            ui.painter()
                .layout_no_wrap(label(*secs), egui::FontId::proportional(13.0), color);
        let pos = rect.center() - galley.size() / 2.0;
        painter.galley(pos, galley, color);
    }
    response
        .on_hover_cursor(CursorIcon::ResizeHorizontal)
        .on_hover_text(
            "Seconds per frame\nDrag sideways or scroll to change it, click for more\n\
             ( , slower · . faster · Shift for bigger steps )",
        )
}

/// Tortoise, slider, hare: slow on the left, fast on the right like the
/// toolbar, on a logarithmic scale so short times get as much room as long
/// ones. Returns true if `secs` changed.
pub fn slider(ui: &mut Ui, secs: &mut f32, width: f32) -> bool {
    let p = palette(ui);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = S1;
        let icon = |ui: &mut Ui, icon: Icon| {
            let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), rect, icon, p.muted);
        };
        icon(ui, Icon::Slower);
        ui.spacing_mut().slider_width = (width - 2.0 * (16.0 + S1)).max(40.0);
        changed = ui
            .add(
                Slider::new(secs, MAX..=MIN)
                    .logarithmic(true)
                    .show_value(false),
            )
            .on_hover_text("Seconds per frame: slower to the left, faster to the right")
            .changed();
        icon(ui, Icon::Faster);
    });
    if changed {
        *secs = tidy(*secs);
    }
    changed
}

/// An exact field, 0.10–5.00 s.
pub fn field(ui: &mut Ui, secs: &mut f32) -> bool {
    let response = ui.add(
        DragValue::new(secs)
            .range(MIN..=MAX)
            .speed(0.01)
            .fixed_decimals(2)
            .suffix(" s"),
    );
    if response.changed() {
        *secs = tidy(*secs);
    }
    response.changed()
}

/// The popover: slider, presets and the exact value.
fn editor(ui: &mut Ui, secs: &mut f32) {
    let p = palette(ui);
    ui.spacing_mut().item_spacing.y = S2;
    ui.label(RichText::new("Blink speed").color(p.text).strong());
    slider(ui, secs, ui.available_width());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = S1;
        let w = (ui.available_width() - 3.0 * S1) / PRESETS.len() as f32;
        for preset in PRESETS {
            let on = (*secs - preset).abs() < 0.005;
            let text = format!("{preset} s");
            if ui
                .add_sized(vec2(w, 22.0), Button::new(text).selected(on))
                .clicked()
            {
                *secs = preset;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Seconds per frame").color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            field(ui, secs);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn fine_steps_change_size_at_one_and_two_seconds() {
        let cases = [
            // (from, slower, to)
            (0.5, true, 0.55),
            (0.5, false, 0.45),
            (0.95, true, 1.0),
            (1.0, true, 1.1),
            (1.0, false, 0.95),
            (1.1, false, 1.0),
            (1.9, true, 2.0),
            (2.0, true, 2.25),
            (2.0, false, 1.9),
            (2.25, false, 2.0),
            (4.75, true, 5.0),
        ];
        for (from, slower, to) in cases {
            let got = step(from, slower);
            assert!(close(got, to), "{from} slower={slower}: {got}, want {to}");
        }
    }

    #[test]
    fn steps_clamp_at_the_ends() {
        assert!(close(step(MIN, false), MIN));
        assert!(close(step(MAX, true), MAX));
        assert!(close(step(0.12, false), MIN));
        assert!(close(step(4.9, true), MAX));
        assert!(close(jump(MIN, false), MIN));
        assert!(close(jump(MAX, true), MAX));
    }

    #[test]
    fn off_grid_values_step_to_their_neighbours() {
        // A dragged value such as 0.53 goes to 0.55 or 0.50, not 0.58.
        assert!(close(step(0.53, true), 0.55));
        assert!(close(step(0.53, false), 0.5));
        assert!(close(step(1.04, true), 1.1));
        assert!(close(step(2.1, true), 2.25));
        assert!(close(step(2.1, false), 2.0));
    }

    #[test]
    fn every_step_round_trips() {
        // Up then down returns to the same stop everywhere in the range.
        let mut s = MIN;
        let mut count = 0;
        while s < MAX {
            let up = step(s, true);
            assert!(up > s);
            assert!(close(step(up, false), s), "{s} -> {up}");
            s = up;
            count += 1;
        }
        // 18 steps of 0.05 to 1 s, 10 of 0.1 to 2 s, 12 of 0.25 to 5 s.
        assert_eq!(count, 18 + 10 + 12);
    }

    #[test]
    fn coarse_jumps_go_to_the_next_preset() {
        assert!(close(jump(0.5, true), 0.75));
        assert!(close(jump(0.5, false), 0.3));
        assert!(close(jump(0.55, false), 0.5));
        assert!(close(jump(0.55, true), 0.75));
        assert!(close(jump(3.0, true), 4.0));
    }

    #[test]
    fn drag_and_tidy_stay_in_range() {
        assert!(close(scaled(1.0, 1.0), 0.5));
        assert!(close(scaled(1.0, -1.0), 2.0));
        assert!(close(scaled(0.2, 10.0), MIN));
        assert!(close(scaled(3.0, -10.0), MAX));
        assert!(close(tidy(0.504), 0.5));
        assert!(close(tidy(9.0), MAX));
        assert!(close(tidy(0.0), MIN));
        assert_eq!(label(0.5), "0.50 s");
    }
}
