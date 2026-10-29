//! OpenCeres desktop app: a native (egui + OpenGL) blink viewer for
//! asteroid searching. Paths given on the command line are opened at start.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod capture;
mod format;
mod gpu;
mod gz;
mod icons;
mod known;
mod load;
mod prefs;
mod settings;
mod theme;
mod update;
mod zip;

use eframe::egui;

fn main() -> eframe::Result {
    // macOS passes `-psn_…` when launched from Finder on old systems; skip flags.
    let paths: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    // Developer screenshots of the app's own window; see `capture.rs`.
    let capture = capture::Capture::from_env();
    let size = capture
        .as_ref()
        .and_then(|c| c.state.size)
        .unwrap_or([1280.0, 820.0]);
    let viewport = egui::ViewportBuilder::default();
    // macOS takes the icon from the app bundle.
    #[cfg(not(target_os = "macos"))]
    let viewport = viewport.with_icon(window_icon());
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_title("OpenCeres")
            .with_app_id("org.openceres.desktop")
            .with_inner_size(size)
            .with_min_inner_size([760.0, 480.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Glow,
        // No multisampling, depth or stencil buffers: egui antialiases its
        // own shapes, and they would only cost video memory.
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };
    eframe::run_native(
        "OpenCeres",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, paths, capture)))),
    )
}

/// The window icon (Windows title bar and taskbar, Linux window list):
/// 256 × 256 RGBA, zlib-compressed by `assets/make_icons.py`.
#[cfg(not(target_os = "macos"))]
fn window_icon() -> egui::IconData {
    let packed = include_bytes!("../assets/icon-256.rgba.z");
    let rgba = miniz_oxide::inflate::decompress_to_vec_zlib(packed).unwrap_or_default();
    if rgba.len() != 256 * 256 * 4 {
        return egui::IconData::default();
    }
    egui::IconData {
        rgba,
        width: 256,
        height: 256,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_icon_inflates_to_256_square_rgba() {
        let packed = include_bytes!("../assets/icon-256.rgba.z");
        let rgba = miniz_oxide::inflate::decompress_to_vec_zlib(packed).unwrap();
        assert_eq!(rgba.len(), 256 * 256 * 4);
    }
}
