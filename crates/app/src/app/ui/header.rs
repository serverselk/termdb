//! Top header bar: app title on the left, quick actions on the right.

use egui::RichText;

use super::{outline_button, pill, primary_button};
use crate::app::{BackendStatus, TermdbApp};
use crate::theme;

pub(crate) fn ui_header_bar(app: &mut TermdbApp, root: &mut egui::Ui) {
    egui::Panel::top("header")
        .exact_size(44.0)
        .show(root, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(
                    RichText::new("TERMDB")
                        .monospace()
                        .strong()
                        .size(17.0)
                        .color(theme::TEXT),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    pill(ui, "MCP: Stopped", theme::TEXT_DIM);
                    ui.add_space(12.0);

                    if ui.add(primary_button("+ NEW CONNECTION")).clicked() {
                        app.show_new_connection = true;
                    }
                    ui.add_space(10.0);

                    let gear = "⚙";
                    if ui
                        .add(
                            egui::Button::new(RichText::new(gear).size(14.0))
                                .fill(theme::CARD)
                                .stroke(egui::Stroke::new(1.0, theme::BORDER)),
                        )
                        .on_hover_text("Settings")
                        .clicked()
                    {
                        app.show_settings = true;
                    }
                    ui.add_space(4.0);
                });
            });
        });
}

/// Settings window: config/store paths, vault kind, backend round-trip.
pub(crate) fn ui_settings_window(app: &mut TermdbApp, ctx: &egui::Context) {
    if !app.show_settings {
        return;
    }
    let mut open = true;
    let mut close = false;
    egui::Window::new("Settings")
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Config store");
                    ui.label(
                        termdb_core::ConfigStore::default_path()
                            .display()
                            .to_string(),
                    );
                    ui.end_row();
                    ui.label("Saved connections");
                    ui.label(format!("{}", app.connections.len()));
                    ui.end_row();
                    ui.label("Backend");
                    match &app.backend_status {
                        BackendStatus::Starting => {
                            ui.label("starting…");
                        }
                        BackendStatus::Ready {
                            version,
                            vault_kind,
                            last_pong,
                        } => {
                            let mut s = format!("ready v{version} · {}", vault_kind.label());
                            if let Some(pong) = last_pong {
                                s.push_str(&format!(
                                    " · last ping \"{}\": {}ms",
                                    pong.payload, pong.latency_ms
                                ));
                            }
                            ui.label(s);
                        }
                    }
                    ui.end_row();
                });
            ui.add_space(6.0);
            let ready = matches!(app.backend_status, BackendStatus::Ready { .. });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(ready, outline_button("Ping backend"))
                    .clicked()
                {
                    app.backend.send(termdb::db::Request::Ping {
                        payload: "hello".into(),
                    });
                    app.push_log("ping sent".into());
                }
                if ui.add(outline_button("Logs")).clicked() {
                    app.show_logs = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(outline_button("Close")).clicked() {
                        close = true;
                    }
                });
            });
        });
    if !open || close {
        app.show_settings = false;
    }
}

/// Logs modal, opened from Settings → Logs.
pub(crate) fn ui_logs_window(app: &mut TermdbApp, ctx: &egui::Context) {
    if !app.show_logs {
        return;
    }
    let mut open = true;
    let mut clear = false;
    egui::Window::new("Logs")
        .open(&mut open)
        .default_width(600.0)
        .default_height(380.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} entries", app.log.len()))
                        .small()
                        .color(theme::TEXT_DIM),
                );
                if ui.small_button("Clear").clicked() {
                    clear = true;
                }
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if app.log.is_empty() {
                        ui.label(
                            RichText::new("no log entries")
                                .color(theme::TEXT_DIM)
                                .italics(),
                        );
                    }
                    for line in &app.log {
                        ui.label(RichText::new(line).monospace().small());
                    }
                });
        });
    if clear {
        app.log.clear();
    }
    if !open {
        app.show_logs = false;
    }
}
