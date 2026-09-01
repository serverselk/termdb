//! Local SQLite-backed config store (connections + key/value settings).
//!
//! Replaces the sql.js store from the Electron app. Only *metadata* lives
//! here; credentials go to the vault (`crate::vault`).

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::models::{ConnectionConfig, Engine};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("filesystem: {0}")]
    Io(#[from] std::io::Error),
    #[error("connection name '{name}' is already in use")]
    DuplicateName { name: String },
}

/// A single persisted connection row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ConnectionRow {
    id: i64,
    name: String,
    engine: String,
    host: String,
    port: u16,
    username: String,
    database: Option<String>,
    ssl: bool,
}

impl From<ConnectionRow> for ConnectionConfig {
    fn from(row: ConnectionRow) -> Self {
        Self {
            id: Some(row.id),
            name: row.name,
            engine: match row.engine.as_str() {
                "postgres" => Engine::Postgres,
                _ => Engine::Mysql,
            },
            host: row.host,
            port: row.port,
            username: row.username,
            database: row.database,
            ssl: row.ssl,
        }
    }
}

impl From<&ConnectionConfig> for ConnectionRow {
    fn from(cfg: &ConnectionConfig) -> Self {
        Self {
            id: cfg.id.unwrap_or(0),
            name: cfg.name.clone(),
            engine: match cfg.engine {
                Engine::Mysql => "mysql",
                Engine::Postgres => "postgres",
            }
            .to_owned(),
            host: cfg.host.clone(),
            port: cfg.port,
            username: cfg.username.clone(),
            database: cfg.database.clone(),
            ssl: cfg.ssl,
        }
    }
}

/// SQLite-backed config store. Cheap local I/O, safe on the UI thread.
pub struct ConfigStore {
    conn: Connection,
}

impl ConfigStore {
    fn new(conn: Connection) -> Result<Self, ConfigError> {
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    /// Open (creating if needed) the store at `path`. Missing parent
    /// directories are created, so a first run on a fresh machine works.
    pub fn open(path: &Path) -> Result<Self, ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::new(conn)
    }

    /// In-memory store, mainly for tests.
    pub fn in_memory() -> Result<Self, ConfigError> {
        let conn = Connection::open_in_memory()?;
        Self::new(conn)
    }

    /// Default location for the config database under the platform config dir.
    pub fn default_path() -> std::path::PathBuf {
        let dir = directories::ProjectDirs::from("com", "serverse", "termdb")
            .expect("project dirs should resolve")
            .config_dir()
            .to_path_buf();
        dir.join("termdb.sqlite3")
    }

    fn init_schema(&self) -> Result<(), ConfigError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS connections (
                id       INTEGER PRIMARY KEY AUTOINCREMENT,
                name     TEXT    NOT NULL UNIQUE,
                engine   TEXT    NOT NULL,
                host     TEXT    NOT NULL,
                port     INTEGER NOT NULL,
                username TEXT    NOT NULL,
                database TEXT,
                ssl      INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    /// All saved connections, ordered by name.
    pub fn list_connections(&self) -> Result<Vec<ConnectionConfig>, ConfigError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, engine, host, port, username, database, ssl
             FROM connections ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ConnectionRow {
                id: row.get(0)?,
                name: row.get(1)?,
                engine: row.get(2)?,
                host: row.get(3)?,
                port: u16::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                username: row.get(5)?,
                database: row.get(6)?,
                ssl: row.get::<_, i64>(7)? != 0,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map(|rows| rows.into_iter().map(ConnectionConfig::from).collect())
            .map_err(Into::into)
    }

    /// Fetch a single connection by id.
    pub fn get_connection(&self, id: i64) -> Result<Option<ConnectionConfig>, ConfigError> {
        let row = self
            .conn
            .query_row(
                "SELECT id, name, engine, host, port, username, database, ssl
                 FROM connections WHERE id = ?1",
                [id],
                |row| {
                    Ok(ConnectionRow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        engine: row.get(2)?,
                        host: row.get(3)?,
                        port: u16::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
                        username: row.get(5)?,
                        database: row.get(6)?,
                        ssl: row.get::<_, i64>(7)? != 0,
                    })
                },
            )
            .optional()?;
        Ok(row.map(ConnectionConfig::from))
    }

