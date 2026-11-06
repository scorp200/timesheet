//! Colors: the softened light and dark themes, plus helpers for picking readable
//! colors on top of arbitrary fills.

use eframe::egui::{Color32, Context, Stroke, Style, Theme, Ui};

/// The colors that differ between the light and dark themes.
struct Palette {
    bg: Color32,
    /// Separators, grid lines, and window outlines.
    line: Color32,
    /// Text inputs.
    field_bg: Color32,
    /// Off-hours shading and alternating rows.
    faint_bg: Color32,
    text: Color32,
    /// Button text at rest.
    button_text: Color32,
    /// Text on hovered and pressed buttons.
    strong_text: Color32,
    button: Color32,
    button_hovered: Color32,
    button_pressed: Color32,
    /// Outline on buttons at rest, if any.
    button_stroke: Option<Color32>,
    button_hovered_stroke: Option<Color32>,
    button_pressed_stroke: Option<Color32>,
}

const DARK: Palette = Palette {
    bg: rgb(0x22, 0x27, 0x2e),
    line: rgb(0x44, 0x4c, 0x56),
    field_bg: rgb(0x1c, 0x21, 0x28),
    faint_bg: rgb(0x1e, 0x23, 0x2a),
    text: rgb(0xad, 0xba, 0xc7),
    button_text: rgb(0xc5, 0xd1, 0xde),
    strong_text: rgb(0xe2, 0xe8, 0xee),
    button: rgb(0x37, 0x3e, 0x47),
    button_hovered: rgb(0x44, 0x4c, 0x56),
    button_pressed: rgb(0x4d, 0x55, 0x60),
    button_stroke: None,
    button_hovered_stroke: None,
    button_pressed_stroke: None,
};

const LIGHT: Palette = Palette {
    bg: rgb(0xf6, 0xf8, 0xfa),
    line: rgb(0xd0, 0xd7, 0xde),
    field_bg: Color32::WHITE,
    faint_bg: rgb(0xe6, 0xea, 0xef),
    text: rgb(0x42, 0x4a, 0x53),
    button_text: rgb(0x32, 0x39, 0x41),
    strong_text: rgb(0x1f, 0x23, 0x28),
    button: rgb(0xdd, 0xe3, 0xe9),
    button_hovered: rgb(0xcf, 0xd7, 0xdf),
    button_pressed: rgb(0xbb, 0xc5, 0xcf),
    button_stroke: Some(rgb(0xc2, 0xca, 0xd3)),
    button_hovered_stroke: Some(rgb(0x8c, 0x95, 0x9f)),
    button_pressed_stroke: Some(rgb(0x57, 0x60, 0x6a)),
};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Replaces egui's stock grays with softer slate tones in both themes.
pub fn install(ctx: &Context) {
    ctx.style_mut_of(Theme::Dark, |style| apply(style, &DARK));
    ctx.style_mut_of(Theme::Light, |style| apply(style, &LIGHT));
}

fn apply(style: &mut Style, p: &Palette) {
    let v = &mut style.visuals;
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.field_bg;
    v.faint_bg_color = p.faint_bg;
    v.window_stroke.color = p.line;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg;
    w.noninteractive.weak_bg_fill = p.bg;
    w.noninteractive.bg_stroke.color = p.line;
    w.noninteractive.fg_stroke.color = p.text;

    for (state, fill, stroke, fg) in [
        (&mut w.inactive, p.button, p.button_stroke, p.button_text),
        (
            &mut w.hovered,
            p.button_hovered,
            p.button_hovered_stroke,
            p.strong_text,
        ),
        (
            &mut w.active,
            p.button_pressed,
            p.button_pressed_stroke,
            p.strong_text,
        ),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.fg_stroke.color = fg;
        if let Some(color) = stroke {
            state.bg_stroke = Stroke::new(1.0, color);
        }
    }

    w.open.bg_fill = p.bg;
    w.open.weak_bg_fill = p.button;
    w.open.bg_stroke.color = p.line;
    w.open.fg_stroke.color = p.strong_text;
}

/// The highlight color for progress and similar indicators.
pub fn accent(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        rgb(0x44, 0x93, 0xf8)
    } else {
        rgb(0x09, 0x69, 0xda)
    }
}

/// Black or white, whichever reads better on `bg`.
pub fn text_color_for(bg: Color32) -> Color32 {
    let lum = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if lum > 150.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// A shade of `color` that stands out against it, shifted toward its text color:
/// darker for light colors, lighter for dark ones.
pub fn border_color_for(color: Color32) -> Color32 {
    color.lerp_to_gamma(text_color_for(color), 0.35)
}
