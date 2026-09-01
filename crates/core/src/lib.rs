//! termdb-core: local config store, credential vault, and shared models.
//!
//! Kept free of egui and tokio so it can be reused by the GUI, the MCP
//! server, and any headless tooling without pulling in a UI stack.

pub mod config;
pub mod models;
pub mod vault;

pub use config::{ConfigError, ConfigStore};
pub use models::{ConnectionConfig, Engine};
pub use vault::{FileVault, KeyringVault, SecretStore, Vault, VaultError, VaultKind};
