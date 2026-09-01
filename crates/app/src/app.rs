//! egui application shell: connection list + add form, live connection status,
//! database/table sidebar and paginated data grid (M3).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use egui::RichText;
use egui_extras::{Column as TableColumn, TableBuilder};
use termdb::db::engine::Column;
use termdb::db::{Backend, Event, Request};
use termdb_core::{ConfigStore, ConnectionConfig, Engine, VaultKind};

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

/// An opened table: describe + cached current page.
struct OpenTable {
    columns: Vec<Column>,
    rows: Vec<Vec<Option<String>>>,
    total: i64,
    page: i64,
    page_size: i64,
    loading: bool,
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
        }
    }
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
                            .color(egui::Color32::from_rgb(0x56, 0xd3, 0x64)),
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

    /// Central panel for a selected table: toolbar + virtualized paginated grid.
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

        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::LEFT))
            .column(TableColumn::auto());
        for _ in columns.iter().skip(1) {
            table = table.column(TableColumn::auto());
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
            })
            .body(|body| {
                body.rows(22.0, row_count, |mut row| {
                    let i = row.index();
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
                });
            });
    }

    fn goto_page(&mut self, key: &TableKey, page: i64) {
        let (conn_id, database, table) = key.clone();
        let Some(open) = self.open_tables.get_mut(key) else {
            return;
        };
        let columns = open.columns.clone();
        let page_size = open.page_size;
        let page = page.max(0);
        open.page = page;
        open.loading = true;
        self.backend.send(Request::LoadPage {
            conn_id,
            database,
            table,
            columns,
            limit: page_size,
            offset: page * page_size,
        });
    }

    fn ui_log(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Log");
            if ui.small_button("Clear").clicked() {
                self.log.clear();
            }
        });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &self.log {
                    ui.monospace(line);
                }
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
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            self.ui_central(ui);
            ui.separator();
            self.ui_log(ui);
        });
    }
}
