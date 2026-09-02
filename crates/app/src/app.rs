//! Application shell: state, backend event handling, and the root layout.
//!
//! Rendering lives in `crate::app::ui::{header, sidebar, table, query}` — this
//! module owns the `TermdbApp` state and the pieces (filters, pagination, row
//! editor, queries) that mutate it.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use egui::RichText;
use termdb::db::engine::{Column, TableFilter};
use termdb::db::{Backend, Event, Request};
use termdb_core::{ConfigStore, ConnectionConfig, Engine, HistoryEntry, VaultKind};

pub mod ui;

use crate::app::ui::{header, sidebar, workspace};

/// Live per-connection state, mirrored from the backend via events.
struct LiveConnection {
    connecting: bool,
    databases: Vec<String>,
    tables: HashMap<String, Vec<String>>,
    loading_tables: HashSet<String>,
    selected_database: Option<String>,
    selected_table: Option<String>,
}

impl LiveConnection {
    fn placeholder() -> Self {
        Self {
            connecting: true,
            databases: Vec::new(),
            tables: HashMap::new(),
            loading_tables: HashSet::new(),
            selected_database: None,
            selected_table: None,
        }
    }
}

/// An opened table: describe + cached current page + filter state.
struct OpenTable {
    columns: Vec<Column>,
    rows: Vec<Vec<Option<String>>>,
    total: i64,
    page: i64,
    page_size: i64,
    loading: bool,
    filter: Option<TableFilter>,
    selected_row: Option<usize>,
}

impl OpenTable {
    fn placeholder(page_size: i64) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            total: 0,
            page: 0,
            page_size,
            loading: true,
            filter: None,
            selected_row: None,
        }
    }
}

/// Right-hand record panel: add/edit a single row.
struct RecordPanel {
    /// Table being edited and its target key.
    key: TableKey,
    mode: RecordPanelMode,
    /// The row being edited (edit mode); `None` for a new record.
    row: Option<usize>,
    /// Per-column, ordered like `open.columns`. Empty → NULL (when nullable)
    /// or omitted (when the column is required).
    fields: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RecordPanelMode {
    Add,
    Edit,
}

/// Result set of an ad-hoc query.
struct QueryState {
    conn_id: i64,
    columns: Vec<String>,
    rows: Vec<Vec<Option<String>>>,
}

enum RecordAction {
    Edit,
    Delete,
}

type TableKey = (i64, String, String);

/// How far along backend startup / last round-trip we are.
enum BackendStatus {
    Starting,
    Ready {
        version: String,
        vault_kind: VaultKind,
        last_pong: Option<PongResult>,
    },
}

struct PongResult {
    payload: String,
    latency_ms: u64,
}

/// Form state for the new-connection modal.
struct NewConnectionForm {
    name: String,
    engine: Engine,
    host: String,
    port: u16,
    username: String,
    password: String,
    database: String,
}

impl Default for NewConnectionForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            engine: Engine::Postgres,
            host: "localhost".to_owned(),
            port: Engine::Postgres.default_port(),
            username: "postgres".to_owned(),
            password: String::new(),
            database: String::new(),
        }
    }
}

impl NewConnectionForm {
    fn first_problem(&self) -> Option<String> {
        if self.name.trim().is_empty() {
            Some("name is required".into())
        } else if self.host.trim().is_empty() {
            Some("host is required".into())
        } else if self.username.trim().is_empty() {
            Some("username is required".into())
        } else {
            None
        }
    }
}

/// Single-column primary key usable for row editing.
fn single_pk(columns: &[Column]) -> Option<&str> {
    let keys: Vec<&str> = columns
        .iter()
        .filter(|c| c.key == "PRI")
        .map(|c| c.name.as_str())
        .collect();
    if keys.len() == 1 {
        Some(keys[0])
    } else {
        None
    }
}

/// Auto-increment-ish columns (MySQL `auto_increment`, PG serial `nextval`,
/// identity/generated). Disabled in the record panel; empty on add.
fn auto_increment_col(col: &Column) -> bool {
    let extra = col.extra.to_lowercase();
    extra.contains("auto_increment")
        || extra.contains("generated")
        || extra.contains("identity")
        || col
            .default
            .as_deref()
            .is_some_and(|d| d.contains("nextval"))
}

