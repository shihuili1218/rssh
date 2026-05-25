//! 统一 SecretStore：master-key envelope encryption + DB 表存储。
//!
//! 架构（所有平台一致）：
//!
//! ```text
//!     master key (32B)  ─────►  MasterKeyBackend
//!         │                       ├─ keychain（首选）
//!         │                       └─ master.key 文件（keychain 不可用时）
//!         │
//!         ▼
//!     ChaCha20-Poly1305(master_key, nonce, plaintext) → ciphertext
//!         │
//!         ▼
//!     DB.secrets 表：(key TEXT PK, value TEXT)
//!         value = "enc:v1:" || base64(nonce || ct_with_tag)
//! ```
//!
//! 为什么不直接调 keychain 存原 secret：
//!   - Windows Credential Manager 硬限 2560 字节，RSA 私钥 PEM 必撞
//!   - 跨平台 keychain API 容量不统一，难维护
//!   - 行为偏好类（boolean）走 keychain 是滥用
//!
//! 主密钥生命周期：首次 set/get 触发 lazy 加载（OnceLock 缓存）。新用户没 secret
//! 就永不触发，零 keychain 调用。

use std::sync::{Arc, Mutex, OnceLock};

use super::crypto::{self, MASTER_KEY_LEN};
use super::db_store::DbStore;
use super::master_key::MasterKeyBackend;
use super::SecretStore;
use crate::error::{self, AppError, AppResult};

pub struct HybridStore {
    db_store: Arc<DbStore>,
    mk_backend: Arc<dyn MasterKeyBackend>,
    /// OnceLock 提供 atomic fast path：已初始化后所有线程无锁 atomic load。
    master_key: OnceLock<[u8; MASTER_KEY_LEN]>,
    /// Init 期间的串行化锁：第一次 master_key.get() == None 时多线程都会进入
    /// load_or_create；如果两个线程同时跑 KeyringMasterKey.load_or_create，
    /// A 看 keychain 没有 → 生成 mk_A 写入，B 看 keychain 没有 → 生成 mk_B 覆盖；
    /// OnceLock 只接受第一个 set（mk_A），但 keychain 持久化的是 mk_B → 重启
    /// 后 mk_A 加密的密文永久无法解。用 Mutex 串行化 init 排除这种 race。
    init_lock: Mutex<()>,
    backend_label: &'static str,
}

impl HybridStore {
    /// `backend_label`: 对外暴露的 backend 名字（CLI `rssh config show`、GUI tauri
    /// command `secret_backend` 显示给用户）。**保留 PR #60 前的字符串契约**：
    /// keychain 路径仍叫 `macos-keychain` / `windows-credential-manager` /
    /// `linux-secret-service`；新增的 `file` 给 FileMasterKey 路径。这样用户在
    /// CLI/GUI 看到的"我的 secret 存在哪"语义不变 —— secret 本来就锚定在 master
    /// key 的存储位置（root of trust），envelope 加密 DB 只是实现细节。
    pub fn new(
        db_store: Arc<DbStore>,
        mk_backend: Arc<dyn MasterKeyBackend>,
        backend_label: &'static str,
    ) -> Self {
        Self {
            db_store,
            mk_backend,
            master_key: OnceLock::new(),
            init_lock: Mutex::new(()),
            backend_label,
        }
    }

    /// Double-checked locking：fast path 无锁；slow path 拿锁后再次 check（防两个
    /// 线程都过了第一次 check）。OnceLock::get_or_try_init 至今仍是 nightly，
    /// stable Rust 用这套模式是惯用替代。
    fn master_key(&self) -> AppResult<&[u8; MASTER_KEY_LEN]> {
        // fast path：已初始化直接返回
        if let Some(k) = self.master_key.get() {
            return Ok(k);
        }
        // slow path：锁住 init 临界区。poison 走 crate::error::locked() 统一映射到
        // AppError::Lock（code = "lock_poisoned"），跟 codebase 其他 Mutex 处理一致，
        // 共享一套已有 i18n。
        let _guard = error::locked(&self.init_lock)?;
        // 二次 check：A 拿锁前可能 B 已经初始化完释放锁
        if let Some(k) = self.master_key.get() {
            return Ok(k);
        }
        let k = self.mk_backend.load_or_create()?;
        // 持锁状态下 set，第一个也是唯一一个
        self.master_key.set(k).expect("OnceLock set under init_lock cannot race");
        Ok(self.master_key.get().expect("just initialized"))
    }
}

