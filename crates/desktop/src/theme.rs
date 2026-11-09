//! Colours, spacing and widget styling for the light and dark themes.
//!
//! One spacing scale (4/8/12/16/24), one corner radius, calm neutral
//! surfaces, a single blue accent for primary and selected state, amber
//! only for the report grid and teal-green only for known objects. The image
//! stage is dark in both themes; what is drawn on it follows the image's
//! invert state, not the window theme.
//!
//! Text is Inter at a 12/13/16 pt scale, with Hack for coordinates and times
//! so their digits do not jitter. The fonts in `assets/fonts` are cut down to
//! Latin (with Extended, so Turkish), Greek, Cyrillic and the symbols the UI
//! uses, keeping the `kern`, `mark`, `mkmk` and `ccmp` layout features that
//! egui's shaper applies. U+25FB is left out on purpose: it is egui's
//! "missing glyph" box, and without it a name in another script shows `?`
//! instead of tofu.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Theme, Visuals,
};

pub const S1: f32 = 4.0;
pub const S2: f32 = 8.0;
pub const S3: f32 = 12.0;
pub const S4: f32 = 16.0;
pub const S5: f32 = 24.0;
pub const RADIUS: u8 = 6;

pub const TOOLBAR_HEIGHT: f32 = 38.0;
pub const STATUS_HEIGHT: f32 = 22.0;
pub const PANEL_WIDTH: f32 = 248.0;
/// Toolbar buttons and toggles are square.
pub const TOOL: f32 = 26.0;

pub struct Palette {
    /// Window background behind cards (empty state).
    pub bg: Color32,
    /// Toolbar, side panel, status bar, windows.
    pub panel: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// Resting toolbar icons, a step down from `text` so the image leads.
    pub icon: Color32,
    pub border: Color32,
    /// Edges that must be seen (inputs, selected segments, off switches):
    /// at least 3:1 on the panel.
    pub border_strong: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    /// Background of selected toggles.
    pub accent_weak: Color32,
    pub hover: Color32,
    /// Background of inset boxes (report text, key caps).
    pub inset: Color32,
    pub danger_bg: Color32,
    pub danger: Color32,
    #[allow(dead_code)] // part of the palette; no status text uses it yet
    pub success: Color32,
    #[allow(dead_code)]
    pub warning: Color32,
    /// The student's own objects and known objects, as text in the side
    /// panel (the overlay colours are chosen for the image, not the panel).
    pub tag: Color32,
    pub known: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const LIGHT: Palette = Palette {
    bg: rgb(0xeef0f3),
    panel: rgb(0xebedf1),
    text: rgb(0x16191f),  // 15.0:1 on panel
    muted: rgb(0x596272), // 5.3:1 on panel
    icon: rgb(0x16191f),
    border: rgb(0xd3d7df),
    border_strong: rgb(0x7f8796), // 3.1:1 on panel
    accent: rgb(0x1f5fd1),        // 5.0:1 on panel, 4.8:1 on accent_weak
    accent_text: rgb(0xffffff),
    accent_weak: rgb(0xe1eafb),
    hover: rgb(0xe0e3e9),
    inset: rgb(0xf5f6f8),
    danger_bg: rgb(0xfcebea),
    danger: rgb(0xa3221c),
    success: rgb(0x14733f), // 5.0:1 on panel
    warning: rgb(0x8a5a00), // 5.1:1 on panel
    tag: rgb(0xa8125f),
    known: rgb(0x00664c),
};

pub const DARK: Palette = Palette {
    bg: rgb(0x16191f),
    panel: rgb(0x1b1d21),
    text: rgb(0xe6e9ee),  // 13.9:1 on panel
    muted: rgb(0x9aa3b1), // 6.6:1 on panel
    icon: rgb(0xc3c9d3),
    border: rgb(0x2e3238),
    border_strong: rgb(0x6a7380), // 3.5:1 on panel
    accent: rgb(0x5b8ff0),        // 5.3:1 on panel, 4.6:1 on accent_weak
    accent_text: rgb(0x0d1117),
    accent_weak: rgb(0x1f2a3d),
    hover: rgb(0x2a2f38),
    inset: rgb(0x262a32),
    danger_bg: rgb(0x3a1f1f),
    danger: rgb(0xffb1aa),
    success: rgb(0x5fd39a), // 9.1:1 on panel
    warning: rgb(0xf2b33d), // 9.1:1 on panel
    tag: rgb(0xff6fb5),
    known: rgb(0x7af0bd),
};

// The image stage and its corner badge look the same in both themes.
pub const STAGE: Color32 = rgb(0x111318);
pub const BADGE: Color32 = Color32::from_rgba_premultiplied(4, 5, 6, 204); // black at 80 %
pub const BADGE_TEXT: Color32 = rgb(0xb4bbc6);
pub const STREAK: Color32 = rgb(0xf2b33d);
pub const OVERLAY_TEXT: Color32 = rgb(0xf1f3f6);

/// What is drawn over the image. The set follows the image's invert state
/// (independent of the window theme): light colours on the black sky, dark
/// ones on an inverted white sky. Rings get a halo of the opposite lightness,
/// which also keeps them readable across a star of their own lightness; a
/// dark halo on white only greys the ring it should set off.
pub struct Overlay {
    pub star: Color32,
    pub grid_line: Color32,
    pub grid_fill: Color32,
    pub grid_text: Color32,
    /// Behind labels on the image: as opaque as needed for text at 4.5:1
    /// even when a star of the wrong lightness is underneath.
    pub chip: Color32,
    pub known: Color32,
    pub known_text: Color32,
    /// Objects the student marked (detailed report).
    pub tag: Color32,
    pub tag_text: Color32,
    pub halo: Color32,
    /// Other students' reports, assigned in turn. Never the only cue: each
    /// also has a ring pattern and the student's initial on its chip.
    pub review: [Color32; 5],
    /// Rings of the other frames, and the track between marks, faded back
    /// where the fade keeps at least 3:1 on the sky.
    pub past: f32,
    pub track: f32,
}

/// On the normal, dark sky. Text on `chip` is at least 4.5:1 even over a
/// white star, rings at least 8.1:1 on black, faded rings 3.3:1 or better.
pub const OVERLAY: Overlay = Overlay {
    star: rgb(0x4cc2ff),
    grid_line: Color32::from_rgba_premultiplied(204, 141, 26, 204), // rgba(255,176,32,.8)
    grid_fill: Color32::from_rgba_premultiplied(71, 49, 9, 71),     // rgba(255,176,32,.28)
    grid_text: rgb(0xffc04d),
    chip: Color32::from_rgba_premultiplied(15, 18, 23, 217), // rgba(18,21,27,.85)
    known: rgb(0x3ddc97),
    known_text: rgb(0x7af0bd),
    tag: rgb(0xff6fb5),
    tag_text: rgb(0xff6fb5),
    halo: Color32::from_rgba_premultiplied(0, 0, 0, 150),
    review: [
        rgb(0xb48cff),
        rgb(0xff9b5e),
        rgb(0xc6f25a),
        rgb(0x5ee0d0),
        rgb(0xff7a8a),
    ],
    past: 0.8,
    track: 0.6,
};

/// On an inverted, white sky: rings 5.2:1 or better on white, text on
/// `chip` at least 4.9:1 even over a black star. Strokes stay nearly opaque
/// because any fade on white loses contrast fast.
pub const OVERLAY_INVERT: Overlay = Overlay {
    star: rgb(0x0072b2),
    grid_line: rgb(0x9a5b00),
    grid_fill: Color32::from_rgba_premultiplied(31, 18, 0, 51), // rgba(154,91,0,.2)
    grid_text: rgb(0x7d4a00),
    chip: Color32::from_rgba_premultiplied(240, 240, 240, 240), // white at 94 %
    known: rgb(0x00795a),
    known_text: rgb(0x00664c),
    tag: rgb(0xc2187a),
    tag_text: rgb(0xa8125f),
    halo: Color32::from_rgba_premultiplied(170, 170, 170, 170), // white at 67 %
    review: [
        rgb(0x6a3fb5),
        rgb(0xa84a12),
        rgb(0x4d6b00),
        rgb(0x007378),
        rgb(0xb02a45),
    ],
    past: 1.0,
    track: 0.85,
};

pub fn overlay(invert: bool) -> &'static Overlay {
    if invert { &OVERLAY_INVERT } else { &OVERLAY }
}

