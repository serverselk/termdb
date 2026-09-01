//! Credential vault: OS keyring via `keyring`, with a plaintext file fallback
//! when no secret service is available (e.g. headless / locked-down sessions).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("keyring: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("vault file: {0}")]
    Io(#[from] io::Error),
    #[error("vault file is not valid JSON: {0}")]
    Corrupt(#[from] serde_json::Error),
}

/// Where the fallback vault stores its secrets.
pub type VaultKindValue = &'static str;

/// Which backend currently backs the vault (for display).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultKind {
    Keyring,
    PlaintextFallback,
}

impl VaultKind {
    pub fn label(&self) -> &'static str {
        match self {
            VaultKind::Keyring => "system keyring (Secret Service / Keychain / DPAPI)",
            VaultKind::PlaintextFallback => "plaintext fallback",
        }
    }
}

/// Secrets for a named connection.
pub trait SecretStore {
    fn get(&self, key: &str) -> Result<Option<String>, VaultError>;
    fn set(&self, key: &str, secret: &str) -> Result<(), VaultError>;
    fn delete(&self, key: &str) -> Result<(), VaultError>;
}

/// OS keyring-backed store.
pub struct KeyringVault {
    service_name: &'static str,
}

impl KeyringVault {
    pub const SERVICE: &'static str = "com.serverse.termdb";

    pub fn new() -> Self {
        Self {
            service_name: Self::SERVICE,
        }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, VaultError> {
        Ok(keyring::Entry::new(self.service_name, key)?)
    }
}

impl Default for KeyringVault {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for KeyringVault {
    fn get(&self, key: &str) -> Result<Option<String>, VaultError> {
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), VaultError> {
        self.entry(key)?.set_password(secret)?;
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), VaultError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Plaintext fallback: one JSON object per vault file, chmod 0600 on unix.
pub struct FileVault {
    path: PathBuf,
}

impl FileVault {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn read_map(&self) -> Result<BTreeMap<String, String>, VaultError> {
        match fs::read(&self.path) {
            Ok(bytes) if bytes.is_empty() => Ok(BTreeMap::new()),
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e.into()),
        }
    }

    fn write_map(&self, map: &BTreeMap<String, String>) -> Result<(), VaultError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&self.path)?;
        use std::io::Write;
        serde_json::to_writer_pretty(&mut file, map)?;
        file.write_all(b"\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

impl SecretStore for FileVault {
    fn get(&self, key: &str) -> Result<Option<String>, VaultError> {
        Ok(self.read_map()?.remove(key))
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), VaultError> {
        let mut map = self.read_map()?;
        map.insert(key.to_owned(), secret.to_owned());
        self.write_map(&map)
    }

    fn delete(&self, key: &str) -> Result<(), VaultError> {
        let mut map = self.read_map()?;
        map.remove(key);
        self.write_map(&map)
    }
}

/// The vault used by the app: tries the OS keyring once at startup and falls
/// back to the plaintext file when no secret service is reachable.
pub struct Vault {
    backend: VaultBackend,
}

enum VaultBackend {
    Keyring(KeyringVault),
    File(FileVault),
}

impl Vault {
    /// Probe the environment and pick the best available backend.
    ///
    /// Connections names live in the config store; the vault keys on the same
    /// names so the two stay in sync after a delete.
    pub fn new(config_dir: &Path) -> Self {
        let keyring = KeyringVault::new();
        let probe_key = "__termdb_probe__";
        let usable = keyring.set(probe_key, "probe").is_ok()
            && keyring.get(probe_key).is_ok()
            && keyring.delete(probe_key).is_ok();

        let backend = if usable {
            VaultBackend::Keyring(keyring)
        } else {
            let file = config_dir.join("vault-plaintext.json");
            VaultBackend::File(FileVault::new(file))
        };
        Self { backend }
    }

    /// Force the plaintext backend (used in tests and for debugging).
    pub fn plaintext(path: PathBuf) -> Self {
        Self {
            backend: VaultBackend::File(FileVault::new(path)),
        }
    }

    /// Which backend is active right now.
    pub fn kind(&self) -> VaultKind {
        match self.backend {
            VaultBackend::Keyring(_) => VaultKind::Keyring,
            VaultBackend::File(_) => VaultKind::PlaintextFallback,
        }
    }
}

impl SecretStore for Vault {
    fn get(&self, key: &str) -> Result<Option<String>, VaultError> {
        match &self.backend {
            VaultBackend::Keyring(v) => v.get(key),
            VaultBackend::File(v) => v.get(key),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), VaultError> {
        match &self.backend {
            VaultBackend::Keyring(v) => v.set(key, secret),
            VaultBackend::File(v) => v.set(key, secret),
        }
    }

    fn delete(&self, key: &str) -> Result<(), VaultError> {
        match &self.backend {
            VaultBackend::Keyring(v) => v.delete(key),
            VaultBackend::File(v) => v.delete(key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault() -> (tempfile::TempDir, FileVault) {
        let dir = tempfile::tempdir().unwrap();
        let vault = FileVault::new(dir.path().join("vault.json"));
        (dir, vault)
    }

    #[test]
    fn file_vault_round_trip() {
        let (_dir, vault) = temp_vault();
        assert_eq!(vault.get("shop").unwrap(), None);
        vault.set("shop", "s3cret").unwrap();
        assert_eq!(vault.get("shop").unwrap(), Some("s3cret".to_owned()));
        vault.set("shop", "rotated").unwrap();
        assert_eq!(vault.get("shop").unwrap(), Some("rotated".to_owned()));
        vault.delete("shop").unwrap();
        assert_eq!(vault.get("shop").unwrap(), None);
    }

    #[test]
    fn file_vault_multiple_keys() {
        let (_dir, vault) = temp_vault();
        vault.set("a", "1").unwrap();
        vault.set("b", "2").unwrap();
        assert_eq!(vault.get("a").unwrap(), Some("1".to_owned()));
        assert_eq!(vault.get("b").unwrap(), Some("2".to_owned()));
    }

    #[cfg(unix)]
    #[test]
    fn file_vault_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, vault) = temp_vault();
        vault.set("shop", "s3cret").unwrap();
        let mode = fs::metadata(&vault.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn vault_plaintext_backend_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::plaintext(dir.path().join("v.json"));
        assert_eq!(vault.kind(), VaultKind::PlaintextFallback);
        vault.set("shop", "pw").unwrap();
        assert_eq!(vault.get("shop").unwrap(), Some("pw".to_owned()));
        vault.delete("shop").unwrap();
        assert_eq!(vault.get("shop").unwrap(), None);
    }
}
