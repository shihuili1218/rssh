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
use crate::db::Db;
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
    /// 用来做跨进程互斥锁。SQLite `BEGIN IMMEDIATE` 是文件级 reserved-lock，
    /// 跨进程天然互斥，不需要新依赖（rssh.db 本来就有）。
    db: Arc<Db>,
}

impl KeyringMasterKey {
    pub fn new(keyring: Arc<dyn SecretStore>, db: Arc<Db>) -> Self {
        Self { keyring, db }
    }
}

impl MasterKeyBackend for KeyringMasterKey {
    /// 并发安全的"读 keychain 或生成":
    ///   - 快路径：keyring 已有 → 直接读，零数据库 IO
    ///   - 慢路径：拿 SQLite IMMEDIATE 锁 → 持锁后二次 check keyring → 生成 + 写入
    ///
    /// 跨进程 race 修复（CLI + GUI 同时首次启动）：
    /// 旧版本 `get() == None → set()` 两步之间另一进程可能也走了相同流程，
    /// 两个 random master key 被先后写入，winner 覆盖 loser；loser 进程缓存
    /// 的 mk_L 加密的密文重启后永久无法解（keychain 里只剩 mk_W）。
    /// 加 SQLite 文件锁后，整个 get→generate→set 序列被同一锁串行化：第二
    /// 个进程拿不到锁阻塞等待，等到时 keyring 已经有 winner 的 key，直接读
    /// 用之即可。
    fn load_or_create(&self) -> AppResult<[u8; MASTER_KEY_LEN]> {
        if let Some(b64) = self.keyring.get(KEYRING_KEY_NAME)? {
            return decode_b64_master_key(&b64);
        }
        // 慢路径：跨进程互斥的临界区
        self.db.with_exclusive_lock(|| {
            // 持锁后二次 check：等锁期间别的进程可能已经写入
            if let Some(b64) = self.keyring.get(KEYRING_KEY_NAME)? {
                return decode_b64_master_key(&b64);
            }
            let mk = random_master_key()?;
            self.keyring.set(KEYRING_KEY_NAME, &STANDARD.encode(mk))?;
            Ok(mk)
        })
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
    /// 并发/重启安全的"读取 or 创建"：
    ///   1. **先 try read** —— 已存在直接解码返回，不调 RNG（避免临时 getrandom 故障
    ///       影响现存用户）
    ///   2. 不存在则确保父目录存在
    ///   3. 用 `create_new = true` 原子创建文件 + 0600 + 写入新密钥
    ///      → OS 层面让"文件已存在"返回 `EEXIST`，**绝不覆盖**对手进程刚写好的密钥
    ///   4. 命中 `AlreadyExists`（read 之后到 create 之前对手抢先写）→ 读对手写的
    ///
    /// 旧版本 `exists() → 不存在则 create+truncate` 在两个进程首次启动时有竞态：
    /// A 看 exists()=false → 生成 mk_A → write_truncate；同一窗口 B 也看 exists()=
    /// false → 生成 mk_B → write_truncate 覆盖 A 写的；A 进程持有 mk_A（OnceLock 缓
    /// 存）继续工作，DB 落下 mk_A 加密的密文；下次启动从文件读 mk_B → mk_A 密文
    /// 永久解不开。`create_new` 把这层 race window 消掉。
    fn load_or_create(&self) -> AppResult<[u8; MASTER_KEY_LEN]> {
        // ── 快路径：已存在直接读，零 RNG 依赖 ──
        // RNG 暂时不可用（rare）的用户仍能解锁现存数据，只在真的需要生成时才 fail。
        if let Some(existing) = read_existing(&self.path)? {
            return decode_b64_master_key(&existing);
        }

        // ── 慢路径：创建 ──
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::other(
                    "master_key_mkdir_failed",
                    json!({ "dir": parent.display().to_string(), "err": e.to_string() }),
                )
            })?;
        }

        let mk = random_master_key()?;
        let b64 = STANDARD.encode(mk);

        match create_new_secure(&self.path, b64.as_bytes()) {
            Ok(()) => Ok(mk),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // 对手在我们 read→create 之间抢先创建；用他的
                let Some(existing) = read_existing(&self.path)? else {
                    return Err(AppError::other(
                        "master_key_read_failed",
                        json!({
                            "path": self.path.display().to_string(),
                            "err": "file vanished after AlreadyExists",
                        }),
                    ));
                };
                decode_b64_master_key(&existing)
            }
            Err(e) => Err(AppError::other(
                "master_key_write_failed",
                json!({
                    "op": "create_new",
                    "path": self.path.display().to_string(),
                    "err": e.to_string(),
                }),
            )),
        }
    }

    fn backend_name(&self) -> &'static str {
        "file"
    }
}

// ── helpers ─────────────────────────────────────────────────────────

/// 读 master.key 内容；不存在返回 None（NotFound 不视为错误，让 caller 决定生成）。
/// 其他 IO 错误（权限、磁盘）作 `master_key_read_failed` 抛出。
fn read_existing(path: &std::path::Path) -> AppResult<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s.trim().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AppError::other(
            "master_key_read_failed",
            json!({ "path": path.display().to_string(), "err": e.to_string() }),
        )),
    }
}

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

/// 原子 0600 **新建**并写入文件。两层保险：
///   - `create_new = true`：文件已存在 → OS 返回 `AlreadyExists`，**不写**。调用方
///     凭这个错误码判断 "对手已建文件" 走读取分支，杜绝覆盖竞争对手写的密钥。
///   - Unix `mode(0o600)`：文件自创建瞬间就是 0600，不经过 0644 中间态。
///     避免默认 umask（022）下"先 0644 再 chmod 0600"的 TOCTOU 窗口。
///
/// 返回 `io::Result` 而不是 `AppResult` —— 让 caller 用 `e.kind() ==
/// AlreadyExists` 决定走读取分支还是真错。把语义在边界处保留。
/// Windows 无 mode 概念，依赖用户目录 ACL（rssh data dir 默认 user-owned）。
fn create_new_secure(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(data)?;
    f.sync_all()?;
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
    fn create_new_race_loser_reads_winner_key() {
        // 模拟竞争：先用 backend A 创建一把 mk_A 写入文件；接下来 backend B 在同
        // 路径上 load_or_create —— B 走 AlreadyExists 分支，读到 mk_A，不能写新的
        // mk_B 覆盖 mk_A。
        let tmp = tempdir();
        let path = tmp.path().join(FILE_NAME);
        let mk_a = FileMasterKey::with_path(path.clone()).load_or_create().unwrap();
        // 第二个 backend 实例（模拟另一进程同时启动），在文件已存在时不能覆盖
        let mk_b = FileMasterKey::with_path(path.clone()).load_or_create().unwrap();
        assert_eq!(mk_a, mk_b, "AlreadyExists 必须走读取，不得覆盖竞争对手密钥");
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
