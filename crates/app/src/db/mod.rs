//! Backend threading skeleton.
//!
//! egui owns the main/UI thread and must never block on I/O. A dedicated
//! thread hosts a tokio multi-thread runtime; the UI and the runtime talk
//! over two `std::sync::mpsc` channels which the UI drains every frame.
//!
//! M1 shipped the shape with a fake `Ping` round-trip; M2 adds real live
//! sqlx sessions for MySQL/Postgres on this same runtime.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

pub mod engine;

use termdb_core::{ConfigStore, ConnectionConfig, HistoryEntry, SecretStore, Vault, VaultKind};

use self::engine::{Column, EngineError, LiveSession, TableFilter};

/// Requests travelling UI -> backend.
#[derive(Debug)]
pub enum Request {
    /// Fake round-trip used to prove the channel plumbing.
    Ping { payload: String },
    /// Persist a connection + its password off the UI thread.
    SaveConnection {
        cfg: ConnectionConfig,
        password: String,
    },
    /// Open a pool to a saved connection and discover databases.
    Connect { conn_id: i64 },
    /// Close the pool for a connection.
    Disconnect { conn_id: i64 },
    /// List tables inside a database of a live connection.
    ListTables { conn_id: i64, database: String },
    /// Describe a table and load its first page in one go.
    OpenTable {
        conn_id: i64,
        database: String,
        table: String,
        filter: Option<TableFilter>,
        limit: i64,
    },
    /// Load a page of rows; `columns` come from the earlier `OpenTable`.
    LoadPage {
        conn_id: i64,
        database: String,
        table: String,
        columns: Vec<Column>,
        filter: Option<TableFilter>,
        limit: i64,
        offset: i64,
    },
    /// Run user-typed SQL against the connection's default pool.
    RunQuery { conn_id: i64, sql: String },
    /// Load recently run queries from disk.
    LoadHistory {},
    /// Forget all saved queries.
    ClearHistory {},
    /// Prepared UPDATE by primary key.
    UpdateRow {
        conn_id: i64,
        database: String,
        table: String,
        columns: Vec<Column>,
        values: Vec<(String, Option<String>)>,
        pk: (String, Option<String>),
    },
    /// Prepared INSERT.
    InsertRow {
        conn_id: i64,
        database: String,
        table: String,
        columns: Vec<Column>,
        values: Vec<(String, Option<String>)>,
    },
    /// Prepared DELETE by primary key.
    DeleteRow {
        conn_id: i64,
        database: String,
        table: String,
        columns: Vec<Column>,
        pk: (String, Option<String>),
    },
    /// Remove a saved connection: config row + vault entry (+ close live pool).
    DeleteConnection { conn_id: i64 },
}

/// Events travelling backend -> UI.
#[derive(Debug)]
pub enum Event {
    /// Emitted once the worker thread and runtime are up.
    BackendReady {
        version: &'static str,
        vault_kind: VaultKind,
    },
    /// Reply to [`Request::Ping`].
    Pong { payload: String, latency_ms: u64 },
    /// Confirmation that a connection was saved.
    ConnectionSaved { cfg: ConnectionConfig },
    /// A live session is up and its databases are listed.
    Connected {
        conn_id: i64,
        cfg: ConnectionConfig,
        server_version: String,
        databases: Vec<String>,
    },
    /// Could not open a session.
    ConnectFailed {
        conn_id: i64,
        name: String,
        message: String,
    },
    /// Pool closed and removed.
    Disconnected { conn_id: i64, name: String },
    /// Tables for a database of a live session.
    TablesListed {
        conn_id: i64,
        database: String,
        tables: Vec<String>,
    },
    /// Table listing failed.
    TablesFailed {
        conn_id: i64,
        database: String,
        message: String,
    },
    /// A table was described and its first page + total count loaded.
    TableOpened {
        conn_id: i64,
        database: String,
        table: String,
        columns: Vec<Column>,
        total: i64,
        rows: Vec<Vec<Option<String>>>,
    },
    /// Describe/first-page load failed.
    TableOpenFailed {
        conn_id: i64,
        database: String,
        table: String,
        message: String,
    },
    /// A later page of rows was loaded.
    PageLoaded {
        conn_id: i64,
        database: String,
        table: String,
        total: i64,
        rows: Vec<Vec<Option<String>>>,
    },
    /// Page load failed.
    PageFailed {
        conn_id: i64,
        database: String,
        table: String,
        message: String,
    },
    /// Ad-hoc query finished; `columns` are the result-set column names.
    QueryResults {
        conn_id: i64,
        columns: Vec<String>,
        rows: Vec<Vec<Option<String>>>,
    },
    /// Ad-hoc query failed.
    QueryFailed {
        conn_id: i64,
        sql: String,
        message: String,
    },
    /// History loaded from disk.
    HistoryListed { entries: Vec<HistoryEntry> },
    /// History cleared.
    HistoryCleared {},
    /// A row mutation (update/delete/insert) succeeded.
    RowChanged { conn_id: i64 },
    /// A row mutation failed.
    MutationFailed { message: String },
    /// A saved connection was removed from the config store + vault.
    ConnectionDeleted { conn_id: i64, name: String },
    /// Any persistence/backend error, surfaced as a log line.
    StoreFailed { message: String },
}

