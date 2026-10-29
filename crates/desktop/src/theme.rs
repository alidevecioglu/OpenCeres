//! Colours, spacing and widget styling for the light and dark themes.
//!
//! One spacing scale (4/8/12/16/24), one corner radius, calm neutral
//! surfaces, a single blue accent for primary and selected state, amber
//! only for the report grid and teal-green only for known objects. The image stage is dark in both themes.

use eframe::egui::{
    self, Color32, CornerRadius, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Theme,
    Visuals,
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
    pub border: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    /// Background of selected toggles.
    pub accent_weak: Color32,
    pub hover: Color32,
    /// Background of inset boxes (report text, key caps).
    pub inset: Color32,
    pub danger_bg: Color32,
    pub danger: Color32,
    pub disc: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const LIGHT: Palette = Palette {
    bg: rgb(0xeef0f3),
    panel: rgb(0xfbfbfc),
    text: rgb(0x16191f),
    muted: rgb(0x596272), // 6.0:1 on panel
    border: rgb(0xdcdfe5),
    accent: rgb(0x1f5fd1),
    accent_text: rgb(0xffffff),
    accent_weak: rgb(0xe1eafb),
    hover: rgb(0xebedf1),
    inset: rgb(0xf0f2f5),
    danger_bg: rgb(0xfcebea),
    danger: rgb(0xa3221c),
    disc: rgb(0x1b3a6b),
};

pub const DARK: Palette = Palette {
    bg: rgb(0x16191f),
    panel: rgb(0x1e2229),
    text: rgb(0xe6e9ee),
    muted: rgb(0x9aa3b1), // 6.4:1 on panel
    border: rgb(0x30363f),
    accent: rgb(0x5b8ff0),
    accent_text: rgb(0x0d1117),
    accent_weak: rgb(0x26354f),
    hover: rgb(0x2a2f38),
    inset: rgb(0x262a32),
    danger_bg: rgb(0x3a1f1f),
    danger: rgb(0xffb1aa),
    disc: rgb(0x3b6fc4),
};

// Image stage and what is drawn on it look the same in both themes.
pub const STAGE: Color32 = rgb(0x111318);
pub const BADGE: Color32 = Color32::from_rgba_premultiplied(5, 6, 8, 140); // black at 55 %
pub const BADGE_TEXT: Color32 = rgb(0xb4bbc6);
pub const STAR: Color32 = rgb(0x4cc2ff);
pub const STREAK: Color32 = rgb(0xf2b33d);
pub const GRID_LINE: Color32 = Color32::from_rgba_premultiplied(204, 141, 26, 204); // rgba(255,176,32,.8)
pub const GRID_FILL: Color32 = Color32::from_rgba_premultiplied(71, 49, 9, 71); // rgba(255,176,32,.28)
pub const GRID_CHIP: Color32 = Color32::from_rgba_premultiplied(13, 15, 19, 184); // rgba(18,21,27,.72)
pub const GRID_TEXT: Color32 = rgb(0xffc04d);
/// Known-object rings and labels: teal-green, apart from the blue star
/// rings and the amber grid, with a dark halo so they read on an inverted
/// (white) image too.
pub const KNOWN: Color32 = rgb(0x3ddc97);
pub const KNOWN_HALO: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 150);
pub const KNOWN_TEXT: Color32 = rgb(0x7af0bd);
pub const OVERLAY_TEXT: Color32 = rgb(0xf1f3f6);

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
        state.bg_stroke = Stroke::new(1.0, p.border);
        state.fg_stroke.color = p.text;
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    w.inactive.bg_fill = p.inset;
    w.hovered.bg_stroke = Stroke::new(1.0, p.muted.gamma_multiply(0.6));
    v
}

pub fn install(ctx: &egui::Context) {
    // The built-in proportional font lacks ″ and arrows; the monospace one
    // (already linked in) has them, so use it as a fallback.
    let mut fonts = FontDefinitions::default();
    if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
        list.insert(1, "Hack".to_owned());
    }
    ctx.set_fonts(fonts);

    for (theme, p) in [(Theme::Light, &LIGHT), (Theme::Dark, &DARK)] {
        ctx.set_visuals_of(theme, visuals(p, theme == Theme::Dark));
    }
    ctx.all_styles_mut(|s| {
        use FontFamily::{Monospace, Proportional};
        s.text_styles = [
            (TextStyle::Small, FontId::new(11.5, Proportional)),
            (TextStyle::Body, FontId::new(13.0, Proportional)),
            (TextStyle::Button, FontId::new(13.0, Proportional)),
            (TextStyle::Heading, FontId::new(17.0, Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, Monospace)),
        ]
        .into();
        s.spacing.button_padding = egui::vec2(S2 + 2.0, 3.0);
        s.spacing.item_spacing = egui::vec2(S2, S2 - 2.0);
        s.spacing.interact_size.y = TOOL;
        s.spacing.window_margin = egui::Margin::same(S4 as i8);
        s.spacing.menu_margin = egui::Margin::same(S1 as i8 + 2);
        s.interaction.tooltip_delay = 0.4;
    });
}
