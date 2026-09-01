//! egui application shell: connections, database/table sidebar, virtualized
//! grid, query editor with history, and row editing (M4).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{text::Galley, FontId, RichText, TextBuffer};
use egui_extras::{Column as TableColumn, TableBuilder};
use termdb::db::engine::{Column, TableFilter, FILTER_OPS};
use termdb::db::{Backend, Event, Request};
use termdb_core::{ConfigStore, ConnectionConfig, Engine, HistoryEntry, VaultKind};

/// Live per-connection state, mirrored from the backend via events.
struct LiveConnection {
    connecting: bool,
    server_version: String,
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
            server_version: String::new(),
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

/// Right-hand record panel, mirroring tup-db-client's add/edit side panel.
struct RecordPanel {
    /// Table being edited and its target key.
    key: TableKey,
    mode: RecordPanelMode,
    /// The row being edited (edit mode); `None` for a new record.
    row: Option<usize>,
    /// Per-column, ordered like `open.columns`. Empty → NULL (when nullable)
    /// or omitted (when the column is required), mirrors the Electron app.
    fields: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RecordPanelMode {
    Add,
    Edit,
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

const BADGE_PK: egui::Color32 = egui::Color32::from_rgb(0xf5, 0x9e, 0x0b);
const BADGE_AI: egui::Color32 = egui::Color32::from_rgb(0x10, 0xb9, 0x81);
const BADGE_REQUIRED: egui::Color32 = egui::Color32::from_rgb(0xf4, 0x43, 0x36);

/// Colored chip like tup-db-client's PK / AI badges.
fn badge(text: &str, color: egui::Color32) -> RichText {
    RichText::new(text)
        .small()
        .color(color)
        .background_color(egui::Color32::from_black_alpha(120))
}

/// Auto-increment-ish columns (MySQL `auto_increment`, PG serial `nextval`
/// and identity/generated columns). Disabled in the record panel; empty on add.
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

/// Result set of an ad-hoc query.
struct QueryState {
    conn_id: i64,
    columns: Vec<String>,
    rows: Vec<Vec<Option<String>>>,
}

/// Which view owns the central panel.
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Table,
    Query,
}

enum RecordAction {
    Edit,
    Delete,
}

type TableKey = (i64, String, String);

/// Fixed height of the bottom log strip; the table viewport stays above it so
/// its borders never cross into the log.
const LOG_HEIGHT: f32 = 150.0;

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

/// Form state for the "New connection" section.
pub struct NewConnectionForm {
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

pub struct TermdbApp {
    backend: Backend,
    started: Instant,
    connections: Vec<ConnectionConfig>,
    selected_id: Option<i64>,
    live: HashMap<i64, LiveConnection>,
    /// The most recently clicked table: `(conn_id, database, table)`.
    table_selection: Option<TableKey>,
    open_tables: HashMap<TableKey, OpenTable>,
    page_size: i64,
    view: View,
    /// Query editor buffer.
    query_text: String,
    /// Result of the last ad-hoc query, if any.
    query_state: Option<QueryState>,
    /// Previous runs, newest first.
    history: Vec<HistoryEntry>,
    /// WHERE-builder state (filter row).
    filter_col: String,
    filter_op: String,
    filter_val: String,
    /// Add/Edit side panel (tup-db-client style).
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

