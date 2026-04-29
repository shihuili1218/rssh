//! 密钥存储抽象层。
//!
//! 优先用系统 keychain（macOS Keychain / Windows Credential Manager / Linux Secret Service）。
//! 不可用时（Android、Linux headless、容器无 D-Bus 等）自动降级到 DB 的 `secrets` 表。
//!
//! Service 名固定 `rssh`，account 命名规则全平台、CLI/GUI 共用：
//! - `cred:<credential_id>:secret`     凭证主 secret（密码或私钥 PEM）
//! - `setting:github_token`            GitHub PAT
//!
//! 历史遗留：`cred:<credential_id>:passphrase` 曾用于存储私钥 passphrase，
//! 已废弃 — 启动时统一清空（参见 `lib.rs` 中的迁移），新流程通过终端交互输入
//! 并仅在进程内缓存。`cred_passphrase_key` 仍保留，仅供该清空逻辑使用。

use std::sync::Arc;

use crate::db::Db;
use crate::error::AppResult;

mod db_store;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod keyring_store;

/// 服务名 — keychain 用，所有 rssh 数据都在这个 service 下。
pub const SERVICE: &str = "rssh";

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> AppResult<Option<String>>;
    fn set(&self, key: &str, value: &str) -> AppResult<()>;
    fn delete(&self, key: &str) -> AppResult<()>;
    fn backend_name(&self) -> &'static str;
}

/// 打开 SecretStore：能用 keychain 就用，否则降级到 DB。
pub fn open(db: Arc<Db>) -> Arc<dyn SecretStore> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        if let Some(store) = keyring_store::try_open() {
            log::info!("secret store backend: keychain ({})", store.backend_name());
            return Arc::new(store);
        }
        log::warn!("system keychain unavailable, falling back to DB-backed secret store");
    }
    Arc::new(db_store::DbStore::new(db))
}

// --- helpers for canonical key naming (CLI/GUI must agree) ---

pub fn cred_secret_key(credential_id: &str) -> String {
    format!("cred:{credential_id}:secret")
}

pub fn cred_passphrase_key(credential_id: &str) -> String {
    format!("cred:{credential_id}:passphrase")
}

pub fn setting_key(name: &str) -> String {
    format!("setting:{name}")
}

/// settings 中按 secret 走 SecretStore 的键白名单。
pub const SECRET_SETTINGS: &[&str] = &["github_token"];

pub fn is_secret_setting(key: &str) -> bool {
    SECRET_SETTINGS.contains(&key)
}
