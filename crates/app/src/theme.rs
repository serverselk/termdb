//! Dark terminal palette shared across the whole app.

use egui::{Color32, FontId, TextStyle, Theme, Visuals};

// Terminal-ish palette (material-one-dark flavored).
const BG: Color32 = Color32::from_rgb(0x0e, 0x11, 0x16);
const PANEL: Color32 = Color32::from_rgb(0x13, 0x17, 0x1d);
const RAISED: Color32 = Color32::from_rgb(0x1a, 0x20, 0x28);
const CODE_BG: Color32 = Color32::from_rgb(0x18, 0x1d, 0x24);
const TEXT: Color32 = Color32::from_rgb(0xc9, 0xd1, 0xd9);
const TEXT_DIM: Color32 = Color32::from_rgb(0xad, 0xba, 0xc7);
const GREEN: Color32 = Color32::from_rgb(0x56, 0xd3, 0x64);
const WARN: Color32 = Color32::from_rgb(0xe3, 0xb3, 0x41);
const ERROR: Color32 = Color32::from_rgb(0xff, 0x53, 0x70);
const HOVER: Color32 = Color32::from_rgb(0x21, 0x29, 0x33);
const ACTIVE: Color32 = Color32::from_rgb(0x24, 0x2e, 0x3a);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(Theme::Dark, visuals());
    ctx.set_style_of(Theme::Dark, style());
}

fn visuals() -> Visuals {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = RAISED;
    visuals.faint_bg_color = BG;
    visuals.code_bg_color = CODE_BG;
    visuals.override_text_color = Some(TEXT);
    visuals.hyperlink_color = GREEN;
    visuals.warn_fg_color = WARN;
    visuals.error_fg_color = ERROR;
    visuals.widgets.inactive.bg_fill = RAISED;
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
    visuals.widgets.hovered.bg_fill = HOVER;
    visuals.widgets.active.bg_fill = ACTIVE;
    visuals
}

fn style() -> egui::Style {
    egui::Style {
        text_styles: [
            (TextStyle::Heading, FontId::proportional(16.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into(),
        spacing: egui::Spacing {
            item_spacing: egui::vec2(8.0, 6.0),
            ..Default::default()
        },
        ..Default::default()
    }
}
