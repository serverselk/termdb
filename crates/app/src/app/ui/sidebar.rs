//! Left sidebar: connections list, database/table tree, bottom disconnect
//! dock, and the new-connection modal (opened from the header).

use egui::RichText;

use super::{ghost_button, icon_button, primary_button, status_dot};
use crate::app::TermdbApp;
use crate::theme;
use termdb::db::Request;
use termdb_core::Engine;

type TableKey = crate::app::TableKey;

pub(crate) fn ui_sidebar(app: &mut TermdbApp, root: &mut egui::Ui) {
    egui::Panel::left("sidebar")
        .resizable(false)
        .exact_size(264.0)
        .show(root, |ui| {
            // Scrollable connection + database lists (top).
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("CONNECTIONS")
                            .small()
                            .strong()
                            .color(theme::TEXT_DIM),
                    );
                    app.ui_connection_list(ui);
                    ui.add_space(16.0);
                    app.ui_favorites_section(ui);
                    if app.favorites.is_empty() {
                        ui.add_space(16.0);
                    } else {
                        ui.add_space(10.0);
                    }
                    ui.label(
                        RichText::new("DATABASES")
                            .small()
                            .strong()
                            .color(theme::TEXT_DIM),
                    );
                    app.ui_database_tree(ui);
                    ui.add_space(8.0);
                });

            // Bottom dock removed — per-connection actions live on each row (× for
            // disconnect, 🗑 for delete). The sidebar now simply scrolls.
        });
}

