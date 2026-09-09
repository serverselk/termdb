//! termdb Tauri shell — Milestone 1.
//!
//! Side-by-side with the egui app (`crates/app`): the web UI talks to these
//! `tauri::command`s instead of the `mpsc Request/Event` channels. Live
//! database sessions (sqlx pools) arrive in Milestone 2 via engine
//! extraction; this milestone proves IPC + shared core + theming.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod palette;

use serde::Serialize;
use termdb_core::{ConfigStore, ConnectionConfig};

/// Minimal round-trip proving the webview ↔ Rust bridge works.
#[derive(Debug, Clone, Serialize)]
struct Pong {
    payload: String,
    latency_ms: u64,
}

#[tauri::command]
fn ping(payload: String) -> Pong {
    let started = std::time::Instant::now();
    Pong {
        payload,
        latency_ms: started.elapsed().as_millis() as u64,
    }
}

/// Saved connections, read from the same SQLite store as the egui app.
#[tauri::command]
fn list_connections() -> Result<Vec<ConnectionConfig>, String> {
    let store = ConfigStore::open(&ConfigStore::default_path()).map_err(|e| e.to_string())?;
    store.list_connections().map_err(|e| e.to_string())
}

/// Active UI palette (Omarchy theme on Linux, slate fallback elsewhere).
#[tauri::command]
fn get_palette() -> palette::PaletteDto {
    palette::current()
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            ping,
            list_connections,
            get_palette
        ])
        .run(tauri::generate_context!())
        .expect("error while running termdb");
}