        // One-time local read on the UI thread; it's our own tiny sqlite file
        // (microseconds), not a network database.
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
            page_size: 50,
            view: View::Table,
            query_text: String::new(),
            query_state: None,
            history: Vec::new(),
            filter_col: String::new(),
            filter_op: "=".to_owned(),
            filter_val: String::new(),
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
                // Upsert — a name may collide during save before error is raised.
                if let Some(existing) = self.connections.iter_mut().find(|c| c.name == cfg.name) {
                    *existing = cfg.clone();
                } else {
                    self.connections.push(cfg.clone());
                }
                self.selected_id = cfg.id;
                self.push_log(format!("saved connection \"{}\"", cfg.name));
            }
            Event::Connected {
                conn_id,
                cfg,
                server_version,
                databases,
            } => {
                let name = cfg.name.clone();
                self.live.insert(
                    conn_id,
                    LiveConnection {
                        connecting: false,
                        server_version,
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
                if let Some((c, _, _)) = &self.table_selection {
                    if *c == conn_id {
                        self.table_selection = None;
                    }
                }
                if let Some(query) = &self.query_state {
                    if query.conn_id == conn_id {
                        self.query_state = None;
                        self.view = View::Table;
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
                self.view = View::Query;
                let n = rows.len();
                self.query_state = Some(QueryState {
                    conn_id,
                    columns,
                    rows: rows.clone(),
                });
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
                // Reload whatever table the connection is showing.
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

    fn ui_status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let label = match &self.backend_status {
                BackendStatus::Starting => "backend: connecting…".to_owned(),
                BackendStatus::Ready {
                    version,
                    vault_kind,
                    last_pong,
                } => {
                    let live = self.live.len();
                    let mut s = format!(
                        "backend: ready v{version} · {} · {live} live",
                        vault_kind.label()
                    );
                    if let Some(pong) = last_pong {
                        s.push_str(&format!(
                            " · last ping \"{}\": {}ms",
                            pong.payload, pong.latency_ms
                        ));
                    }
                    s
                }
            };
            ui.label(label);

            let ready = matches!(&self.backend_status, BackendStatus::Ready { .. });
            if ui
                .add_enabled(ready, egui::Button::new("Ping backend"))
                .clicked()
            {
                self.backend.send(Request::Ping {
                    payload: "hello".into(),
                });
                self.push_log("ping sent".into());
            }
        });
    }

    fn ui_connections(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connections");
        if self.connections.is_empty() {
            ui.label(RichText::new("no saved connections yet").weak());
        }

        let connection_ids: Vec<i64> = self.connections.iter().filter_map(|c| c.id).collect();

        for id in connection_ids {
            let Some(cfg) = self.connections.iter().find(|c| c.id == Some(id)) else {
                continue;
            };
            let label = format!("{} · {}:{}", cfg.name, cfg.engine.label(), cfg.port);

            let mut connect = false;
            let mut disconnect = false;
            ui.horizontal(|ui| {
                let selected = self.selected_id == Some(id);
                if ui.selectable_label(selected, label).clicked() {
                    self.selected_id = Some(id);
                }
                match self.live.get(&id) {
                    Some(live) if live.connecting => {
                        ui.label(RichText::new("connecting…").weak().small());
                    }
                    Some(_) => {
                        if ui.small_button("Disconnect").clicked() {
                            disconnect = true;
                        }
                    }
                    None => {
                        if ui.small_button("Connect").clicked() {
                            connect = true;
                        }
                    }
                }
            });

            if connect {
                self.live.insert(id, LiveConnection::placeholder());
                self.backend.send(Request::Connect { conn_id: id });
            }
            if disconnect {
                self.backend.send(Request::Disconnect { conn_id: id });
            }

            self.ui_connection_tree(ui, id);
        }
    }

    /// Databases (collapsing) → tables underneath a live connection.
    fn ui_connection_tree(&mut self, ui: &mut egui::Ui, conn_id: i64) {
        let Some(live) = self.live.get(&conn_id) else {
            return;
        };
        if live.connecting || live.databases.is_empty() {
            return;
        }
        let databases = live.databases.clone();

        let live = self.live.get_mut(&conn_id).expect("connection still live");
        for db in databases {
            let tables = live.tables.get(&db).cloned();
            let loading = live.loading_tables.contains(&db);

            let resp = egui::CollapsingHeader::new(RichText::new(&db).strong())
                .id_salt(("db", conn_id, &db))
                .show(ui, |ui| {
                    if let Some(ts) = &tables {
                        for table in ts {
                            let active = live.selected_database.as_deref() == Some(db.as_str())
                                && live.selected_table.as_deref() == Some(table.as_str());
                            if ui.selectable_label(active, table).clicked() {
                                live.selected_database = Some(db.clone());
                                live.selected_table = Some(table.clone());
                                self.table_selection = Some((conn_id, db.clone(), table.clone()));
                                self.view = View::Table;
                            }
                        }
                    } else if loading {
                        ui.label("loading…");
                    } else {
                        ui.label(RichText::new("click to load tables").weak());
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

    fn ui_new_connection(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("New connection")
            .default_open(self.connections.is_empty())
            .show(ui, |ui| {
                egui::Grid::new("conn_form")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut self.form.name);
                        ui.end_row();

                        ui.label("Engine");
                        let prev = self.form.engine;
                        egui::ComboBox::from_id_salt("conn_engine")
                            .selected_text(self.form.engine.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.form.engine,
                                    Engine::Postgres,
                                    "PostgreSQL",
                                );
                                ui.selectable_value(&mut self.form.engine, Engine::Mysql, "MySQL");
                            });
                        if self.form.engine != prev {
                            self.form.port = self.form.engine.default_port();
                        }
                        ui.end_row();

                        ui.label("Host");
                        ui.text_edit_singleline(&mut self.form.host);
                        ui.end_row();

                        ui.label("Port");
                        ui.add(egui::DragValue::new(&mut self.form.port).range(1..=65535));
                        ui.end_row();

                        ui.label("Username");
                        ui.text_edit_singleline(&mut self.form.username);
                        ui.end_row();

                        ui.label("Password");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.password)
                                .password(true)
                                .hint_text("stored in the vault"),
                        );
                        ui.end_row();

                        ui.label("Database");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.database)
                                .hint_text("optional initial database"),
                        );
                        ui.end_row();
                    });

                if let Some(problem) = self.form.first_problem() {
                    ui.label(
                        RichText::new(problem).color(egui::Color32::from_rgb(0xff, 0x53, 0x70)),
                    );
                }
                if ui
                    .add_enabled(!self.saving, egui::Button::new("Save connection"))
                    .clicked()
                    && self.form.first_problem().is_none()
                {
                    self.save_form();
                }
            });
    }

