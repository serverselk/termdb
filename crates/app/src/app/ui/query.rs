//! Query-editor card, query history, results, and a slim log — plus the SQL
//! token-highlighting layouter and the read-only results grid.

use std::sync::Arc;

use egui::{text::Galley, FontId, RichText, TextBuffer};
use egui_extras::{Column as TableColumn, TableBuilder};

use super::{primary_button, section_header, RESULTS_GRID_HEIGHT};
use crate::app::TermdbApp;
use termdb::db::Request;

impl TermdbApp {
    /// Collapsible QUERY EDITOR card with a right-aligned ▶ RUN button.
    pub(crate) fn ui_query_card(&mut self, ui: &mut egui::Ui) {
        let mut open = self.query_open;
        let mut run = false;
        let run_enabled = {
            let live = self
                .selected_id
                .map(|id| self.live.contains_key(&id))
                .unwrap_or(false);
            live && !self.query_text.trim().is_empty()
        };
        section_header(ui, &mut open, "QUERY EDITOR", |ui| {
            if ui
                .add_enabled(run_enabled, primary_button("▶ RUN"))
                .clicked()
            {
                run = true;
            }
        });
        if open {
            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::multiline(&mut self.query_text)
                    .code_editor()
                    .hint_text("SELECT * FROM table_name;")
                    .desired_rows(5)
                    .layouter(&mut sql_layouter),
            );
            ui.add_space(2.0);
            ui.label(
                RichText::new("ad-hoc SQL runs on the selected connection (results below)")
                    .small()
                    .color(crate::theme::palette().text_dim),
            );
        }
        self.query_open = open;
        if run {
            self.run_query();
        }
    }

    /// Collapsible QUERY HISTORY card.
    pub(crate) fn ui_history_card(&mut self, ui: &mut egui::Ui) {
        let mut open = self.history_open;
        let mut clear = false;
        let mut clicked_sql: Option<String> = None;
        section_header(ui, &mut open, "QUERY HISTORY", |ui| {
            if !self.history.is_empty() && ui.small_button("Clear").clicked() {
                clear = true;
            }
        });
        if open {
            if self.history.is_empty() {
                ui.label(
                    RichText::new("No query history")
                        .color(crate::theme::palette().text_dim)
                        .italics(),
                );
            }
            for entry in &self.history {
                if ui
                    .selectable_label(false, crate::app::short(&entry.sql))
                    .clicked()
                {
                    clicked_sql = Some(entry.sql.clone());
                }
            }
        }
        self.history_open = open;
        if let Some(sql) = clicked_sql {
            self.query_text = sql;
            self.run_query();
        }
        if clear {
            self.backend.send(Request::ClearHistory {});
        }
    }

    /// Collapsible RESULTS card: last query output + export.
    pub(crate) fn ui_results_card(&mut self, ui: &mut egui::Ui) {
        let mut open = self.results_open;
        let mut export: Option<&'static str> = None;
        section_header(ui, &mut open, "RESULTS", |ui| {
            if let Some(state) = &self.query_state {
                ui.label(
                    RichText::new(format!(
                        "{} column(s) · {} row(s)",
                        state.columns.len(),
                        state.rows.len()
                    ))
                    .small()
                    .color(crate::theme::palette().text_dim),
                );
                if ui.small_button("JSON").clicked() {
                    export = Some("json");
                }
                if ui.small_button("CSV").clicked() {
                    export = Some("csv");
                }
            }
        });
        if open {
            if let Some(state) = &self.query_state {
                let columns = state.columns.clone();
                let rows = &state.rows;
                ui.add_space(4.0);
                ui_result_grid(ui, &columns, rows);
            } else {
                ui.label(
                    RichText::new("Execute a query to see results")
                        .color(crate::theme::palette().text_dim)
                        .italics(),
                );
            }
        }
        self.results_open = open;
        if let Some(format) = export {
            self.export_snapshot(format);
        }
    }
}

