//! 系统 keychain 后端。仅在 macOS / Windows / Linux（Secret Service 可达时）可用。

use super::SecretStore;
use crate::error::{AppError, AppResult};

pub struct KeyringStore;

const PROBE_KEY: &str = "__rssh_probe__";
const PROBE_VALUE: &str = "ok";

/// 实测 keychain 可用性：写 → 读回 → 删。任何一步失败就拒绝。
pub fn try_open() -> Option<KeyringStore> {
    let entry = keyring::Entry::new(super::SERVICE, PROBE_KEY).ok()?;
    if entry.set_password(PROBE_VALUE).is_err() {
        return None;
    }
    let read_ok = entry
        .get_password()
        .ok()
        .as_deref()
        .map(|s| s == PROBE_VALUE)
        .unwrap_or(false);
    let _ = entry.delete_credential();
    if !read_ok {
        return None;
    }
    Some(KeyringStore)
}

fn entry(key: &str) -> AppResult<keyring::Entry> {
    keyring::Entry::new(super::SERVICE, key)
        .map_err(|e| AppError::Other(format!("keyring entry: {e}")))
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        match entry(key)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AppError::Other(format!("keyring get: {e}"))),
        }
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        entry(key)?
            .set_password(value)
            .map_err(|e| AppError::Other(format!("keyring set: {e}")))
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        match entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AppError::Other(format!("keyring delete: {e}"))),
        }
    }

    fn backend_name(&self) -> &'static str {
        if cfg!(target_os = "macos") {
            "macos-keychain"
        } else if cfg!(target_os = "windows") {
            "windows-credential-manager"
        } else {
            "linux-secret-service"
        }
    }
}
