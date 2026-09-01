//! Shared domain models for termdb.

use serde::{Deserialize, Serialize};

/// Database engine a connection talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Mysql,
    Postgres,
}

impl Engine {
    /// Default listen port for the engine.
    pub fn default_port(self) -> u16 {
        match self {
            Engine::Mysql => 3306,
            Engine::Postgres => 5432,
        }
    }

    /// Human-readable engine name.
    pub fn label(self) -> &'static str {
        match self {
            Engine::Mysql => "MySQL",
            Engine::Postgres => "PostgreSQL",
        }
    }
}

/// A saved server connection. Passwords live in the vault, never here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// Row id once persisted; `None` for unsaved entries.
    pub id: Option<i64>,
    /// Unique display name, also used as the keyring lookup key.
    pub name: String,
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Initial database to select on connect, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default)]
    pub ssl: bool,
}

impl ConnectionConfig {
    /// Convenience for tests and examples.
    pub fn test(name: &str, engine: Engine) -> Self {
        Self {
            id: None,
            name: name.to_owned(),
            engine,
            host: "127.0.0.1".to_owned(),
            port: engine.default_port(),
            username: "root".to_owned(),
            database: None,
            ssl: false,
        }
    }
}
