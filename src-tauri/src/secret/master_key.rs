//! 主密钥（32 字节）的持久化存放。
//!
//! 加密 DB secret 用一把固定的 32B 主密钥。这把密钥本身不能再放 DB（鸡生蛋），
//! 它要去一个 attacker 拿不到、进程能拿到的地方：
//!
//!   - **keychain 可用**（mac/win/linux-desktop）→ `KeyringMasterKey`。
//!     32 字节 base64 编码后 44 字符，远低于 Windows Credential Manager 2560 字节硬限。
//!   - **keychain 不可用**（headless / 容器 / Android）→ `FileMasterKey`，
//!     落 `<data_dir>/master.key`，Unix 设 0600 权限。
//!     注：security 等级与"DB 明文"几乎相同（attacker 拿到 user 权限就同时拿
//!     master.key + DB）；但保留这条路径让"统一架构"在所有平台真正统一。
//!
//! 生命周期：lazy 创建。首次 encrypt/decrypt 调用触发 `load_or_create`。
//!   - 已存在 → 解码返回
//!   - 不存在 → 生成 32B 随机 → 持久化 → 返回
//!
//! **跨设备：本模块只管"本机生成 + 本机持有"。GitHub Sync / export / import
//! 走的是用户密码 KDF（src/crypto.rs），跟本主密钥独立。**

use std::path::PathBuf;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::json;

use super::crypto::MASTER_KEY_LEN;
use super::SecretStore;
use crate::error::{AppError, AppResult};

/// keychain 里主密钥的 key 名。带 v1 后缀方便未来轮换。
/// 不用 `setting:` 命名空间前缀 —— 这是 rssh 自管的内部数据，不是 user-facing setting。
const KEYRING_KEY_NAME: &str = "rssh_master_key_v1";

/// master.key 文件名（在 data_dir 里）。
const FILE_NAME: &str = "master.key";

pub trait MasterKeyBackend: Send + Sync {
    /// 已存在则解码返回；不存在则生成 + 持久化 + 返回。
    /// 多线程并发首次调用由调用方（HybridStore 用 OnceLock）串行化。
    fn load_or_create(&self) -> AppResult<[u8; MASTER_KEY_LEN]>;

    /// 后端名，给日志/诊断用。
    fn backend_name(&self) -> &'static str;
}

// ── KeyringMasterKey ────────────────────────────────────────────────

pub struct KeyringMasterKey {
    /// 任意 SecretStore，但**绝不能**传 HybridStore，否则 HybridStore 找主密钥时
    /// 又调回 HybridStore.get 死循环。secret::open 内部保证只传 raw KeyringStore。
    keyring: Arc<dyn SecretStore>,
}

impl KeyringMasterKey {
    pub fn new(keyring: Arc<dyn SecretStore>) -> Self {
        Self { keyring }
    }
}

impl MasterKeyBackend for KeyringMasterKey {
    fn load_or_create(&self) -> AppResult<[u8; MASTER_KEY_LEN]> {
        if let Some(b64) = self.keyring.get(KEYRING_KEY_NAME)? {
            return decode_b64_master_key(&b64);
        }
        let mk = random_master_key()?;
        self.keyring.set(KEYRING_KEY_NAME, &STANDARD.encode(mk))?;
        Ok(mk)
    }

    fn backend_name(&self) -> &'static str {
        "keyring"
    }
}

// ── FileMasterKey ───────────────────────────────────────────────────

pub struct FileMasterKey {
    path: PathBuf,
}

impl FileMasterKey {
    pub fn new(data_dir: &std::path::Path) -> Self {
        Self {
            path: data_dir.join(FILE_NAME),
        }
    }

    #[cfg(test)]
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }
}

impl MasterKeyBackend for FileMasterKey {
    fn load_or_create(&self) -> AppResult<[u8; MASTER_KEY_LEN]> {
        if self.path.exists() {
            let b64 = std::fs::read_to_string(&self.path).map_err(|e| {
                AppError::other(
                    "master_key_read_failed",
                    json!({ "path": self.path.display().to_string(), "err": e.to_string() }),
                )
            })?;
            return decode_b64_master_key(b64.trim());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::other(
                    "master_key_mkdir_failed",
                    json!({ "dir": parent.display().to_string(), "err": e.to_string() }),
                )
            })?;
        }
        let mk = random_master_key()?;
        write_file_secure(&self.path, STANDARD.encode(mk).as_bytes())?;
        Ok(mk)
    }

    fn backend_name(&self) -> &'static str {
        "file"
    }
}