    fn ui_central(&mut self, ui: &mut egui::Ui) {
        if let Some(key) = self.table_selection.clone() {
            self.ui_open_table(ui, key);
        } else {
            self.ui_connection_details(ui);
        }
    }

    /// Query editor + results workspace. Lives in the central panel when the
    /// last action was running a query (clicking a table returns to it).
    fn ui_query_workspace(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("SQL");
            ui.separator();
            let live = self.selected_id.map(|id| self.live.contains_key(&id));
            if ui
                .add_enabled(
                    live == Some(true) && !self.query_text.trim().is_empty(),
                    egui::Button::new("Run ▸"),
                )
                .clicked()
            {
                self.run_query();
            }
            if let Some(state) = &self.query_state {
                ui.label(format!(
                    "{} column(s) · {} row(s)",
                    state.columns.len(),
                    state.rows.len()
                ));
                ui.separator();
                if ui.small_button("Export CSV").clicked() {
                    self.export_snapshot("csv");
                }
                if ui.small_button("Export JSON").clicked() {
                    self.export_snapshot("json");
                }
            }
        });

        ui.add(
            egui::TextEdit::multiline(&mut self.query_text)
                .code_editor()
                .hint_text("SELECT * FROM customers;\n-- run with the button above")
                .desired_rows(5)
                .layouter(&mut sql_layouter),
        );

        ui.separator();
        if let Some(state) = &self.query_state {
            let columns: Vec<String> = state.columns.clone();
            let rows = &state.rows;
            ui_result_grid(ui, &columns, rows);
        } else if self.query_text.trim().is_empty() {
            ui.label(RichText::new("pick a connected server and run a query").weak());
        } else {
            ui.label(RichText::new("ready — press Run ▸").weak());
        }
    }

    fn export_snapshot(&mut self, format: &str) {
        if let Some(state) = self.query_state.as_ref() {
            let columns = state.columns.clone();
            let rows = state.rows.clone();
            self.export(&columns, &rows, format);
        }
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

    fn ui_connection_details(&mut self, ui: &mut egui::Ui) {
        ui.heading("Details");
        if let Some(id) = self.selected_id {
            if let Some(cfg) = self.connections.iter().find(|c| c.id == Some(id)) {
                egui::Grid::new("conn_details")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.label(&cfg.name);
                        ui.end_row();
                        ui.label("Engine");
                        ui.label(cfg.engine.label());
                        ui.end_row();
                        ui.label("Host");
                        ui.label(format!("{}:{}", cfg.host, cfg.port));
                        ui.end_row();
                        ui.label("Username");
                        ui.label(&cfg.username);
                        ui.end_row();
                        ui.label("Database");
                        ui.label(cfg.database.as_deref().unwrap_or("(default)"));
                        ui.end_row();
                        ui.label("SSL");
                        ui.label(if cfg.ssl { "enabled" } else { "disabled" });
                        ui.end_row();
                    });
                ui.add_space(8.0);

                if let Some(live) = self.live.get(&id) {
                    ui.label(
                        RichText::new(format!("● connected — {}", live.server_version))
                            .color(egui::Color32::from_rgb(0x7b, 0xc9, 0x6f)),
                    );
                    ui.label(format!("{} database(s)", live.databases.len()));
                } else {
                    ui.label(
                        RichText::new("connect from the sidebar to browse databases and tables")
                            .weak()
                            .italics(),
                    );
                }
            }
        } else {
            ui.label(RichText::new("select a connection, or add one").weak());
        }
    }

    /// Central panel for a selected table: toolbar + filter + grid + row editor.
    fn ui_open_table(&mut self, ui: &mut egui::Ui, key: TableKey) {
        let (conn_id, database, table) = key.clone();
        if !self.live.contains_key(&conn_id) {
            self.table_selection = None;
            return;
        }

        // Lazy open: first time this table is shown, ask the backend.
        if !self.open_tables.contains_key(&key) {
            self.open_tables
                .insert(key.clone(), OpenTable::placeholder(self.page_size));
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
        let page_size = self
            .open_tables
            .get(&key)
            .map(|o| o.page_size)
            .unwrap_or(self.page_size);
        let pages = total_pages(total, page_size);
        let on_last = page + 1 >= pages;
        let prev_size = self.page_size;

        ui.horizontal(|ui| {
            ui.heading(format!("{database} › {table}"));
            ui.separator();

            if loaded && ui.button("+ Add").clicked() {
                self.open_record_panel(&key, RecordPanelMode::Add, None);
            }

            egui::ComboBox::from_id_salt("page_size")
                .selected_text(format!("{} / page", self.page_size))
                .show_ui(ui, |ui| {
                    for size in [25i64, 50, 100, 200] {
                        ui.selectable_value(&mut self.page_size, size, format!("{size} / page"));
                    }
                });

            ui.separator();
            let prev = ui.add_enabled(loaded && page > 0 && !loading, egui::Button::new("‹ Prev"));
            if prev.clicked() {
                self.goto_page(&key, page - 1);
            }
            if loaded {
                ui.label(format!("page {} / {}", page + 1, pages));
            }
            let next = ui.add_enabled(loaded && !on_last && !loading, egui::Button::new("Next ›"));
            if next.clicked() {
                self.goto_page(&key, page + 1);
            }
            ui.label(format!("{total} rows"));
            if loading {
                ui.label(RichText::new("…").weak());
            }
        });

        // Page-size change reloads from page 0.
        if self.page_size != prev_size && loaded {
            if let Some(open) = self.open_tables.get_mut(&key) {
                open.page_size = self.page_size;
            }
            self.goto_page(&key, 0);
        }

        if !loaded {
            ui.label(RichText::new("loading table…").weak());
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

        // WHERE-builder row.
        let col_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
        if !col_names.is_empty() && !col_names.contains(&self.filter_col) {
            self.filter_col = col_names[0].clone();
        }
        ui.horizontal(|ui| {
            ui.label("Filter:");
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
                    .desired_width(160.0),
            );
            if ui
                .add_enabled(!self.filter_val.is_empty(), egui::Button::new("Apply"))
                .clicked()
            {
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
            if let Some(f) = &active_filter {
                ui.label(
                    RichText::new(format!("{} {op} \"{}\"", f.column, f.value, op = f.op))
                        .weak()
                        .italics(),
                );
            }
        });

        // Grid with row selection, per-row actions and double-click-to-edit.
        // (tup-db-client parity: actions column only when we can edit/delete,
        // i.e. the table has a single-column primary key.)
        let has_pk = single_pk(&columns).is_some();
        let mut clicked: Option<usize> = None;
        let mut double_clicked: Option<usize> = None;
        let mut row_action: Option<(usize, RecordAction)> = None;
        let ctx = ui.ctx().clone();
        let table_height = (ui.available_height() - LOG_HEIGHT - 30.0).max(120.0);
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .max_scroll_height(table_height)
            .cell_layout(egui::Layout::left_to_right(egui::Align::LEFT))
            .column(TableColumn::auto());
        for _ in columns.iter().skip(1) {
            table = table.column(TableColumn::auto());
        }
        if has_pk {
            table = table.column(TableColumn::exact(130.0));
        }
        table
            .header(38.0, |mut header| {
                for col in &columns {
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
                            ui.label(RichText::new(&col.ty).small().weak());
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
                        .get(&key)
                        .map(|o| o.selected_row == Some(i))
                        .unwrap_or(false);
                    row.set_selected(selected);
                    if let Some(open) = self.open_tables.get(&key) {
                        if let Some(cells) = open.rows.get(i) {
                            for cell in cells {
                                row.col(|ui| match cell {
                                    Some(value) => {
                                        ui.label(RichText::new(value));
                                    }
                                    None => {
                                        ui.label(RichText::new("NULL").weak().italics());
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
                                    .map(|(k, r)| k == &key && *r == i)
                                    .unwrap_or(false);
                                let label = if armed { "Delete?" } else { "Delete" };
                                if ui.small_button(label).clicked() {
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
            self.select_table_row(&key, i);
            self.view = View::Table;
        }
        if let Some(i) = double_clicked {
            self.open_record_panel(&key, RecordPanelMode::Edit, Some(i));
        }
        if let Some((i, action)) = row_action {
            match action {
                RecordAction::Edit => {
                    self.open_record_panel(&key, RecordPanelMode::Edit, Some(i));
                }
                RecordAction::Delete => {
                    let armed = self
                        .delete_arm
                        .as_ref()
                        .map(|(k, r)| k == &key && *r == i)
                        .unwrap_or(false);
                    if armed {
                        self.delete_confirm(&key, i);
                    } else {
                        self.delete_arm = Some((key.clone(), i));
                    }
                }
            }
        } else if ctx.input(|i| i.pointer.any_click()) {
            // Clicking anywhere else disarms the two-step delete.
            if let Some((k, _)) = &self.delete_arm {
                if k == &key {
                    self.delete_arm = None;
                }
            }
        }
    }

    /// Open the add/edit side panel for a table (row = `None` for a new one).
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

    /// The sliding record panel, tup-db-client style.
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
                                            ui.label(badge("PK", BADGE_PK));
                                        }
                                        if auto_increment_col(col) {
                                            ui.label(badge("AI", BADGE_AI));
                                        }
                                        if !col.nullable {
                                            ui.label(RichText::new("*").color(BADGE_REQUIRED));
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
                        if ui.button("Save").clicked() {
                            save = true;
                        }
                        if ui.button("Cancel").clicked() {
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

    fn ui_history(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("History")
            .default_open(false)
            .show(ui, |ui| {
                if self.history.is_empty() {
                    ui.label(RichText::new("no queries yet").weak());
                }
                let mut clicked_sql: Option<String> = None;
                for entry in &self.history {
                    if ui
                        .selectable_label(false, short(&entry.sql))
                        .on_hover_text(&entry.sql)
                        .clicked()
                    {
                        clicked_sql = Some(entry.sql.clone());
                    }
                }
                if let Some(sql) = clicked_sql {
                    self.query_text = sql.clone();
                    self.view = View::Query;
                    self.run_query();
                }
                if !self.history.is_empty() && ui.small_button("Clear").clicked() {
                    self.backend.send(Request::ClearHistory {});
                }
            });
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

    fn ui_log(&mut self, ui: &mut egui::Ui) {
        // The log is a bordered, opaque terminal pane. It is painted last, so
        // its fill covers any stray lines from widgets above; the top border
        // separates it from the content (the separator is inside this frame).
        let fill = ui.visuals().panel_fill;
        let border = ui.visuals().window_stroke.color;
        let _ = egui::Frame::default()
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, border))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Log");
                    if ui.small_button("Clear").clicked() {
                        self.log.clear();
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(LOG_HEIGHT)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.log {
                            ui.monospace(line);
                        }
                    });
            });
    }
}

fn total_pages(total: i64, page_size: i64) -> i64 {
    if total <= 0 {
        1
    } else {
        (total + page_size - 1) / page_size
    }
}

/// Truncate a query for one-line log/history display.
fn short(sql: &str) -> String {
    let one_line = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 48 {
        let trimmed: String = one_line.chars().take(45).collect();
        format!("{trimmed}…")
    } else {
        one_line
    }
}

/// Read-only virtualized grid for ad-hoc query results (headers are plain
/// strings; no row selection).
fn ui_result_grid(ui: &mut egui::Ui, columns: &[String], rows: &[Vec<Option<String>>]) {
    let table_height = (ui.available_height() - LOG_HEIGHT - 30.0).max(120.0);
    let mut table = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .max_scroll_height(table_height)
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
                                ui.label(RichText::new("NULL").weak().italics());
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
    const STRING: Color32 = Color32::from_rgb(0x6f, 0xa8, 0x5f);
    const NUMBER: Color32 = Color32::from_rgb(0xd1, 0x9a, 0x66);
    const COMMENT: Color32 = Color32::from_rgb(0x6b, 0x74, 0x80);
    const DEFAULT: Color32 = Color32::from_rgb(0xc9, 0xd1, 0xd9);

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

impl eframe::App for TermdbApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Always wake up promptly so late-arriving async events render even
        // without user input.
        ctx.request_repaint_after(Duration::from_millis(250));

        self.drain_events();

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            self.ui_status_bar(ui);
        });

        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(320.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(4.0);
                    self.ui_connections(ui);
                    ui.add_space(12.0);
                    self.ui_new_connection(ui);
                    ui.add_space(12.0);
                    self.ui_history(ui);
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            if self.view == View::Query {
                self.ui_query_workspace(ui);
            } else {
                self.ui_central(ui);
            }
            ui.add_space(4.0);
            self.ui_log(ui);
        });

        // Floating record panel (drawn last so it sits above everything).
        self.ui_record_panel(&ctx);
    }
}