/// Boolean-ish columns get a checkbox.
fn is_bool_col(col: &Column) -> bool {
    col.ty.to_lowercase().contains("bool")
}

/// Text/blob/json-ish columns get a textarea; everything else a singleline.
fn is_complex_col(col: &Column) -> bool {
    const SIMPLE: &[&str] = &[
        "varchar",
        "char",
        "int",
        "bigint",
        "smallint",
        "tinyint",
        "decimal",
        "float",
        "double",
        "real",
        "bool",
        "boolean",
        "bit",
        "date",
        "time",
        "timestamp",
        "datetime",
        "enum",
        "serial",
        "uuid",
        "inet",
        "money",
    ];
    let t = col.ty.to_lowercase();
    !SIMPLE.iter().any(|s| t.contains(s))
}

/// `(pk_name, pk_value)` of a row, for UPDATE/DELETE.
fn record_pk(open: &OpenTable, row: usize) -> Option<(String, Option<String>)> {
    let name = single_pk(&open.columns)?.to_owned();
    let index = open.columns.iter().position(|c| c.name == name)?;
    let value = open
        .rows
        .get(row)
        .and_then(|cells| cells.get(index))
        .cloned()
        .flatten();
    Some((name, value))
}

pub struct TermdbApp {
    backend: Backend,
    started: Instant,
    connections: Vec<ConnectionConfig>,
    selected_id: Option<i64>,
    live: HashMap<i64, LiveConnection>,
    /// `(conn_id, database, table)` of the table currently open, if any.
    table_selection: Option<TableKey>,
    open_tables: HashMap<TableKey, OpenTable>,
    /// Favorite (starred) tables, cosmetic.
    favorites: HashSet<TableKey>,
    page_size: i64,
    /// Query editor + section collapse states.
    query_text: String,
    query_state: Option<QueryState>,
    query_open: bool,
    history_open: bool,
    results_open: bool,
    /// Overlay windows.
    show_new_connection: bool,
    show_settings: bool,
    show_logs: bool,
    /// Previous runs, newest first.
    history: Vec<HistoryEntry>,
    /// WHERE-builder state.
    filter_col: String,
    filter_op: String,
    filter_val: String,
    /// Whether the "+ Add Filter" builder row is visible.
    filter_open: bool,
    /// Add/Edit record side panel.
    record_panel: Option<RecordPanel>,
    /// Armed delete-confirmation: `(table, row)` deleted on the second click.
    delete_arm: Option<(TableKey, usize)>,
    form: NewConnectionForm,
    saving: bool,
    backend_status: BackendStatus,
    log: Vec<String>,
}

impl TermdbApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, backend: Backend) -> Self {
        let started = Instant::now();

        // One-time local read on the UI thread (our own tiny sqlite file).
        let mut connections = Vec::new();
        if let Ok(store) = ConfigStore::open(&ConfigStore::default_path()) {
            connections = store.list_connections().unwrap_or_default();
        }

        let mut log = Vec::new();
        log.push(format!(
            "loaded {} saved connection(s) from {}",
            connections.len(),
            ConfigStore::default_path().display()
        ));

