use egui::{
    style::WidgetVisuals, Color32, CornerRadius, FontId, Shadow, Stroke, TextStyle, Theme, Visuals,
};

// Deep navy slate palette.
pub const BG: Color32 = Color32::from_rgb(0x0f, 0x14, 0x1c);
pub const CARD: Color32 = Color32::from_rgb(0x17, 0x1d, 0x27);
pub const RAISED: Color32 = Color32::from_rgb(0x1d, 0x25, 0x30);
pub const HOVER: Color32 = Color32::from_rgb(0x24, 0x2e, 0x3d);
pub const GRID: Color32 = Color32::from_rgb(0x28, 0x31, 0x41);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x3b, 0x47, 0x59);
pub const TEXT: Color32 = Color32::from_rgb(0xd7, 0xde, 0xe8);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x82, 0x93, 0xa8);
pub const BLUE: Color32 = Color32::from_rgb(0x4f, 0x8d, 0xff);
pub const BLUE_DARK: Color32 = Color32::from_rgb(0x36, 0x6b, 0xd0);
pub const BLUE_SOFT: Color32 = Color32::from_rgb(0x1b, 0x2a, 0x43);
pub const RED: Color32 = Color32::from_rgb(0xe5, 0x53, 0x4b);
pub const RED_DARK: Color32 = Color32::from_rgb(0xb4, 0x3d, 0x38);
pub const GREEN: Color32 = Color32::from_rgb(0x2f, 0xce, 0x72);
pub const AMBER: Color32 = Color32::from_rgb(0xe0, 0xa9, 0x4b);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    // Embed the visuals in the stored style: `set_style_of` replaces the whole
    // style object, so separate `set_visuals_of` + `set_style_of` would clash.
    let style = egui::Style {
        visuals: visuals(),
        ..style()
    };
    ctx.set_style_of(Theme::Dark, style);
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();

    // Layers: navy base → card panels → raised, hairline grid borders.
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = RAISED; // inputs, scrollbars
    v.faint_bg_color = GRID; // striped table rows
    v.code_bg_color = RAISED;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = BLUE;
    v.warn_fg_color = AMBER;
    v.error_fg_color = RED;

    v.window_corner_radius = CornerRadius::ZERO;
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.menu_corner_radius = CornerRadius::ZERO;

    v.selection.bg_fill = BLUE_SOFT;
    v.selection.stroke = Stroke::new(1.0, BLUE);

    widget(&mut v.widgets.noninteractive, CARD, GRID, TEXT_DIM);
    widget(&mut v.widgets.inactive, CARD, BORDER_STRONG, TEXT);
    widget(&mut v.widgets.hovered, HOVER, BLUE_DARK, TEXT);
    widget(&mut v.widgets.active, BLUE_SOFT, BLUE, TEXT);
    v.widgets.open = v.widgets.active;

    v
}

fn widget(w: &mut WidgetVisuals, fill: Color32, border: Color32, text: Color32) {
    w.bg_fill = fill;
    w.weak_bg_fill = fill;
    w.bg_stroke = Stroke::new(1.0, border);
    w.fg_stroke = Stroke::new(1.0, text);
    w.corner_radius = CornerRadius::ZERO;
    // Hover/active states otherwise expand the widget a fraction of a pixel,
    // reflowing neighbours and making lists feel jumpy.
    w.expansion = 0.0;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slate_theme_is_flat() {
        let v = visuals();
        for w in [
            &v.widgets.noninteractive,
            &v.widgets.inactive,
            &v.widgets.hovered,
            &v.widgets.active,
        ] {
            assert_eq!(w.corner_radius, CornerRadius::ZERO);
        }
        assert_eq!(v.window_corner_radius, CornerRadius::ZERO);
        assert_eq!(v.menu_corner_radius, CornerRadius::ZERO);
        assert_eq!(v.window_shadow, Shadow::NONE);
        assert_eq!(v.popup_shadow, Shadow::NONE);
    }
}
