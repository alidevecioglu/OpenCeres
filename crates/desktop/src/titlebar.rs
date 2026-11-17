//! macOS unified title bar: the window content runs under a transparent title
//! bar, the toolbar takes its place, and the close / minimise / zoom buttons
//! are moved down to the middle of the toolbar. Empty toolbar space moves the
//! window and a double click zooms it, as in a native title bar. On other
//! systems the native title bar stays and nothing here does anything.

use eframe::egui::{self, Id, Rect, Sense, Ui, ViewportBuilder};

/// Whether this build draws its own title bar (macOS only).
pub const UNIFIED: bool = cfg!(target_os = "macos");

/// Room on the left of the toolbar for the window buttons at their usual inset.
const WINDOW_BUTTONS_W: f32 = 76.0;
/// Left edge of the close button, points.
#[cfg(target_os = "macos")]
const WINDOW_BUTTONS_X: f64 = 13.0;

/// The builder with a transparent, title-less title bar over the content; the
/// window buttons stay. No change outside macOS.
pub fn unified(b: ViewportBuilder) -> ViewportBuilder {
    if UNIFIED {
        b.with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_titlebar_buttons_shown(true)
    } else {
        b
    }
}

/// Space to leave at the left of the toolbar for the window buttons (none in
/// full screen, where macOS hides them).
pub fn left_inset(ui: &Ui) -> f32 {
    if UNIFIED && !ui.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        WINDOW_BUTTONS_W
    } else {
        0.0
    }
}

/// Makes `rect` a window drag area. Call it before the widgets on top of it:
/// egui gives a click to the widget added last, so buttons keep theirs and
/// only empty space drags. A double click zooms, like the native title bar.
pub fn drag_area(ui: &Ui, rect: Rect, id: Id) {
    if !UNIFIED {
        return;
    }
    let response = ui.interact(rect, id, Sense::click_and_drag());
    if response.double_clicked() {
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if response.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

/// Centres the window buttons vertically on a bar `bar_h` points high. AppKit
/// lays the title bar out again on every resize, so this runs each frame; it
/// only writes frames that differ, so an idle window stays idle.
#[cfg(target_os = "macos")]
pub fn place_window_buttons(frame: &eframe::Frame, bar_h: f32, fullscreen: bool) {
    use objc2_app_kit::{NSView, NSWindowButton};
    use objc2_foundation::NSPoint;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    if fullscreen {
        return;
    }
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit hands out the window's content NSView, which lives as long
    // as the window; eframe calls the app on the main thread.
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
        return;
    };
    let buttons: Vec<_> = [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ]
    .into_iter()
    .filter_map(|b| window.standardWindowButton(b))
    .collect();
    let Some(close) = buttons.first() else {
        return;
    };
    // SAFETY: plain view-hierarchy getters on the main thread.
    let Some(container) = (unsafe { close.superview().and_then(|v| v.superview()) }) else {
        return;
    };
    // The buttons keep their place inside the title bar view, so size that
    // view to put their centre at bar_h / 2 from the top of the window.
    let button = close.frame();
    let height = bar_h as f64 / 2.0 + button.origin.y + button.size.height / 2.0;
    let mut bar = container.frame();
    let window_h = window.frame().size.height;
    if (bar.size.height - height).abs() > 0.5 || (bar.origin.y - (window_h - height)).abs() > 0.5 {
        bar.size.height = height;
        bar.origin.y = window_h - height;
        container.setFrame(bar);
    }
    let spacing = buttons
        .get(1)
        .map_or(20.0, |m| m.frame().origin.x - button.origin.x);
    for (i, b) in buttons.iter().enumerate() {
        let f = b.frame();
        let x = WINDOW_BUTTONS_X + i as f64 * spacing;
        if (f.origin.x - x).abs() > 0.5 {
            b.setFrameOrigin(NSPoint::new(x, f.origin.y));
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn place_window_buttons(_frame: &eframe::Frame, _bar_h: f32, _fullscreen: bool) {}