impl TermdbApp {
    /// One row per saved connection: status light + name + close button.
    fn ui_connection_list(&mut self, ui: &mut egui::Ui) {
        if self.connections.is_empty() {
            ui.label(
                RichText::new("no saved connections")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            ui.label(
                RichText::new("use \"+ NEW CONNECTION\" in the header")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            return;
        }
        let ids: Vec<i64> = self.connections.iter().filter_map(|c| c.id).collect();
        for id in ids {
            let Some(cfg) = self.connections.iter().find(|c| c.id == Some(id)) else {
                continue;
            };
            let connected = self.live.contains_key(&id);
            let connecting = self.live.get(&id).map(|l| l.connecting).unwrap_or(false);
            let selected = self.selected_id == Some(id);
            let name = cfg.name.clone();

            let mut select = false;
            let mut close = false;
            let mut delete = false;
            ui.horizontal(|ui| {
                let color = if connecting {
                    theme::AMBER
                } else if connected {
                    theme::GREEN
                } else {
                    theme::BORDER_STRONG
                };
                status_dot(ui, color);
                let label = ui.selectable_label(selected, &name);
                if label.clicked() {
                    select = true;
                }
                if connected
                    && ui
                        .add(icon_button("×"))
                        .on_hover_text("Disconnect")
                        .clicked()
                {
                    close = true;
                }
                if ui
                    .add(icon_button("🗑"))
                    .on_hover_text("Remove connection")
                    .clicked()
                {
                    delete = true;
                }
            });

            if select {
                self.selected_id = Some(id);
                if !connected && !connecting {
                    self.connect_to(id);
                }
            }
            if close {
                self.backend.send(Request::Disconnect { conn_id: id });
            }
            if delete {
                self.request_delete_connection(id, &name);
            }
        }
    }

    /// Favorite (starred) tables, surfaced at the top of the sidebar.
    fn ui_favorites_section(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("FAVORITES")
                .small()
                .strong()
                .color(theme::TEXT_DIM),
        );
        if self.favorites.is_empty() {
            ui.label(
                RichText::new("no favourites — star a table below")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            return;
        }
        let keys: Vec<TableKey> = self.favorites.iter().cloned().collect();
        for key in keys {
            let (conn_id, _, _) = &key;
            let live = self.live.contains_key(conn_id);
            ui.horizontal(|ui| {
                let active = self.table_selection.as_ref() == Some(&key);
                let label = ui.selectable_label(active, format!("{}.{}", key.1, key.2));
                if label.clicked() {
                    self.open_table(key.clone());
                }
                if ui.add(ghost_button("★").small()).clicked() {
                    self.toggle_favorite(key);
                }
                if !live {
                    ui.label(RichText::new("•").small().color(theme::TEXT_DIM));
                }
            });
        }
    }

    /// Databases (collapsing) → tables underneath every live connection.
    fn ui_database_tree(&mut self, ui: &mut egui::Ui) {
        let live_ids: Vec<i64> = self.live.keys().copied().collect();
        if live_ids.is_empty() {
            ui.label(
                RichText::new("connect to a server to browse databases")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            return;
        }
        for conn_id in live_ids {
            let Some(live) = self.live.get(&conn_id) else {
                continue;
            };
            if live.connecting || live.databases.is_empty() {
                continue;
            }
            let databases = live.databases.clone();

            // Defer `open_table` (a whole-self method) until the `live` borrow
            // ends, so the tree row closures only touch disjoint fields.
            let mut requested_opens: Vec<TableKey> = Vec::new();
            {
                let live = self.live.get_mut(&conn_id).expect("connection still live");
                for db in databases {
                    let tables = live.tables.get(&db).cloned();
                    let loading = live.loading_tables.contains(&db);

                    let resp = egui::CollapsingHeader::new(RichText::new(&db).strong())
                        .id_salt(("db", conn_id, &db))
                        .show(ui, |ui| {
                            if let Some(ts) = &tables {
                                for table in ts {
                                    let key = (conn_id, db.clone(), table.clone());
                                    let active = live.selected_database.as_deref()
                                        == Some(db.as_str())
                                        && live.selected_table.as_deref() == Some(table.as_str());
                                    let starred = self.favorites.contains(&key);
                                    ui.horizontal(|ui| {
                                        if ui.selectable_label(active, table).clicked() {
                                            requested_opens.push(key.clone());
                                        }
                                        ui.add_space(4.0);
                                        if starred {
                                            if ui
                                                .add(ghost_button("★").small())
                                                .on_hover_text("Unfavorite")
                                                .clicked()
                                            {
                                                self.favorites.remove(&key);
                                            }
                                        } else if ui
                                            .add(ghost_button("☆").small())
                                            .on_hover_text("Favorite")
                                            .clicked()
                                        {
                                            self.favorites.insert(key);
                                        }
                                    });
                                }
                            } else if loading {
                                ui.label(RichText::new("loading…").small().color(theme::TEXT_DIM));
                            } else {
                                ui.label(
                                    RichText::new("click to load")
                                        .small()
                                        .color(theme::TEXT_DIM),
                                );
                            }
                        });

                    let needs_load = tables.is_none() && !loading;
                    if resp.header_response.clicked() && needs_load {
                        live.loading_tables.insert(db.clone());
                        live.selected_database = Some(db.clone());
                        self.backend.send(Request::ListTables {
                            conn_id,
                            database: db,
                        });
                    }
                }
            }
            for key in requested_opens {
                self.open_table(key);
            }
        }
    }
}

/// New-connection modal form (opened from the header "+ NEW CONNECTION").
pub(crate) fn ui_new_connection_window(app: &mut TermdbApp, ctx: &egui::Context) {
    if !app.show_new_connection {
        return;
    }
    let mut open = true;
    let mut save = false;
    let mut cancel = false;
    egui::Window::new("New Connection")
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            egui::Grid::new("conn_form")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut app.form.name);
                    ui.end_row();

                    ui.label("Engine");
                    let prev = app.form.engine;
                    egui::ComboBox::from_id_salt("conn_engine")
                        .selected_text(app.form.engine.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut app.form.engine,
                                Engine::Postgres,
                                "PostgreSQL",
                            );
                            ui.selectable_value(&mut app.form.engine, Engine::Mysql, "MySQL");
                        });
                    if app.form.engine != prev {
                        app.form.port = app.form.engine.default_port();
                    }
                    ui.end_row();

                    ui.label("Host");
                    ui.text_edit_singleline(&mut app.form.host);
                    ui.end_row();

                    ui.label("Port");
                    ui.add(egui::DragValue::new(&mut app.form.port).range(1..=65535));
                    ui.end_row();

                    ui.label("Username");
                    ui.text_edit_singleline(&mut app.form.username);
                    ui.end_row();

                    ui.label("Password");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.form.password)
                            .password(true)
                            .hint_text("stored in the vault"),
                    );
                    ui.end_row();

                    ui.label("Database");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.form.database)
                            .hint_text("optional initial database"),
                    );
                    ui.end_row();
                });

            if let Some(problem) = app.form.first_problem() {
                ui.label(RichText::new(problem).color(theme::RED));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!app.saving, primary_button("Save connection"))
                        .clicked()
                        && app.form.first_problem().is_none()
                    {
                        save = true;
                    }
                    if ui.add(super::outline_button("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        });
    if cancel {
        open = false;
    }
    if save {
        app.save_form();
    }
    if !open {
        app.show_new_connection = false;
    }
}
