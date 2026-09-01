//! Backend threading skeleton.
//!
//! egui owns the main/UI thread and must never block on I/O. A dedicated
//! thread hosts a tokio multi-thread runtime; the UI and the runtime talk
//! over two `std::sync::mpsc` channels which the UI drains every frame.
//!
//! M1 only ships a fake `Ping` round-trip and local-save plumbing to prove
//! the shape. M2 will grow `Request::Connect`/`Query` variants that hand off
//! to sqlx pools living on this same runtime.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use termdb_core::{ConfigStore, ConnectionConfig, SecretStore, Vault, VaultKind};

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
    /// Any persistence/backend error, surfaced as a log line.
    StoreFailed { message: String },
}

/// Handle held by the UI to talk to the backend.
pub struct Backend {
    tx: Sender<Request>,
    rx: Receiver<Event>,
}

impl Backend {
    pub fn spawn() -> Self {
        let (req_tx, req_rx) = channel();
        let (ev_tx, ev_rx) = channel();

        thread::Builder::new()
            .name("termdb-backend".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .thread_name("termdb-worker")
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

async fn worker_loop(req_rx: Receiver<Request>, ev_tx: Sender<Event>) {
    // Creating the vault probes the OS keyring, which may touch D-Bus, so run
    // it off the async context.
    let config_dir = config_dir();
    let probe_dir = config_dir.clone();
    let vault_kind = tokio::task::spawn_blocking(move || Vault::new(&probe_dir).kind())
        .await
        .unwrap_or(VaultKind::PlaintextFallback);

    let _ = ev_tx.send(Event::BackendReady {
        version: env!("CARGO_PKG_VERSION"),
        vault_kind,
    });

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
        }
    }
}

fn config_dir() -> PathBuf {
    let path = ConfigStore::default_path();
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