impl SecretStore for HybridStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        let Some(stored) = self.db_store.get(key)? else {
            return Ok(None);
        };
        let mk = self.master_key()?;
        // key 作 AAD 绑死：行被 cut-and-paste 到别的 key 名下后 decrypt 必失败
        let plaintext = crypto::decrypt(mk, key, &stored)?;
        Ok(Some(String::from_utf8(plaintext).map_err(|e| {
            AppError::other(
                "secret_utf8_decode_failed",
                serde_json::json!({ "err": e.to_string() }),
            )
        })?))
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        let mk = self.master_key()?;
        // key 作 AAD（详见 crypto.rs 文档）
        let encrypted = crypto::encrypt(mk, key, value.as_bytes())?;
        self.db_store.set(key, &encrypted)
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        self.db_store.delete(key)
    }

    fn backend_name(&self) -> &'static str {
        self.backend_label
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::secret::master_key::FileMasterKey;

    fn make_hybrid() -> (HybridStore, tempfile::TempDir) {
        // in-memory DB + 临时目录里的 master.key 文件
        let tmp = tempfile::TempDir::new().unwrap();
        let db = Arc::new(Db::open_in_memory().unwrap());
        let db_store = Arc::new(DbStore::new(db));
        let mk = Arc::new(FileMasterKey::with_path(tmp.path().join("mk")));
        let hs = HybridStore::new(db_store, mk, "file");
        (hs, tmp)
    }

    #[test]
    fn set_then_get_roundtrip() {
        let (hs, _tmp) = make_hybrid();
        hs.set("cred:abc:secret", "my-secret-pem").unwrap();
        assert_eq!(
            hs.get("cred:abc:secret").unwrap().as_deref(),
            Some("my-secret-pem")
        );
    }

    #[test]
    fn long_value_works() {
        // 模拟 RSA 4096 PEM 超 2560 byte 场景
        let (hs, _tmp) = make_hybrid();
        let pem = "x".repeat(3500);
        hs.set("cred:foo:secret", &pem).unwrap();
        assert_eq!(hs.get("cred:foo:secret").unwrap().as_deref(), Some(pem.as_str()));
    }

    #[test]
    fn missing_key_returns_none() {
        let (hs, _tmp) = make_hybrid();
        assert!(hs.get("ghost").unwrap().is_none());
    }

    #[test]
    fn delete_removes_value() {
        let (hs, _tmp) = make_hybrid();
        hs.set("k", "v").unwrap();
        hs.delete("k").unwrap();
        assert!(hs.get("k").unwrap().is_none());
    }

    #[test]
    fn db_stores_ciphertext_not_plaintext() {
        // 关键安全断言：DB raw 值是加密后的 blob，不是用户明文
        let (hs, _tmp) = make_hybrid();
        hs.set("k", "plaintext-marker").unwrap();
        let raw = hs.db_store.get("k").unwrap().unwrap();
        assert!(raw.starts_with("enc:v1:"));
        assert!(!raw.contains("plaintext-marker"));
    }

    #[test]
    fn empty_value_roundtrips() {
        let (hs, _tmp) = make_hybrid();
        hs.set("k", "").unwrap();
        assert_eq!(hs.get("k").unwrap().as_deref(), Some(""));
    }

    #[test]
    fn overwrite_uses_new_nonce() {
        // 同 key 第二次 set 应该产出不同 nonce（DB 里 raw 值不同），但 get 都回新值
        let (hs, _tmp) = make_hybrid();
        hs.set("k", "v").unwrap();
        let raw1 = hs.db_store.get("k").unwrap().unwrap();
        hs.set("k", "v").unwrap();
        let raw2 = hs.db_store.get("k").unwrap().unwrap();
        assert_ne!(raw1, raw2, "随机 nonce 让同明文同 key 两次 set 产出不同密文");
        assert_eq!(hs.get("k").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn cross_key_swap_rejected_at_secretstore_layer() {
        // 攻击模拟：attacker 能写 SQLite secrets 表，把 cred:A:secret 的密文 raw
        // 复制到 cred:B:secret 行。HybridStore.get(cred:B:secret) 必须报 AEAD tag
        // 失败，不能静默返回 A 的明文密码。
        let (hs, _tmp) = make_hybrid();
        hs.set("cred:A:secret", "password-A").unwrap();
        // 拿 raw 密文（绕过 HybridStore 直接读 DB.secrets）
        let raw_a = hs.db_store.get("cred:A:secret").unwrap().unwrap();
        // 把 A 的密文搬到 B 的 key 下
        hs.db_store.set("cred:B:secret", &raw_a).unwrap();
        // HybridStore.get 解 B 必须失败 — AAD 不一致 tag fail
        let err = hs.get("cred:B:secret").unwrap_err();
        assert_eq!(err.code(), "secret_decrypt_failed_or_wrong_key");
    }

    #[test]
    fn lazy_master_key_not_loaded_until_first_use() {
        // 创建 HybridStore 不应立刻触发主密钥生成
        let (hs, tmp) = make_hybrid();
        // master.key 文件还没被创建
        assert!(!tmp.path().join("mk").exists());
        // 首次 set 触发
        hs.set("k", "v").unwrap();
        assert!(tmp.path().join("mk").exists());
    }
}
