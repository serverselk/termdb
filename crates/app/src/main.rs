//! termdb — a lightweight multi-engine database tool.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod theme;

use app::TermdbApp;
use termdb::db::Backend;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("termdb")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };

    let backend = Backend::spawn();

    eframe::run_native(
        "termdb",
        native_options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(TermdbApp::new(cc, backend)))
        }),
    )
}