/// Whether the embedded fonts can draw `c` (the letters of a name, say).
/// `Fonts::has_glyph` cannot tell: it compares faces, and egui's replacement
/// `?` lives in the same Inter face as the letters. Keep in step with the
/// subset described at the top of this file.
pub fn can_draw(c: char) -> bool {
    matches!(c, '\u{21}'..='\u{24f}' | '\u{384}'..='\u{3ce}' | '\u{400}'..='\u{45f}')
}

/// Family for headings and emphasis; egui's `.strong()` only changes colour.
pub fn semibold() -> FontFamily {
    FontFamily::Name(SEMIBOLD.into())
}

const SEMIBOLD: &str = "Inter-SemiBold";

pub fn palette(ui: &egui::Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.inset;
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.inset;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent_weak;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(RADIUS + 2);
    v.menu_corner_radius = CornerRadius::same(RADIUS);
    v.window_shadow.blur = 24;
    v.window_shadow.offset = [0, 8];
    v.popup_shadow.blur = 12;
    v.popup_shadow.offset = [0, 4];
    v.window_shadow.color = Color32::from_black_alpha(if dark { 110 } else { 40 });
    v.popup_shadow.color = Color32::from_black_alpha(if dark { 90 } else { 30 });
    let radius = CornerRadius::same(RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.fg_stroke.color = p.text;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.corner_radius = radius;
    for (state, fill) in [
        (&mut w.inactive, p.panel),
        (&mut w.hovered, p.hover),
        (&mut w.active, p.hover),
        (&mut w.open, p.hover),
    ] {
        state.weak_bg_fill = fill;
        state.bg_fill = p.hover;
        state.bg_stroke = Stroke::new(1.0, p.border_strong);
        state.fg_stroke.color = p.text;
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    w.inactive.bg_fill = p.inset;
    w.hovered.bg_stroke = Stroke::new(1.0, p.muted);
    v
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::empty();
    for (name, bytes) in [
        (
            "Inter",
            &include_bytes!("../assets/fonts/Inter-Regular.ttf")[..],
        ),
        (
            SEMIBOLD,
            &include_bytes!("../assets/fonts/Inter-SemiBold.ttf")[..],
        ),
        (
            "Hack",
            &include_bytes!("../assets/fonts/Hack-Regular.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let own = |names: &[&str]| names.iter().map(|&n| n.to_owned()).collect::<Vec<_>>();
    fonts
        .families
        .insert(FontFamily::Proportional, own(&["Inter"]));
    fonts.families.insert(semibold(), own(&[SEMIBOLD, "Inter"]));
    // Hack has no ⌘ or names in other scripts; Inter fills in for it.
    fonts
        .families
        .insert(FontFamily::Monospace, own(&["Hack", "Inter"]));
    ctx.set_fonts(fonts);

    for (theme, p) in [(Theme::Light, &LIGHT), (Theme::Dark, &DARK)] {
        ctx.set_visuals_of(theme, visuals(p, theme == Theme::Dark));
    }
    ctx.all_styles_mut(|s| {
        use FontFamily::{Monospace, Proportional};
        s.text_styles = [
            (TextStyle::Small, FontId::new(12.0, Proportional)),
            (TextStyle::Body, FontId::new(13.0, Proportional)),
            (TextStyle::Button, FontId::new(13.0, Proportional)),
            (TextStyle::Heading, FontId::new(16.0, semibold())),
            (TextStyle::Monospace, FontId::new(12.0, Monospace)),
        ]
        .into();
        s.spacing.button_padding = egui::vec2(S2, S1);
        s.spacing.item_spacing = egui::vec2(S2, S1);
        s.spacing.interact_size.y = TOOL;
        s.spacing.window_margin = egui::Margin::same(S4 as i8);
        s.spacing.menu_margin = egui::Margin::same(S2 as i8);
        s.interaction.tooltip_delay = 0.4;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` during a frame, when the fonts exist. The texture deltas of
    /// a frame that is never drawn must be cleared by hand.
    fn with_fonts(mut f: impl FnMut(&egui::Context)) {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f(ui.ctx()));
        out.textures_delta.clear();
    }

    /// Where `c` is in the glyph atlas: a character with no glyph lands on
    /// the replacement `?`.
    fn atlas_rect(ctx: &egui::Context, font: &FontId, c: char) -> ([u16; 2], [u16; 2]) {
        let galley =
            ctx.fonts_mut(|f| f.layout_no_wrap(c.to_string(), font.clone(), Color32::WHITE));
        let uv = galley.rows[0].glyphs[0].uv_rect;
        (uv.min, uv.max)
    }

    #[test]
    fn fonts_cover_turkish_and_the_ui_symbols() {
        with_fonts(|ctx| {
            let needed = "\u{c7}\u{e7}\u{11e}\u{11f}\u{130}\u{131}\u{d6}\u{f6}\u{15e}\u{15f}\u{dc}\u{fc}\u{2318}\u{2190}\u{2192}\u{2033}\u{2032}\u{d7}\u{b7}\u{2026}\u{b0}\u{b1}\u{b2}\u{b3}\u{2013}\u{2014}\u{201c}\u{201d}\u{2039}\u{203a}\u{2212}\u{221a}\u{2248}\u{2264}\u{25cf}\u{25cb}\u{2022}\u{394}\u{3c3}";
            for font in [
                FontId::proportional(13.0),
                FontId::new(13.0, semibold()),
                FontId::monospace(12.0),
            ] {
                let missing = atlas_rect(ctx, &font, '?');
                for c in needed.chars() {
                    assert!(
                        atlas_rect(ctx, &font, c) != missing,
                        "{c} missing in {font:?}"
                    );
                }
            }
        });
    }

    /// Names in a script the app has no glyphs for show `?`, not a box, and
    /// `can_draw` agrees with the fonts on either side of that line.
    #[test]
    fn a_missing_character_is_a_question_mark_not_tofu() {
        with_fonts(|ctx| {
            let font = FontId::proportional(13.0);
            let question = atlas_rect(ctx, &font, '?');
            for c in "\u{674e}\u{5c0f}\u{627}\u{5d0}".chars() {
                assert!(atlas_rect(ctx, &font, c) == question, "{c}");
                assert!(!can_draw(c), "{c}");
            }
            for c in "A\u{15f}\u{130}\u{416}\u{3a9}".chars() {
                assert!(atlas_rect(ctx, &font, c) != question, "{c}");
                assert!(can_draw(c), "{c}");
            }
        });
    }
}
