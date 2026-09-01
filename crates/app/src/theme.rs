//! Dark terminal theme matching tup-db-client: near-black layers, thin square
//! borders, monospace everywhere, terminal-green accent. No rounded corners.

use egui::{
    style::WidgetVisuals, Color32, CornerRadius, FontId, Shadow, Stroke, TextStyle, Theme, Visuals,
};

const BG: Color32 = Color32::from_rgb(0x0c, 0x0c, 0x0c);
const PANEL: Color32 = Color32::from_rgb(0x13, 0x13, 0x13);
const RAISED: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1a);
const HOVER: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);
const BORDER: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x2b);
const BORDER_STRONG: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3a);
const TEXT: Color32 = Color32::from_rgb(0xd6, 0xd6, 0xd6);
const TEXT_DIM: Color32 = Color32::from_rgb(0x70, 0x70, 0x70);
const ACCENT: Color32 = Color32::from_rgb(0x7b, 0xc9, 0x6f);
const ACCENT_DIM: Color32 = Color32::from_rgb(0x67, 0xa8, 0x5c);
const ACCENT_DARK: Color32 = Color32::from_rgb(0x20, 0x30, 0x1d);
const DANGER: Color32 = Color32::from_rgb(0xf4, 0x87, 0x71);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(Theme::Dark, visuals());
    ctx.set_style_of(Theme::Dark, style());
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();

    // Layers: bg → panel → raised, borders between them.
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG; // inputs, scrollbars
    v.faint_bg_color = BG; // striped table rows (subtle)
    v.code_bg_color = RAISED;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = Color32::from_rgb(0xd6, 0xb8, 0x6f);
    v.error_fg_color = DANGER;

    // Flat, TUI-style: square corners, no shadows, hairline borders.
    v.window_corner_radius = CornerRadius::ZERO;
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.menu_corner_radius = CornerRadius::ZERO;

    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    widget(&mut v.widgets.noninteractive, RAISED, BORDER, TEXT_DIM);
    widget(&mut v.widgets.inactive, RAISED, BORDER_STRONG, TEXT);
    widget(&mut v.widgets.hovered, HOVER, ACCENT_DIM, TEXT);
    widget(&mut v.widgets.active, ACCENT_DARK, ACCENT, TEXT);
    v.widgets.open = v.widgets.active;

    v
}

fn widget(w: &mut WidgetVisuals, fill: Color32, border: Color32, text: Color32) {
    w.bg_fill = fill;
    w.weak_bg_fill = fill;
    w.bg_stroke = Stroke::new(1.0, border);
    w.fg_stroke = Stroke::new(1.0, text);
    w.corner_radius = CornerRadius::ZERO;
}

fn style() -> egui::Style {
    egui::Style {
        text_styles: [
            (TextStyle::Heading, FontId::monospace(16.0)),
            (TextStyle::Body, FontId::monospace(14.0)),
            (TextStyle::Button, FontId::monospace(13.0)),
            (TextStyle::Small, FontId::monospace(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into(),
        // Denser, terminal-ish spacing with thin square scrollbars.
        spacing: egui::Spacing {
            item_spacing: egui::vec2(6.0, 4.0),
            button_padding: egui::vec2(10.0, 4.0),
            interact_size: egui::vec2(28.0, 18.0),
            window_margin: egui::Margin::symmetric(8, 8),
            menu_margin: egui::Margin::symmetric(4, 4),
            scroll: egui::style::ScrollStyle {
                floating: true,
                bar_width: 8.0,
                bar_inner_margin: 2.0,
                bar_outer_margin: 0.0,
                handle_min_length: 20.0,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    }
}