    /// Insert a connection. Returns the new row id.
    pub fn insert_connection(&self, cfg: &ConnectionConfig) -> Result<i64, ConfigError> {
        let row = ConnectionRow::from(cfg);
        let res = self.conn.execute(
            "INSERT INTO connections (name, engine, host, port, username, database, ssl)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.name,
                row.engine,
                row.host,
                i64::from(row.port),
                row.username,
                row.database,
                i64::from(row.ssl)
            ],
        );
        match res {
            Ok(_) => Ok(self.conn.last_insert_rowid()),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ffi::ErrorCode::ConstraintViolation =>
            {
                Err(ConfigError::DuplicateName {
                    name: cfg.name.clone(),
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Update an existing connection (matched by id). `Missing` is returned
    /// with `usize`? Use count: returns 0 when no row matched.
    pub fn update_connection(&self, cfg: &ConnectionConfig) -> Result<bool, ConfigError> {
        let Some(id) = cfg.id else {
            return Ok(false);
        };
        let row = ConnectionRow::from(cfg);
        let changed = self.conn.execute(
            "UPDATE connections SET name=?1, engine=?2, host=?3, port=?4,
                    username=?5, database=?6, ssl=?7 WHERE id=?8",
            params![
                row.name,
                row.engine,
                row.host,
                i64::from(row.port),
                row.username,
                row.database,
                i64::from(row.ssl),
                id
            ],
        )?;
        Ok(changed > 0)
    }

    /// Delete a connection by id. Returns true if a row was removed.
    pub fn delete_connection(&self, id: i64) -> Result<bool, ConfigError> {
        Ok(self
            .conn
            .execute("DELETE FROM connections WHERE id = ?1", [id])?
            > 0)
    }

    /// Look up a string setting.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, ConfigError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Create or update a string setting.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), ConfigError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Engine;

    #[test]
    fn connection_round_trip() {
        let store = ConfigStore::in_memory().unwrap();
        let id = store
            .insert_connection(&ConnectionConfig::test("shop", Engine::Postgres))
            .unwrap();
        assert_eq!(id, 1);

        let mut want = ConnectionConfig::test("shop", Engine::Postgres);
        want.id = Some(id);
        want.host = "pg.example.com".to_owned();
        want.port = 5433;
        want.database = Some("widgets".to_owned());
        want.ssl = true;
        store.update_connection(&want).unwrap();

        let got = store.get_connection(id).unwrap().unwrap();
        assert_eq!(got, want);

        let all = store.list_connections().unwrap();
        assert_eq!(all, vec![want.clone()]);
        assert!(store.delete_connection(id).unwrap());
        assert!(store.list_connections().unwrap().is_empty());
    }

    #[test]
    fn connection_names_are_unique() {
        let store = ConfigStore::in_memory().unwrap();
        store
            .insert_connection(&ConnectionConfig::test("shop", Engine::Mysql))
            .unwrap();
        let err = store
            .insert_connection(&ConnectionConfig::test("shop", Engine::Mysql))
            .unwrap_err();
        assert!(matches!(err, ConfigError::DuplicateName { .. }));
    }

    #[test]
    fn settings_round_trip_and_upsert() {
        let store = ConfigStore::in_memory().unwrap();
        assert_eq!(store.get_setting("theme").unwrap(), None);
        store.set_setting("theme", "omarchy").unwrap();
        store.set_setting("theme", "matrix").unwrap();
        assert_eq!(
            store.get_setting("theme").unwrap(),
            Some("matrix".to_owned())
        );
    }

    #[test]
    fn open_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("nested")
            .join("deeper")
            .join("termdb.sqlite3");
        assert!(!path.parent().unwrap().exists());

        let store = ConfigStore::open(&path).unwrap();
        let id = store
            .insert_connection(&ConnectionConfig::test("shop", Engine::Postgres))
            .unwrap();
        drop(store);

        assert!(path.exists());
        let reopened = ConfigStore::open(&path).unwrap();
        assert_eq!(reopened.get_connection(id).unwrap().unwrap().name, "shop");
    }
}