// ── helpers ─────────────────────────────────────────────────────────

fn random_master_key() -> AppResult<[u8; MASTER_KEY_LEN]> {
    let mut mk = [0u8; MASTER_KEY_LEN];
    getrandom::getrandom(&mut mk)
        .map_err(|e| AppError::other("master_key_rng_failed", json!({ "err": e.to_string() })))?;
    Ok(mk)
}

fn decode_b64_master_key(b64: &str) -> AppResult<[u8; MASTER_KEY_LEN]> {
    let bytes = STANDARD
        .decode(b64.as_bytes())
        .map_err(|e| AppError::other("master_key_b64_decode_failed", json!({ "err": e.to_string() })))?;
    bytes.try_into().map_err(|v: Vec<u8>| {
        AppError::other(
            "master_key_wrong_length",
            json!({ "expected": MASTER_KEY_LEN, "got": v.len() }),
        )
    })
}

/// 写入文件并尝试设置 0600 权限（Unix）；Windows 没有 mode 概念，依赖用户目录的 ACL。
fn write_file_secure(path: &std::path::Path, data: &[u8]) -> AppResult<()> {
    std::fs::write(path, data).map_err(|e| {
        AppError::other(
            "master_key_write_failed",
            json!({ "path": path.display().to_string(), "err": e.to_string() }),
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
            AppError::other(
                "master_key_chmod_failed",
                json!({ "path": path.display().to_string(), "err": e.to_string() }),
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backend_creates_then_reads_same_key() {
        let tmp = tempdir();
        let backend = FileMasterKey::with_path(tmp.path().join(FILE_NAME));
        let k1 = backend.load_or_create().unwrap();
        let k2 = backend.load_or_create().unwrap();
        assert_eq!(k1, k2, "二次调用必须返回同一把密钥");
    }

    #[test]
    fn file_backend_persists_to_disk() {
        // 不同 backend 实例（模拟进程重启）读同一文件应得到同 key
        let tmp = tempdir();
        let path = tmp.path().join(FILE_NAME);
        let k1 = FileMasterKey::with_path(path.clone()).load_or_create().unwrap();
        let k2 = FileMasterKey::with_path(path).load_or_create().unwrap();
        assert_eq!(k1, k2);
    }

    #[test]
    fn file_backend_creates_parent_dir() {
        let tmp = tempdir();
        let nested = tmp.path().join("does_not_exist").join("yet");
        let backend = FileMasterKey::with_path(nested.join(FILE_NAME));
        let _ = backend.load_or_create().unwrap();
        assert!(nested.exists());
    }

    #[cfg(unix)]
    #[test]
    fn file_backend_sets_0600_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir();
        let path = tmp.path().join(FILE_NAME);
        let backend = FileMasterKey::with_path(path.clone());
        backend.load_or_create().unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "master.key 必须是 0600");
    }

    #[test]
    fn corrupted_b64_returns_clear_error() {
        let tmp = tempdir();
        let path = tmp.path().join(FILE_NAME);
        std::fs::write(&path, b"!!!not base64!!!").unwrap();
        let err = FileMasterKey::with_path(path).load_or_create().unwrap_err();
        assert_eq!(err.code(), "master_key_b64_decode_failed");
    }

    #[test]
    fn wrong_length_returns_clear_error() {
        // base64 解码后是 8 字节，不是 32，必须报错而不是 panic
        let tmp = tempdir();
        let path = tmp.path().join(FILE_NAME);
        std::fs::write(&path, STANDARD.encode(b"too short").as_bytes()).unwrap();
        let err = FileMasterKey::with_path(path).load_or_create().unwrap_err();
        assert_eq!(err.code(), "master_key_wrong_length");
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::TempDir::new().expect("tempdir")
    }
}