/// Read-only virtualized grid for ad-hoc query results.
fn ui_result_grid(ui: &mut egui::Ui, columns: &[String], rows: &[Vec<Option<String>>]) {
    let mut table = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .max_scroll_height(RESULTS_GRID_HEIGHT)
        .cell_layout(egui::Layout::left_to_right(egui::Align::LEFT))
        .column(TableColumn::auto());
    for _ in columns.iter().skip(1) {
        table = table.column(TableColumn::auto());
    }
    table
        .header(24.0, |mut header| {
            for name in columns {
                header.col(|ui| {
                    ui.strong(name);
                });
            }
        })
        .body(|body| {
            body.rows(20.0, rows.len(), |mut row| {
                let i = row.index();
                if let Some(cells) = rows.get(i) {
                    for cell in cells {
                        row.col(|ui| match cell {
                            Some(value) => {
                                ui.label(RichText::new(value));
                            }
                            None => {
                                ui.label(
                                    RichText::new("NULL")
                                        .color(crate::theme::palette().text_dim)
                                        .italics(),
                                );
                            }
                        });
                    }
                }
            });
        });
}

/// Extremely small SQL token-highlighter used as the editor layouter.
const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "AND",
    "OR",
    "NOT",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "LIMIT",
    "OFFSET",
    "ORDER",
    "BY",
    "GROUP",
    "HAVING",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "FULL",
    "CROSS",
    "AS",
    "ON",
    "NULL",
    "TRUE",
    "FALSE",
    "LIKE",
    "ILIKE",
    "IS",
    "IN",
    "EXISTS",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "CREATE",
    "TABLE",
    "DROP",
    "ALTER",
    "PRIMARY",
    "KEY",
    "REFERENCES",
    "DISTINCT",
];

fn sql_layouter(ui: &egui::Ui, buffer: &dyn TextBuffer, wrap_width: f32) -> Arc<Galley> {
    use egui::text::{LayoutJob, TextFormat};
    use egui::Color32;

    const KEYWORD: Color32 = Color32::from_rgb(0x7d, 0xc9, 0xff);
    const STRING: Color32 = Color32::from_rgb(0x8f, 0xcf, 0x9d);
    const NUMBER: Color32 = Color32::from_rgb(0xe0, 0xa9, 0x4b);
    const COMMENT: Color32 = Color32::from_rgb(0x52, 0x60, 0x70);
    const DEFAULT: Color32 = Color32::from_rgb(0xd7, 0xde, 0xe8);

    let font_id = FontId::monospace(13.0);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;

    let is_keyword = |w: &str| {
        SQL_KEYWORDS
            .iter()
            .any(|k| k.eq_ignore_ascii_case(w.trim()))
    };
    let is_number = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit() || c == '.');

    let chars: Vec<char> = buffer.as_str().chars().collect();
    let mut i = 0;
    let mut word = String::new();
    let mut push = |text: &str, color: Color32| {
        if text.is_empty() {
            return;
        }
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: font_id.clone(),
                color,
                ..Default::default()
            },
        );
    };
    macro_rules! flush_word {
        () => {
            if !word.is_empty() {
                let color = if is_keyword(&word) {
                    KEYWORD
                } else if is_number(&word) {
                    NUMBER
                } else {
                    DEFAULT
                };
                push(&word.clone(), color);
                word.clear();
            }
        };
    }

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                flush_word!();
                let mut lit = String::from("'");
                i += 1;
                while i < chars.len() {
                    if chars[i] == '\'' {
                        if i + 1 < chars.len() && chars[i + 1] == '\'' {
                            lit.push_str("''");
                            i += 2;
                            continue;
                        }
                        lit.push('\'');
                        i += 1;
                        break;
                    }
                    lit.push(chars[i]);
                    i += 1;
                }
                push(&lit, STRING);
            }
            '-' if i + 1 < chars.len() && chars[i + 1] == '-' => {
                flush_word!();
                let mut comment = String::new();
                while i < chars.len() && chars[i] != '\n' {
                    comment.push(chars[i]);
                    i += 1;
                }
                push(&comment, COMMENT);
            }
            c if c.is_whitespace() || matches!(c, '(' | ')' | ',' | ';') => {
                flush_word!();
                push(&c.to_string(), DEFAULT);
                i += 1;
            }
            _ => {
                word.push(c);
                i += 1;
            }
        }
    }
    flush_word!();

    ui.ctx().fonts_mut(|f| f.layout_job(job))
}