        Self {
            backend,
            started,
            connections,
            selected_id: None,
            live: HashMap::new(),
            table_selection: None,
            open_tables: HashMap::new(),
            favorites: HashSet::new(),
            page_size: 50,
            query_text: String::new(),
            query_state: None,
            query_open: true,
            history_open: false,
            results_open: true,
            show_new_connection: false,
            show_settings: false,
            show_logs: false,
            history: Vec::new(),
            filter_col: String::new(),
            filter_op: "=".to_owned(),
            filter_val: String::new(),
            filter_open: false,
            record_panel: None,
            delete_arm: None,
            form: NewConnectionForm::default(),
            saving: false,
            backend_status: BackendStatus::Starting,
            log,
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::BackendReady {
                version,
                vault_kind,
            } => {
                self.backend_status = BackendStatus::Ready {
                    version: version.to_owned(),
                    vault_kind,
                    last_pong: None,
                };
                self.backend.send(Request::LoadHistory {});
                self.push_log(format!(
                    "backend ready v{version} · vault: {}",
                    vault_kind.label()
                ));
            }
            Event::Pong {
                payload,
                latency_ms,
            } => {
                if let BackendStatus::Ready { last_pong, .. } = &mut self.backend_status {
                    *last_pong = Some(PongResult {
                        payload: payload.clone(),
                        latency_ms,
                    });
                }
                self.push_log(format!("pong \"{payload}\" in {latency_ms}ms"));
            }
            Event::ConnectionSaved { cfg } => {
                self.saving = false;
                if let Some(existing) = self.connections.iter_mut().find(|c| c.name == cfg.name) {
                    *existing = cfg.clone();
                } else {
                    self.connections.push(cfg.clone());
                }
                self.selected_id = cfg.id;
                self.show_new_connection = false;
                self.push_log(format!("saved connection \"{}\"", cfg.name));
            }
            Event::Connected {
                conn_id,
                cfg,
                databases,
                ..
            } => {
                let name = cfg.name.clone();
                self.live.insert(
                    conn_id,
                    LiveConnection {
                        connecting: false,
                        databases,
                        tables: HashMap::new(),
                        loading_tables: HashSet::new(),
                        selected_database: None,
                        selected_table: None,
                    },
                );
                self.selected_id = Some(conn_id);
                self.push_log(format!("connected to \"{name}\""));
            }
            Event::ConnectFailed {
                conn_id,
                name,
                message,
            } => {
                self.live.remove(&conn_id);
                self.push_log(format!("connect \"{name}\" failed: {message}"));
            }
            Event::Disconnected { conn_id, name } => {
                self.live.remove(&conn_id);
                self.open_tables.retain(|(c, _, _), _| *c != conn_id);
                self.favorites.retain(|(c, _, _)| *c != conn_id);
                if let Some((c, _, _)) = &self.table_selection {
                    if *c == conn_id {
                        self.table_selection = None;
                    }
                }
                if let Some(query) = &self.query_state {
                    if query.conn_id == conn_id {
                        self.query_state = None;
                    }
                }
                if let Some(panel) = &self.record_panel {
                    if panel.key.0 == conn_id {
                        self.record_panel = None;
                    }
                }
                if let Some((key, _)) = &self.delete_arm {
                    if key.0 == conn_id {
                        self.delete_arm = None;
                    }
                }
                self.push_log(format!("disconnected \"{name}\""));
            }
            Event::TablesListed {
                conn_id,
                database,
                tables,
            } => {
                if let Some(live) = self.live.get_mut(&conn_id) {
                    live.loading_tables.remove(&database);
                    live.tables.insert(database.clone(), tables.clone());
                }
                self.push_log(format!("\"{database}\": {} table(s)", tables.len()));
            }
            Event::TablesFailed {
                conn_id,
                database,
                message,
            } => {
                if let Some(live) = self.live.get_mut(&conn_id) {
                    live.loading_tables.remove(&database);
                }
                self.push_log(format!("tables \"{database}\": {message}"));
            }
            Event::TableOpened {
                conn_id,
                database,
                table,
                columns,
                total,
                rows,
            } => {
                let key = (conn_id, database.clone(), table.clone());
                if let Some(open) = self.open_tables.get_mut(&key) {
                    open.columns = columns;
                    open.rows = rows;
                    open.total = total;
                    open.page = 0;
                    open.loading = false;
                }
                self.push_log(format!(
                    "opened \"{database}.{table}\" — {} column(s), {total} row(s)",
                    self.open_tables
                        .get(&key)
                        .map(|o| o.columns.len())
                        .unwrap_or(0)
                ));
            }
            Event::TableOpenFailed {
                conn_id,
                database,
                table,
                message,
            } => {
                self.open_tables.remove(&(conn_id, database, table));
                self.push_log(format!("open table failed: {message}"));
            }
            Event::PageLoaded {
                conn_id,
                database,
                table,
                total,
                rows,
            } => {
                if let Some(open) = self.open_tables.get_mut(&(conn_id, database, table)) {
                    open.rows = rows;
                    open.total = total;
                    open.loading = false;
                }
            }
            Event::PageFailed {
                conn_id,
                database,
                table,
                message,
            } => {
                if let Some(open) = self.open_tables.get_mut(&(conn_id, database, table)) {
                    open.loading = false;
                }
                self.push_log(format!("page failed: {message}"));
            }
            Event::QueryResults {
                conn_id,
                columns,
                rows,
            } => {
                let n = rows.len();
                self.query_state = Some(QueryState {
                    conn_id,
                    columns,
                    rows: rows.clone(),
                });
                self.results_open = true;
                let name = self
                    .connections
                    .iter()
                    .find(|c| c.id == Some(conn_id))
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| format!("#{conn_id}"));
                self.push_log(format!("query on \"{name}\": {n} row(s)"));
            }
            Event::QueryFailed { sql, message, .. } => {
                self.push_log(format!(
                    "query \"{}\" failed: {message}",
                    short(sql.as_str())
                ));
            }
            Event::HistoryListed { entries } => {
                self.history = entries;
            }
            Event::HistoryCleared {} => {
                self.history.clear();
                self.push_log("history cleared".into());
            }
            Event::RowChanged { conn_id } => {
                self.delete_arm = None;
                self.record_panel = None;
                if let Some((c, _, _)) = &self.table_selection {
                    if *c == conn_id {
                        let key = self.table_selection.clone().unwrap();
                        let page = self.open_tables.get(&key).map(|o| o.page).unwrap_or(0);
                        if let Some(open) = self.open_tables.get_mut(&key) {
                            open.selected_row = None;
                        }
                        self.goto_page(&key, page);
                    }
                }
                self.push_log("row changed".into());
            }
            Event::MutationFailed { message } => {
                self.push_log(format!("edit failed: {message}"));
            }
            Event::StoreFailed { message } => {
                self.saving = false;
                self.push_log(format!("error: {message}"));
            }
        }
    }

    fn push_log(&mut self, line: String) {
        self.log.push(format!(
            "[{:>5.1}s] {line}",
            self.started.elapsed().as_secs_f64()
        ));
    }

    fn drain_events(&mut self) {
        while let Some(event) = self.backend.poll() {
            self.handle_event(event);
        }
    }

    fn save_form(&mut self) {
        let cfg = ConnectionConfig {
            id: None,
            name: self.form.name.trim().to_owned(),
            engine: self.form.engine,
            host: self.form.host.trim().to_owned(),
            port: self.form.port,
            username: self.form.username.trim().to_owned(),
            database: match self.form.database.trim() {
                "" => None,
                db => Some(db.to_owned()),
            },
            ssl: false,
        };
        let password = std::mem::take(&mut self.form.password);
        self.backend.send(Request::SaveConnection { cfg, password });
        self.saving = true;
        self.push_log(format!("saving connection \"{}\"…", self.form.name.trim()));
    }

    /// Connect to `conn_id`, inserting a placeholder while the pool opens.
    fn connect_to(&mut self, conn_id: i64) {
        let Some(cfg) = self.connections.iter().find(|c| c.id == Some(conn_id)) else {
            return;
        };
        self.live.insert(conn_id, LiveConnection::placeholder());
        self.selected_id = Some(conn_id);
        self.backend.send(Request::Connect { conn_id });
        self.push_log(format!("connecting to \"{}\"…", cfg.name));
    }

    fn run_query(&mut self) {
        let sql = self.query_text.trim().to_owned();
        if sql.is_empty() {
            return;
        }
        let Some(conn_id) = self.selected_id else {
            self.push_log("select a connection first".into());
            return;
        };
        if !self.live.contains_key(&conn_id) {
            self.push_log(format!("connection #{conn_id} is not connected"));
            return;
        }
        self.push_log(format!("running: {}", short(&sql)));
        self.backend.send(Request::RunQuery { conn_id, sql });
    }

    fn export_snapshot(&mut self, format: &str) {
        if let Some(state) = self.query_state.as_ref() {
            let columns = state.columns.clone();
            let rows = state.rows.clone();
            self.export(&columns, &rows, format);
        }
    }

    fn export(&mut self, columns: &[String], rows: &[Vec<Option<String>>], format: &str) {
        let body = match format {
            "csv" => termdb::export::to_csv(columns, rows),
            "json" => termdb::export::to_json(columns, rows),
            _ => return,
        };
        let extension = format;
        let mut dialog = rfd::FileDialog::new().set_file_name(format!("export.{extension}"));
        if format == "csv" {
            dialog = dialog.add_filter("CSV", &["csv"]);
        } else {
            dialog = dialog.add_filter("JSON", &["json"]);
        }
        if let Some(path) = dialog.save_file() {
            match std::fs::write(&path, body) {
                Ok(_) => self.push_log(format!(
                    "exported {} bytes to {}",
                    path.display(),
                    std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
                )),
                Err(e) => self.push_log(format!("export failed: {e}")),
            }
        }
    }

    /// Open the add/edit record side panel for a table (row = `None` for new).
    fn open_record_panel(&mut self, key: &TableKey, mode: RecordPanelMode, row: Option<usize>) {
        let Some(open) = self.open_tables.get(key) else {
            return;
        };
        let columns = open.columns.clone();
        let fields = columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let text = match (mode, row) {
                    (RecordPanelMode::Edit, Some(r)) => open
                        .rows
                        .get(r)
                        .and_then(|cells| cells.get(i))
                        .and_then(Clone::clone)
                        .unwrap_or_default(),
                    _ => col
                        .default
                        .as_deref()
                        .and_then(termdb::record::clean_default)
                        .unwrap_or_default(),
                };
                if mode == RecordPanelMode::Add && auto_increment_col(col) {
                    String::new()
                } else {
                    text
                }
            })
            .collect();
        self.record_panel = Some(RecordPanel {
            key: key.clone(),
            mode,
            row,
            fields,
        });
    }

    /// The right-side record panel window (add/edit a row).
    fn ui_record_panel(&mut self, ctx: &egui::Context) {
        let Some(mut panel) = self.record_panel.take() else {
            return;
        };
        let columns = self
            .open_tables
            .get(&panel.key)
            .map(|o| o.columns.clone())
            .unwrap_or_default();
        let title = match panel.mode {
            RecordPanelMode::Add => "Add New Record",
            RecordPanelMode::Edit => "Edit Record",
        };
        let mut fields = std::mem::take(&mut panel.fields);
        let mut save = false;
        let mut cancel = false;
        let mut open_flag = true;

        egui::Window::new(title)
            .open(&mut open_flag)
            .anchor(egui::Align2::RIGHT_CENTER, [-8.0, 0.0])
            .default_width(420.0)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("record_panel_fields")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            for (i, col) in columns.iter().enumerate() {
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(&col.name);
                                        if col.key == "PRI" {
                                            ui.label(
                                                RichText::new("PK")
                                                    .small()
                                                    .color(crate::theme::AMBER),
                                            );
                                        }
                                        if auto_increment_col(col) {
                                            ui.label(
                                                RichText::new("AI")
                                                    .small()
                                                    .color(crate::theme::GREEN),
                                            );
                                        }
                                        if !col.nullable {
                                            ui.label(
                                                RichText::new("*").small().color(crate::theme::RED),
                                            );
                                        }
                                    });
                                });
                                let disabled = auto_increment_col(col);
                                let Some(field) = fields.get_mut(i) else {
                                    ui.end_row();
                                    continue;
                                };
                                if is_bool_col(col) {
                                    let mut value = field != "false" && !field.is_empty();
                                    let resp = ui.add_enabled(
                                        !disabled,
                                        egui::Checkbox::without_text(&mut value),
                                    );
                                    if resp.changed() {
                                        *field = value.to_string();
                                    }
                                } else if is_complex_col(col) {
                                    ui.add_enabled(
                                        !disabled,
                                        egui::TextEdit::multiline(field).desired_rows(4),
                                    );
                                } else {
                                    ui.add_enabled(
                                        !disabled,
                                        egui::TextEdit::singleline(field).desired_width(240.0),
                                    );
                                }
                                ui.end_row();
                            }
                        });
                });
                ui.add_space(6.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(ui::primary_button("Save")).clicked() {
                            save = true;
                        }
                        if ui.add(ui::outline_button("Cancel")).clicked() {
                            cancel = true;
                        }
                    });
                });
            });

        if cancel {
            open_flag = false;
        }
        if open_flag {
            panel.fields = fields;
            if save {
                self.submit_record_panel(&panel);
            }
            self.record_panel = Some(panel);
        }
    }

    /// Build the parametrized INSERT/UPDATE for the panel's fields.
    fn submit_record_panel(&mut self, panel: &RecordPanel) {
        let (conn_id, database, table) = panel.key.clone();
        let Some(open) = self.open_tables.get(&panel.key) else {
            return;
        };
        let columns = open.columns.clone();
        let values: Vec<(String, Option<String>)> = columns
            .iter()
            .enumerate()
            .filter_map(|(i, col)| {
                let text = panel.fields.get(i).map(String::as_str).unwrap_or("");
                if text.is_empty() {
                    if col.nullable {
                        return Some((col.name.clone(), None));
                    }
                    return None; // omit required-but-empty fields
                }
                Some((col.name.clone(), Some(text.to_owned())))
            })
            .collect();
        let request = match panel.mode {
            RecordPanelMode::Add => Some(Request::InsertRow {
                conn_id,
                database,
                table,
                columns,
                values,
            }),
            RecordPanelMode::Edit => {
                panel
                    .row
                    .and_then(|row| record_pk(open, row))
                    .map(|pk| Request::UpdateRow {
                        conn_id,
                        database,
                        table,
                        columns,
                        values,
                        pk,
                    })
            }
        };
        if let Some(request) = request {
            self.backend.send(request);
        }
    }

    fn delete_confirm(&mut self, key: &TableKey, row: usize) {
        self.delete_arm = None;
        let (conn_id, database, table) = key.clone();
        let Some(open) = self.open_tables.get(key) else {
            return;
        };
        let columns = open.columns.clone();
        if let Some(pk) = record_pk(open, row) {
            self.backend.send(Request::DeleteRow {
                conn_id,
                database,
                table,
                columns,
                pk,
            });
        }
    }

    fn set_filter(&mut self, key: &TableKey, filter: Option<TableFilter>) {
        let (conn_id, database, table) = key.clone();
        let page_size = match self.open_tables.get_mut(key) {
            Some(open) => {
                open.filter = filter.clone();
                open.loading = true;
                open.page = 0;
                open.selected_row = None;
                open.page_size
            }
            None => self.page_size,
        };
        self.backend.send(Request::OpenTable {
            conn_id,
            database,
            table,
            filter,
            limit: page_size,
        });
    }

    fn select_table_row(&mut self, key: &TableKey, index: usize) {
        let Some(open) = self.open_tables.get_mut(key) else {
            return;
        };
        open.selected_row = Some(index);
    }

    fn goto_page(&mut self, key: &TableKey, page: i64) {
        let (conn_id, database, table) = key.clone();
        let Some(open) = self.open_tables.get_mut(key) else {
            return;
        };
        let columns = open.columns.clone();
        let page_size = open.page_size;
        let filter = open.filter.clone();
        let page = page.max(0);
        open.page = page;
        open.loading = true;
        self.backend.send(Request::LoadPage {
            conn_id,
            database,
            table,
            columns,
            filter,
            limit: page_size,
            offset: page * page_size,
        });
    }
}

fn short(sql: &str) -> String {
    let one_line = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 48 {
        let trimmed: String = one_line.chars().take(45).collect();
        format!("{trimmed}…")
    } else {
        one_line
    }
}

fn total_pages(total: i64, page_size: i64) -> i64 {
    if total <= 0 {
        1
    } else {
        (total + page_size - 1) / page_size
    }
}

impl eframe::App for TermdbApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Always wake up promptly so late-arriving async events render even
        // without user input.
        ctx.request_repaint_after(Duration::from_millis(250));

        self.drain_events();

        // Top header bar, fixed-width sidebar, then the workspace cards.
        ui::header::ui_header_bar(self, ui);
        ui::sidebar::ui_sidebar(self, ui);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(crate::theme::BG)
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| {
                workspace(self, ui);
            });

        // Floating overlays (drawn last so they sit above everything).
        header::ui_settings_window(self, &ctx);
        header::ui_logs_window(self, &ctx);
        sidebar::ui_new_connection_window(self, &ctx);
        self.ui_record_panel(&ctx);
    }
}
