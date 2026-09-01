//! egui application shell for M1: connection list + add form, backend status,
//! event log. The data grid and query editor land in later milestones.

use std::time::{Duration, Instant};

use egui::RichText;
use termdb_core::{ConfigStore, ConnectionConfig, Engine, VaultKind};

use crate::db::{Backend, Event, Request};

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
                    let mut s = format!("backend: ready v{version} · {}", vault_kind.label());
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
        for cfg in &self.connections {
            let selected = self.selected_id == cfg.id;
            let label = format!("{} · {}:{}", cfg.name, cfg.engine.label(), cfg.port);
            if ui.selectable_label(selected, label).clicked() {
                self.selected_id = cfg.id;
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
                ui.label(
                    RichText::new("connecting to a live engine arrives in the M2 milestone")
                        .weak()
                        .italics(),
                );
            }
        } else {
            ui.label(RichText::new("select a connection, or add one").weak());
        }
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
