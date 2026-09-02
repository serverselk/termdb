//! Reusable widget "lego" (action buttons, pills, cards) plus the assembly of
//! the central workspace from the per-area modules.

pub(crate) mod header;
pub(crate) mod query;
pub(crate) mod sidebar;
pub(crate) mod table;

use crate::app::TermdbApp;
use crate::theme;
use egui::{Color32, RichText, Stroke, Ui};

/// Fixed heights for the virtualized grids inside the scrolling workspace, so
/// their borders and frames never bleed into the sections around them.
pub(crate) const TABLE_GRID_HEIGHT: f32 = 420.0;
pub(crate) const RESULTS_GRID_HEIGHT: f32 = 260.0;

/// Blue primary action button ("+ NEW CONNECTION", "▶ RUN", "Save", "+ Add").
pub(crate) fn primary_button<'a>(text: &'a str) -> egui::Button<'a> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE).strong())
        .fill(theme::BLUE)
        .stroke(Stroke::new(1.0, theme::BLUE_DARK))
}

/// Outline/ghost button ("MCP: Stopped" style tags, secondary actions).
pub(crate) fn outline_button<'a>(text: &'a str) -> egui::Button<'a> {
    egui::Button::new(RichText::new(text).color(theme::TEXT_DIM))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, theme::BORDER_STRONG))
}

/// Borderless text button, mostly for inline chips.
pub(crate) fn ghost_button<'a>(text: &'a str) -> egui::Button<'a> {
    egui::Button::new(RichText::new(text).color(theme::TEXT))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
}

/// Compact uniform icon button (equally sized glyphs for × / 🗑 and friends).
pub(crate) fn icon_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).size(14.0).color(theme::TEXT))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
}

/// Outlined tag pill ("MCP: Stopped", "+ Add Filter", active-filter chips).
pub(crate) fn pill(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    egui::Frame::default()
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, theme::BORDER_STRONG))
        .corner_radius(0)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(text).small().color(color))
                    .sense(egui::Sense::click()),
            )
        })
        .inner
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Collapsible section header: `▾ TITLE` on the left, trailing widgets on the
/// right. Clicking the title toggles `open`.
pub(crate) fn section_header(
    ui: &mut Ui,
    open: &mut bool,
    title: &str,
    right: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        let arrow = if *open { "▾" } else { "▸" };
        let clicked = ui
            .selectable_label(false, RichText::new(format!("{arrow} {title}")).strong())
            .clicked();
        if clicked {
            *open = !*open;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            right(ui);
        });
    });
}

/// A framed card panel (navy card fill, hairline grid border).
pub(crate) fn card(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    egui::Frame::default()
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0, theme::GRID))
        .corner_radius(0)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, body);
}

/// A small colored status dot.
pub(crate) fn status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// The central workspace: a scrolling stack of cards.
pub(crate) fn workspace(app: &mut TermdbApp, ui: &mut Ui) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            card(ui, |ui| {
                app.ui_table_card(ui);
            });
            ui.add_space(10.0);
            card(ui, |ui| {
                app.ui_query_card(ui);
            });
            ui.add_space(10.0);
            card(ui, |ui| {
                app.ui_history_card(ui);
            });
            ui.add_space(10.0);
            card(ui, |ui| {
                app.ui_results_card(ui);
            });
            ui.add_space(4.0);
        });
}
