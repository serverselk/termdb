//! Table card: active table header, "+ Add", filter builder, and the
//! virtualized data grid with per-row actions. Includes a demo grid matching
//! the mockup when no live table is open.

use egui::{Align, Layout, RichText};
use egui_extras::{Column as TableColumn, TableBuilder};

use super::{outline_button, pill, primary_button};
use crate::app::{single_pk, RecordAction, RecordPanelMode, TermdbApp};
use crate::theme;
use termdb::db::engine::{Column, TableFilter, FILTER_OPS};
use termdb::db::Request;

impl TermdbApp {
    /// The active-table card, collapsible like the other sections. Falls back
    /// to a hint when no table is open.
    pub(crate) fn ui_table_card(&mut self, ui: &mut egui::Ui) {
        let mut open = self.table_open;
        let mut add = false;

        if let Some(key) = self.table_selection.clone() {
            let (_, db, table) = &key;
            let title = format!("TABLE: {db}.{table}");
            let loaded = self
                .open_tables
                .get(&key)
                .map(|o| !o.columns.is_empty())
                .unwrap_or(false);
            let total = self.open_tables.get(&key).map(|o| o.total).unwrap_or(0);

            super::section_header(ui, &mut open, &title, |ui| {
                if loaded {
                    ui.label(
                        RichText::new(format!("{total} rows"))
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                    if ui
                        .add(primary_button("+ Add"))
                        .on_hover_text("Insert a new row")
                        .clicked()
                    {
                        add = true;
                    }
                }
            });

            if add {
                self.open_record_panel(&key, RecordPanelMode::Add, None);
            }
            if open {
                ui.separator();
                self.ui_table_view(ui, key);
            }
        } else {
            super::section_header(ui, &mut open, "TABLE: (no table selected)", |_| {});
            if open {
                ui.separator();
                ui.add_space(8.0);
                ui.label(
                    RichText::new("pick a table in the sidebar to browse it")
                        .small()
                        .color(theme::TEXT_DIM),
                );
                ui.label(
                    RichText::new("or run an ad-hoc query in the QUERY EDITOR below")
                        .small()
                        .color(theme::TEXT_DIM),
                );
                ui.add_space(8.0);
            }
        }
        self.table_open = open;
    }

    /// The opened table: header toolbar, filter builder, paginated grid.
    fn ui_table_view(&mut self, ui: &mut egui::Ui, key: crate::app::TableKey) {
        let (conn_id, database, table) = key.clone();
        if !self.live.contains_key(&conn_id) {
            self.table_selection = None;
            return;
        }

        // Lazy open on first render of this table.
        if !self.open_tables.contains_key(&key) {
            self.open_tables.insert(
                key.clone(),
                crate::app::OpenTable::placeholder(self.page_size),
            );
            self.backend.send(Request::OpenTable {
                conn_id,
                database: database.clone(),
                table: table.clone(),
                filter: None,
                limit: self.page_size,
            });
            self.push_log(format!("opening \"{database}.{table}\"…"));
        }

        let loading = self
            .open_tables
            .get(&key)
            .map(|o| o.loading)
            .unwrap_or(false);
        let loaded = self
            .open_tables
            .get(&key)
            .map(|o| !o.columns.is_empty())
            .unwrap_or(false);
        let page = self.open_tables.get(&key).map(|o| o.page).unwrap_or(0);
        let total = self.open_tables.get(&key).map(|o| o.total).unwrap_or(0);
        let pages = crate::app::total_pages(
            total,
            self.open_tables
                .get(&key)
                .map(|o| o.page_size)
                .unwrap_or(self.page_size),
        );
        let on_last = page + 1 >= pages;
        let prev_size = self.page_size;

        // Page toolbar (right-aligned).
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if loading {
                    ui.label(RichText::new("…").color(theme::TEXT_DIM));
                }
                ui.label(format!("{total} rows"));
                let next =
                    ui.add_enabled(loaded && !on_last && !loading, egui::Button::new("Next ›"));
                if next.clicked() {
                    self.goto_page(&key, page + 1);
                }
                if loaded {
                    ui.label(format!("page {} / {}", page + 1, pages));
                }
                let prev =
                    ui.add_enabled(loaded && page > 0 && !loading, egui::Button::new("‹ Prev"));
                if prev.clicked() {
                    self.goto_page(&key, page - 1);
                }
                egui::ComboBox::from_id_salt("page_size")
                    .selected_text(format!("{} / page", self.page_size))
                    .show_ui(ui, |ui| {
                        for size in [25i64, 50, 100, 200] {
                            ui.selectable_value(
                                &mut self.page_size,
                                size,
                                format!("{size} / page"),
                            );
                        }
                    });
            });
        });

        // Page-size change reloads from page 0.
        if self.page_size != prev_size && loaded {
            if let Some(open) = self.open_tables.get_mut(&key) {
                open.page_size = self.page_size;
            }
            self.goto_page(&key, 0);
        }

        if !loaded {
            ui.label(
                RichText::new("loading table…")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            return;
        }

        let columns = self
            .open_tables
            .get(&key)
            .map(|o| o.columns.clone())
            .unwrap_or_default();
        let row_count = self
            .open_tables
            .get(&key)
            .map(|o| o.rows.len())
            .unwrap_or(0);
        let active_filter = self.open_tables.get(&key).and_then(|o| o.filter.clone());

        // Filter builder row: pill toggles the builder; active filter shows a chip.
        let col_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
        if !col_names.is_empty() && !col_names.contains(&self.filter_col) {
            self.filter_col = col_names[0].clone();
        }

        ui.horizontal(|ui| {
            if pill(ui, "+ Add Filter", theme::BLUE).clicked() {
                self.filter_open = !self.filter_open;
            }
            if let Some(f) = &active_filter {
                if pill(
                    ui,
                    &format!("{} {op} \"{}\"", f.column, f.value, op = f.op),
                    theme::AMBER,
                )
                .clicked()
                {
                    self.set_filter(&key, None);
                }
            }
            if self.filter_open {
                ui.add_space(8.0);
                egui::ComboBox::from_id_salt("filter_col")
                    .width(120.0)
                    .selected_text(self.filter_col.clone())
                    .show_ui(ui, |ui| {
                        for c in &col_names {
                            ui.selectable_value(&mut self.filter_col, c.clone(), c);
                        }
                    });
                egui::ComboBox::from_id_salt("filter_op")
                    .selected_text(self.filter_op.clone())
                    .show_ui(ui, |ui| {
                        for op in FILTER_OPS {
                            let value = (*op).to_owned();
                            ui.selectable_value(&mut self.filter_op, value.clone(), value);
                        }
                    });
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter_val)
                        .hint_text("value")
                        .desired_width(150.0),
                );
                if ui.add(outline_button("Apply")).clicked() && !self.filter_val.is_empty() {
                    let filter = TableFilter {
                        column: self.filter_col.clone(),
                        op: self.filter_op.clone(),
                        value: self.filter_val.clone(),
                    };
                    self.set_filter(&key, Some(filter));
                }
                if active_filter.is_some() && ui.small_button("Clear").clicked() {
                    self.set_filter(&key, None);
                }
            }
        });
        ui.separator();

        // The grid.
        self.ui_table_grid(ui, &key, &columns, row_count, active_filter.is_some());
    }

    fn ui_table_grid(
        &mut self,
        ui: &mut egui::Ui,
        key: &crate::app::TableKey,
        columns: &[Column],
        row_count: usize,
        _filtered: bool,
    ) {
        let has_pk = single_pk(columns).is_some();
        let mut clicked: Option<usize> = None;
        let mut double_clicked: Option<usize> = None;
        let mut row_action: Option<(usize, RecordAction)> = None;
        let ctx = ui.ctx().clone();

        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .max_scroll_height(super::TABLE_GRID_HEIGHT)
            .cell_layout(Layout::left_to_right(Align::LEFT))
            .column(TableColumn::auto());
        for _ in columns.iter().skip(1) {
            table = table.column(TableColumn::auto());
        }
        if has_pk {
            table = table.column(TableColumn::exact(130.0));
        }
        table
            .header(38.0, |mut header| {
                for col in columns {
                    header.col(|ui| {
                        let title = if col.key == "PRI" {
                            format!("{} (PK)", col.name)
                        } else {
                            col.name.clone()
                        };
                        let tooltip = format!(
                            "Type: {}\nNull: {}\nKey: {}\nDefault: {}\nExtra: {}",
                            col.ty,
                            if col.nullable { "yes" } else { "no" },
                            if col.key.is_empty() { "—" } else { &col.key },
                            col.default.as_deref().unwrap_or("—"),
                            if col.extra.is_empty() {
                                "—"
                            } else {
                                &col.extra
                            },
                        );
                        ui.vertical(|ui| {
                            ui.strong(title);
                            ui.label(RichText::new(&col.ty).small().color(theme::TEXT_DIM));
                        })
                        .response
                        .on_hover_text(tooltip);
                    });
                }
                if has_pk {
                    header.col(|_| {});
                }
            })
            .body(|body| {
                body.rows(22.0, row_count, |mut row| {
                    let i = row.index();
                    let selected = self
                        .open_tables
                        .get(key)
                        .map(|o| o.selected_row == Some(i))
                        .unwrap_or(false);
                    row.set_selected(selected);
                    if let Some(open) = self.open_tables.get(key) {
                        if let Some(cells) = open.rows.get(i) {
                            for cell in cells {
                                row.col(|ui| match cell {
                                    Some(value) => {
                                        ui.label(RichText::new(value));
                                    }
                                    None => {
                                        ui.label(
                                            RichText::new("NULL").color(theme::TEXT_DIM).italics(),
                                        );
                                    }
                                });
                            }
                        }
                    }
                    if has_pk {
                        row.col(|ui| {
                            ui.horizontal(|ui| {
                                if ui.small_button("Edit").clicked() {
                                    row_action = Some((i, RecordAction::Edit));
                                }
                                let armed = self
                                    .delete_arm
                                    .as_ref()
                                    .map(|(k, r)| k == key && *r == i)
                                    .unwrap_or(false);
                                let label = if armed { "Delete?" } else { "Delete" };
                                if ui
                                    .add(
                                        egui::Button::new(label)
                                            .fill(if armed {
                                                theme::RED_DARK
                                            } else {
                                                crate::theme::CARD
                                            })
                                            .stroke(egui::Stroke::new(1.0, theme::BORDER_STRONG)),
                                    )
                                    .clicked()
                                {
                                    row_action = Some((i, RecordAction::Delete));
                                }
                            });
                        });
                    }
                    if row.response().double_clicked() && has_pk {
                        double_clicked = Some(i);
                    } else if row.response().clicked() {
                        clicked = Some(i);
                    }
                });
            });

        if let Some(i) = clicked {
            self.select_table_row(key, i);
        }
        if let Some(i) = double_clicked {
            self.open_record_panel(key, RecordPanelMode::Edit, Some(i));
        }
        if let Some((i, action)) = row_action {
            match action {
                RecordAction::Edit => self.open_record_panel(key, RecordPanelMode::Edit, Some(i)),
                RecordAction::Delete => {
                    let armed = self
                        .delete_arm
                        .as_ref()
                        .map(|(k, r)| k == key && *r == i)
                        .unwrap_or(false);
                    if armed {
                        self.delete_confirm(key, i);
                    } else {
                        self.delete_arm = Some(((*key).clone(), i));
                    }
                }
            }
        } else if ctx.input(|i| i.pointer.any_click()) {
            if let Some((k, _)) = &self.delete_arm {
                if k == key {
                    self.delete_arm = None;
                }
            }
        }
    }
}