/// Handle held by the UI to talk to the backend.
pub struct Backend {
    tx: mpsc::Sender<Request>,
    rx: mpsc::Receiver<Event>,
}

impl Backend {
    pub fn spawn() -> Self {
        let (req_tx, req_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();

        thread::Builder::new()
            .name("termdb-backend".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .thread_name("termdb-worker")
                    .enable_io()
                    .enable_time()
                    .build()
                    .expect("tokio runtime failed to start");
                rt.block_on(worker_loop(req_rx, ev_tx));
            })
            .expect("failed to spawn backend thread");

        Self {
            tx: req_tx,
            rx: ev_rx,
        }
    }

    pub fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }

    /// Non-blocking drain; called once per frame.
    pub fn poll(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

async fn worker_loop(req_rx: mpsc::Receiver<Request>, ev_tx: mpsc::Sender<Event>) {
    // Creating the vault probes the OS keyring, which may touch D-Bus, so run
    // it off the async context.
    let config_dir = config_dir();
    let probe_dir = config_dir.clone();
    let vault = Arc::new(
        tokio::task::spawn_blocking(move || Vault::new(&probe_dir))
            .await
            .unwrap_or_else(|_| Vault::plaintext(config_dir.join("vault-plaintext.json"))),
    );

    let _ = ev_tx.send(Event::BackendReady {
        version: env!("CARGO_PKG_VERSION"),
        vault_kind: vault.kind(),
    });

    let mut sessions: HashMap<i64, LiveSession> = HashMap::new();

    // `recv` blocks; `block_in_place` parks a runtime worker on it so the
    // async context stays free. The channel closes when the UI shuts down.
    loop {
        let request = tokio::task::block_in_place(|| req_rx.recv());
        let Ok(request) = request else { break };
        match request {
            Request::Ping { payload } => {
                let started = Instant::now();
                tokio::time::sleep(Duration::from_millis(60)).await;
                let latency_ms = started.elapsed().as_millis() as u64;
                let _ = ev_tx.send(Event::Pong {
                    payload,
                    latency_ms,
                });
            }
            Request::SaveConnection { cfg, password } => {
                let config_dir = config_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let store = ConfigStore::open(&ConfigStore::default_path())
                        .map_err(|e| e.to_string())?;
                    let id = store.insert_connection(&cfg).map_err(|e| e.to_string())?;
                    let mut saved = cfg;
                    saved.id = Some(id);
                    let vault = Vault::new(&config_dir);
                    vault
                        .set(&saved.name, &password)
                        .map_err(|e| e.to_string())?;
                    Ok::<ConnectionConfig, String>(saved)
                })
                .await;
                match result.unwrap_or_else(|e| Err(format!("task join failed: {e}"))) {
                    Ok(cfg) => {
                        let _ = ev_tx.send(Event::ConnectionSaved { cfg });
                    }
                    Err(message) => {
                        let _ = ev_tx.send(Event::StoreFailed { message });
                    }
                }
            }
            Request::Connect { conn_id } => {
                let cfg = match ConfigStore::open(&ConfigStore::default_path())
                    .and_then(|store| store.get_connection(conn_id))
                {
                    Ok(Some(cfg)) => cfg,
                    Ok(None) => {
                        let _ = ev_tx.send(Event::ConnectFailed {
                            conn_id,
                            name: format!("connection #{conn_id}"),
                            message: "connection not found in config store".into(),
                        });
                        continue;
                    }
                    Err(e) => {
                        let _ = ev_tx.send(Event::ConnectFailed {
                            conn_id,
                            name: format!("connection #{conn_id}"),
                            message: e.to_string(),
                        });
                        continue;
                    }
                };
                let name = cfg.name.clone();
                let vault = vault.clone();
                let lookup_name = name.clone();
                let password = tokio::task::spawn_blocking(move || vault.get(&lookup_name)).await;
                match password {
                    Ok(Ok(Some(password))) => match LiveSession::connect(&cfg, &password).await {
                        Ok(session) => {
                            let server_version = session.server_version.clone();
                            let databases = session.databases.clone();
                            let listed = cfg.clone();
                            sessions.insert(conn_id, session);
                            let _ = ev_tx.send(Event::Connected {
                                conn_id,
                                cfg: listed,
                                server_version,
                                databases,
                            });
                        }
                        Err(message) => {
                            let _ = ev_tx.send(Event::ConnectFailed {
                                conn_id,
                                name,
                                message: format_engine_error(&message),
                            });
                        }
                    },
                    Ok(Ok(None)) => {
                        let _ = ev_tx.send(Event::ConnectFailed {
                            conn_id,
                            name: name.clone(),
                            message: format!("no password stored for \"{name}\" in the vault"),
                        });
                    }
                    Ok(Err(e)) => {
                        let _ = ev_tx.send(Event::ConnectFailed {
                            conn_id,
                            name,
                            message: e.to_string(),
                        });
                    }
                    Err(e) => {
                        let _ = ev_tx.send(Event::ConnectFailed {
                            conn_id,
                            name,
                            message: format!("task join failed: {e}"),
                        });
                    }
                }
            }
            Request::Disconnect { conn_id } => {
                if let Some(session) = sessions.remove(&conn_id) {
                    let name = session.cfg.name.clone();
                    session.disconnect().await;
                    let _ = ev_tx.send(Event::Disconnected { conn_id, name });
                }
            }
            Request::ListTables { conn_id, database } => match sessions.get_mut(&conn_id) {
                Some(session) => match session.tables(&database).await {
                    Ok(tables) => {
                        let _ = ev_tx.send(Event::TablesListed {
                            conn_id,
                            database,
                            tables,
                        });
                    }
                    Err(message) => {
                        let _ = ev_tx.send(Event::TablesFailed {
                            conn_id,
                            database,
                            message: format_engine_error(&message),
                        });
                    }
                },
                None => {
                    let _ = ev_tx.send(Event::TablesFailed {
                        conn_id,
                        database,
                        message: "not connected".into(),
                    });
                }
            },
            Request::OpenTable {
                conn_id,
                database,
                table,
                filter,
                limit,
            } => {
                let Some(session) = sessions.get_mut(&conn_id) else {
                    let _ = ev_tx.send(Event::TableOpenFailed {
                        conn_id,
                        database,
                        table,
                        message: "not connected".into(),
                    });
                    continue;
                };
                let result = async {
                    let columns = session.describe(&database, &table).await?;
                    let total = session
                        .count(&database, &table, &columns, filter.as_ref())
                        .await?;
                    let rows = session
                        .page(&database, &table, &columns, filter.as_ref(), limit, 0)
                        .await?;
                    Ok::<_, EngineError>((columns, total, rows))
                }
                .await;
                match result {
                    Ok((columns, total, rows)) => {
                        let _ = ev_tx.send(Event::TableOpened {
                            conn_id,
                            database,
                            table,
                            columns,
                            total,
                            rows,
                        });
                    }
                    Err(message) => {
                        let _ = ev_tx.send(Event::TableOpenFailed {
                            conn_id,
                            database,
                            table,
                            message: format_engine_error(&message),
                        });
                    }
                }
            }
            Request::LoadPage {
                conn_id,
                database,
                table,
                columns,
                filter,
                limit,
                offset,
            } => {
                let Some(session) = sessions.get_mut(&conn_id) else {
                    let _ = ev_tx.send(Event::PageFailed {
                        conn_id,
                        database,
                        table,
                        message: "not connected".into(),
                    });
                    continue;
                };
                let result = async {
                    let total = session
                        .count(&database, &table, &columns, filter.as_ref())
                        .await?;
                    let rows = session
                        .page(&database, &table, &columns, filter.as_ref(), limit, offset)
                        .await?;
                    Ok::<_, EngineError>((total, rows))
                }
                .await;
                match result {
                    Ok((total, rows)) => {
                        let _ = ev_tx.send(Event::PageLoaded {
                            conn_id,
                            database,
                            table,
                            total,
                            rows,
                        });
                    }
                    Err(message) => {
                        let _ = ev_tx.send(Event::PageFailed {
                            conn_id,
                            database,
                            table,
                            message: format_engine_error(&message),
                        });
                    }
                }
            }
            Request::RunQuery { conn_id, sql } => {
                let session = sessions.get(&conn_id);
                let outcome = match session {
                    Some(session) => match session.query_results(&sql).await {
                        Ok((columns, rows)) => Some((columns, rows)),
                        Err(message) => {
                            let _ = ev_tx.send(Event::QueryFailed {
                                conn_id,
                                sql: sql.clone(),
                                message: format_engine_error(&message),
                            });
                            None
                        }
                    },
                    None => {
                        let _ = ev_tx.send(Event::QueryFailed {
                            conn_id,
                            sql: sql.clone(),
                            message: "not connected".into(),
                        });
                        None
                    }
                };
                if let Some((columns, rows)) = outcome {
                    let _ = ev_tx.send(Event::QueryResults {
                        conn_id,
                        columns,
                        rows,
                    });
                }
                // Persist the query regardless of outcome so it can be retried.
                let history = sql.clone();
                tokio::task::spawn_blocking(move || {
                    let store = ConfigStore::open(&ConfigStore::default_path());
                    if let Ok(store) = store {
                        let _ = store.insert_history(Some(conn_id), &history);
                    }
                });
            }
            Request::LoadHistory {} => {
                let entries = tokio::task::spawn_blocking(move || {
                    ConfigStore::open(&ConfigStore::default_path())
                        .and_then(|store| store.list_history(50))
                })
                .await;
                match entries {
                    Ok(Ok(entries)) => {
                        let _ = ev_tx.send(Event::HistoryListed { entries });
                    }
                    Ok(Err(message)) => {
                        let _ = ev_tx.send(Event::StoreFailed {
                            message: message.to_string(),
                        });
                    }
                    Err(e) => {
                        let _ = ev_tx.send(Event::StoreFailed {
                            message: format!("history join failed: {e}"),
                        });
                    }
                }
            }
            Request::ClearHistory {} => {
                let _ = tokio::task::spawn_blocking(move || {
                    ConfigStore::open(&ConfigStore::default_path())
                        .and_then(|store| store.clear_history())
                })
                .await;
                let _ = ev_tx.send(Event::HistoryCleared {});
            }
            Request::UpdateRow {
                conn_id,
                database,
                table,
                columns,
                values,
                pk,
            } => {
                let outcome = sessions
                    .get_mut(&conn_id)
                    .map(|session| session.update_row(&database, &table, &columns, &values, &pk));
                match outcome {
                    Some(result) => match result.await {
                        Ok(_) => {
                            let _ = ev_tx.send(Event::RowChanged { conn_id });
                        }
                        Err(message) => {
                            let _ = ev_tx.send(Event::MutationFailed {
                                message: format_engine_error(&message),
                            });
                        }
                    },
                    None => {
                        let _ = ev_tx.send(Event::MutationFailed {
                            message: "not connected".into(),
                        });
                    }
                }
            }
            Request::InsertRow {
                conn_id,
                database,
                table,
                columns,
                values,
            } => {
                let outcome = sessions
                    .get_mut(&conn_id)
                    .map(|session| session.insert_row(&database, &table, &columns, &values));
                match outcome {
                    Some(result) => match result.await {
                        Ok(_) => {
                            let _ = ev_tx.send(Event::RowChanged { conn_id });
                        }
                        Err(message) => {
                            let _ = ev_tx.send(Event::MutationFailed {
                                message: format_engine_error(&message),
                            });
                        }
                    },
                    None => {
                        let _ = ev_tx.send(Event::MutationFailed {
                            message: "not connected".into(),
                        });
                    }
                }
            }
            Request::DeleteRow {
                conn_id,
                database,
                table,
                columns,
                pk,
            } => {
                let outcome = sessions
                    .get_mut(&conn_id)
                    .map(|session| session.delete_row(&database, &table, &columns, &pk));
                match outcome {
                    Some(result) => match result.await {
                        Ok(_) => {
                            let _ = ev_tx.send(Event::RowChanged { conn_id });
                        }
                        Err(message) => {
                            let _ = ev_tx.send(Event::MutationFailed {
                                message: format_engine_error(&message),
                            });
                        }
                    },
                    None => {
                        let _ = ev_tx.send(Event::MutationFailed {
                            message: "not connected".into(),
                        });
                    }
                }
            }
            Request::DeleteConnection { conn_id } => {
                // Drop any live session, then remove the config row + vault entry.
                if let Some(session) = sessions.remove(&conn_id) {
                    session.disconnect().await;
                }
                let config_dir = config_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let store = ConfigStore::open(&ConfigStore::default_path())
                        .map_err(|e| e.to_string())?;
                    let existing = store.get_connection(conn_id).map_err(|e| e.to_string())?;
                    store
                        .delete_connection(conn_id)
                        .map_err(|e| e.to_string())?;
                    if let Some(cfg) = &existing {
                        let vault = Vault::new(&config_dir);
                        vault.delete(&cfg.name).map_err(|e| e.to_string())?;
                    }
                    Ok(existing
                        .map(|c| c.name)
                        .unwrap_or_else(|| format!("#{conn_id}")))
                })
                .await;
                match result {
                    Ok(Ok(name)) => {
                        let _ = ev_tx.send(Event::ConnectionDeleted { conn_id, name });
                    }
                    Ok(Err(message)) => {
                        let _ = ev_tx.send(Event::StoreFailed { message });
                    }
                    Err(e) => {
                        let _ = ev_tx.send(Event::StoreFailed {
                            message: format!("delete join failed: {e}"),
                        });
                    }
                }
            }
        }
    }
}

fn format_engine_error(error: &EngineError) -> String {
    error.to_string()
}

fn config_dir() -> PathBuf {
    let path = ConfigStore::default_path();
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
