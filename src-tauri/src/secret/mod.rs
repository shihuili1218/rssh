//! 密钥存储抽象层 —— 统一架构（所有平台一致）。
//!
//! 设计：master-key envelope encryption + DB 表存储。
//!
//! ```text
//!     SecretStore.set/get   ← 调用方（cred:* / setting:* 等）
//!         │
//!         ▼
//!     HybridStore             ← ChaCha20-Poly1305 加/解密
//!       ├── master_key (32B)
//!       │     ├── KeyringMasterKey  ← keychain（mac/win/linux-desktop）
//!       │     └── FileMasterKey     ← <data_dir>/master.key（headless/Android）
//!       │
//!       └── DbStore  ← rssh.db 的 `secrets` 表（密文 base64）
//! ```
//!
//! 为什么不再像旧版那样直接走 keychain：
//!   - Windows Credential Manager 硬限 2560 字节，RSA 私钥 PEM 必撞
//!   - 跨平台 keychain 容量/性能不统一
//!   - "把布尔开关塞 keychain" 是滥用（PR #59 已把行为偏好搬出去）
//!
//! 主密钥 lazy 生成：首次 set/get 触发；新用户没 secret 就永不触发 keychain。
//!
//! Backend 选择规则（sticky）：
//!   - 首次启动按运行期 `try_open()` 探测决定：keychain 可用 → keyring，不可用
//!     → file。结果写 `db.settings.master_key_backend` 标记。
//!   - 后续启动按标记走，**永不翻转**：曾经 keyring → 必须 keyring，keychain 不可
//!     用就硬 fail（避免 silently 切到 file 用新主密钥让旧密文全部解不开）。
//!   - 编译期 `#[cfg(...)]` 只是排除 iOS/Android（keyring crate 没这俩平台后端），
//!     **运行期是否选 keyring 由 `try_open()` 真探测决定**，不是看 OS。
//!
//! Service 名固定 `rssh`，account 命名规则全平台、CLI/GUI 共用：
//! - `cred:<credential_id>:secret`     凭证主 secret（密码或私钥 PEM）
//! - `setting:github_token`            GitHub PAT
//! - `setting:ai_<provider>_key`       BYOK API key
//!
//! 历史遗留：`cred:<credential_id>:passphrase` 曾用于存私钥 passphrase，
//! 已废弃 — 启动时统一清空（migration），新流程仅进程内缓存。

use std::path::Path;
use std::sync::Arc;

use serde_json::json;

use crate::db::{self, Db};
use crate::error::{AppError, AppResult};

pub mod crypto;
mod db_store;
mod hybrid_store;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod keyring_store;
mod master_key;

pub use db_store::DbStore;
pub use hybrid_store::HybridStore;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use keyring_store::KeyringStore;
pub use master_key::{FileMasterKey, KeyringMasterKey, MasterKeyBackend};

/// 持久化 backend 选择标记。db.settings 表里存储。
/// "keyring" / "file"。一旦写入永不修改（除非用户手动操作 DB）。
const BACKEND_MARKER: &str = "master_key_backend";
const BACKEND_KEYRING: &str = "keyring";
const BACKEND_FILE: &str = "file";

/// 服务名 — keychain 用，所有 rssh 数据都在这个 service 下。
pub const SERVICE: &str = "rssh";

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> AppResult<Option<String>>;
    fn set(&self, key: &str, value: &str) -> AppResult<()>;
    fn delete(&self, key: &str) -> AppResult<()>;
    fn backend_name(&self) -> &'static str;
}

/// 打开 SecretStore 系统 —— 返回组合对象：
///   - `store`：调用方用的统一 SecretStore（HybridStore，加密 DB 入口）
///   - `raw_keyring`：底层 keychain handle 作为 trait object，给 migration 用来读老
///     keychain 残留；keychain 不可用时为 None
///
/// migration::run_migrations 启动时被调一次（lib.rs setup / CLI ctx）。
///
/// **可能失败**：sticky backend 标记为 keyring 但 keychain 现在不可用时硬 fail，
/// 而不是 silently fallback 到 file（那会用新主密钥让旧密文全部解不开）。
pub struct SecretSystem {
    pub store: Arc<dyn SecretStore>,
    pub raw_keyring: Option<Arc<dyn SecretStore>>,
}

pub fn open(db: Arc<Db>, data_dir: &Path) -> AppResult<SecretSystem> {
    let db_store = Arc::new(DbStore::new(db.clone()));
    let recorded = db::settings::get(&db, BACKEND_MARKER)?;

    // 运行期探测：编译进 keyring crate 的平台 (mac/win/linux) + 真能 probe 写读
    let probed: Option<Arc<dyn SecretStore>> = probe_keyring();

    match (recorded.as_deref(), probed) {
        // ── sticky keyring + 可用 → 用 keyring ──
        (Some(BACKEND_KEYRING), Some(kr)) => {
            log::debug!("secret backend: keyring (sticky)");
            Ok(build_keyring_system(db.clone(), db_store, kr))
        }

        // ── sticky keyring + 不可用 → 硬 fail ──
        // 这条是数据安全护栏：之前主密钥写在 keychain 里，现在 keychain 拿不到，
        // 任何 secret 读写都会失败。silently fallback 到 file 会生成新主密钥，
        // 把之前所有 keychain 加密过的 DB 密文永久变废纸。
        (Some(BACKEND_KEYRING), None) => Err(AppError::other(
            "keychain_unavailable_but_required",
            json!({
                "hint": "之前选过系统 keychain 存主密钥，现在 keychain 不可用。\
                         请检查系统 keychain（macOS Keychain Access / Windows \
                         Credential Manager / Linux Secret Service）是否正常，\
                         然后重启 rssh。"
            }),
        )),

        // ── sticky file → 永远 file ──
        // raw_keyring 仍传 migration 用：如果用户当前 keychain 可用，可能有老
        // 版本残留的 keychain 数据要迁过来（v0.1.x 走过 keyring backend 但当时
        // 没 BACKEND_MARKER 标记，sticky 升级时记录的是 file 是另一种 case 不存
        // 在 — 仅升级路径会写 marker）。
        (Some(BACKEND_FILE), probed_for_migration) => {
            log::debug!("secret backend: file (sticky)");
            Ok(build_file_system(db_store, data_dir, probed_for_migration))
        }

        // ── 首次启动：按 probe 结果选 + 写 marker ──
        (None, Some(kr)) => {
            db::settings::set(&db, BACKEND_MARKER, BACKEND_KEYRING)?;
            log::info!("secret backend selected: keyring (first run)");
            Ok(build_keyring_system(db.clone(), db_store, kr))
        }
        (None, None) => {
            db::settings::set(&db, BACKEND_MARKER, BACKEND_FILE)?;
            log::warn!(
                "secret backend selected: file (first run, keychain unavailable); \
                 master key will be stored at <data_dir>/master.key"
            );
            Ok(build_file_system(db_store, data_dir, None))
        }

        // ── 未知 marker 值 → 硬 fail，让人来看 DB ──
        (Some(other), _) => Err(AppError::other(
            "master_key_backend_unknown",
            json!({ "value": other.to_string() }),
        )),
    }
}

/// 运行期探测系统 keychain。`#[cfg(...)]` 排除编译不进 keyring crate 的平台
/// （iOS / Android），其他平台靠 `try_open()` 真探测（写 probe key + 读回 + 删）。
fn probe_keyring() -> Option<Arc<dyn SecretStore>> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        keyring_store::try_open().map(|kr| {
            let arc: Arc<KeyringStore> = Arc::new(kr);
            arc as Arc<dyn SecretStore>
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

fn build_keyring_system(
    db: Arc<Db>,
    db_store: Arc<DbStore>,
    keyring: Arc<dyn SecretStore>,
) -> SecretSystem {
    // backend_label 直接透传 KeyringStore 的 OS-specific 名字（"macos-keychain"
    // 等），跟 PR #60 前 CLI/GUI 显示一致，不破坏 user-facing 字符串契约。
    let label = keyring.backend_name();
    // db 传给 KeyringMasterKey 做 SQLite IMMEDIATE 跨进程互斥（CLI+GUI 并发首次启
    // 动时序列化 master key 生成）。FileMasterKey 不需要，文件创建已经原子。
    let mk_backend: Arc<dyn MasterKeyBackend> =
        Arc::new(KeyringMasterKey::new(keyring.clone(), db));
    let store: Arc<dyn SecretStore> = Arc::new(HybridStore::new(db_store, mk_backend, label));
    SecretSystem {
        store,
        raw_keyring: Some(keyring),
    }
}

fn build_file_system(
    db_store: Arc<DbStore>,
    data_dir: &Path,
    probed_keyring: Option<Arc<dyn SecretStore>>,
) -> SecretSystem {
    let mk_backend: Arc<dyn MasterKeyBackend> = Arc::new(FileMasterKey::new(data_dir));
    let store: Arc<dyn SecretStore> = Arc::new(HybridStore::new(db_store, mk_backend, "file"));
    SecretSystem {
        store,
        raw_keyring: probed_keyring,
    }
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

#[cfg(test)]
mod tests {
    //! 这套测试覆盖 SecretStore 的"协议契约"——CLI 直接读 DB（AGENT.md P5），
    //! GUI 走 Tauri command 走同一套 helper。两端必须用**完全一致**的 key
    //! 字面量；任何动模板的修改都会让一端写、另一端读不到。
    //!
    //! 故意用字面量字符串断言（而不是 `format!("cred:{}:secret", id)` 自我比对）——
    //! 那种"测试 mirror 实现"的写法过不了变更，是无效测试。这里是真协议钉住。
    use super::*;

    // ── 服务名 ────────────────────────────────────────────────────

    #[test]
    fn service_name_is_rssh() {
        // keychain backend 用这个 service 名落键。CLI/GUI/任何外部工具
        // (Keychain Access.app / secret-tool) 都按 "rssh" 找。
        assert_eq!(SERVICE, "rssh");
    }

    // ── cred_secret_key 形状 ──────────────────────────────────────

    #[test]
    fn cred_secret_key_basic() {
        assert_eq!(cred_secret_key("abc"), "cred:abc:secret");
    }

    #[test]
    fn cred_secret_key_uuid_shape() {
        // 真实环境的 credential_id 是 UUID v4
        let k = cred_secret_key("550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(k, "cred:550e8400-e29b-41d4-a716-446655440000:secret");
    }

    #[test]
    fn cred_secret_key_empty_id() {
        // 不应 panic；保留实际行为以便 caller 显式知道
        assert_eq!(cred_secret_key(""), "cred::secret");
    }

    // ── cred_passphrase_key 形状 ──────────────────────────────────

    #[test]
    fn cred_passphrase_key_basic() {
        assert_eq!(cred_passphrase_key("abc"), "cred:abc:passphrase");
    }

    #[test]
    fn cred_passphrase_key_kept_for_legacy_cleanup() {
        // 该 key 已被弃用（启动时迁移代码 unconditional 清空）。
        // 模板任何变化（cred → credentials / passphrase → secret）都会让
        // 旧版残留 passphrase 不被清理，留 stale secret 在 keychain 里。
        let k = cred_passphrase_key("any-id");
        assert!(k.starts_with("cred:"));
        assert!(k.ends_with(":passphrase"));
    }

    // ── setting_key 形状 ──────────────────────────────────────────

    #[test]
    fn setting_key_basic() {
        assert_eq!(setting_key("github_token"), "setting:github_token");
    }

    #[test]
    fn setting_key_empty_name() {
        assert_eq!(setting_key(""), "setting:");
    }

    // ── 命名空间隔离 ──────────────────────────────────────────────

    #[test]
    fn cred_and_setting_namespaces_dont_collide() {
        // cred 与 setting 完全不交叉前缀
        assert!(cred_secret_key("x").starts_with("cred:"));
        assert!(setting_key("x").starts_with("setting:"));
        assert!(!cred_secret_key("x").starts_with("setting:"));
        assert!(!setting_key("x").starts_with("cred:"));
    }

    #[test]
    fn cred_secret_and_passphrase_unique_per_id() {
        // 同一个 cred id，secret 和 passphrase 必须落不同 key
        let id = "shared-cred-id";
        assert_ne!(cred_secret_key(id), cred_passphrase_key(id));
    }

    #[test]
    fn different_cred_ids_yield_different_keys() {
        // 同样后缀（secret），不同 id 必出不同 key
        assert_ne!(cred_secret_key("id-a"), cred_secret_key("id-b"));
        assert_ne!(cred_passphrase_key("id-a"), cred_passphrase_key("id-b"));
    }

    #[test]
    fn different_settings_yield_different_keys() {
        assert_ne!(setting_key("a"), setting_key("b"));
    }

    /// 已知锐边：id 含 ':' 时 key 会有 4+ 段。当前没人按段解 key（只 build
    /// 没 parse），所以不是 bug——但测试钉住"不会 silent 出歧义键"。
    /// credential_id 是 UUID v4 不含 ':'，setting name 是固定白名单不含 ':'，
    /// 生产场景触不到。改 caller 时如果有人允许 id 含 ':'，回头审这块。
    #[test]
    fn id_with_colon_produces_extra_segments() {
        let k = cred_secret_key("a:b");
        // 5 段：cred / a / b / secret —— 实际是 4 段（":" 切出）
        assert_eq!(k, "cred:a:b:secret");
        assert_eq!(k.matches(':').count(), 3);
    }

    // ── SECRET_SETTINGS 白名单稳定 ────────────────────────────────

    #[test]
    fn secret_settings_includes_github_token() {
        assert!(SECRET_SETTINGS.contains(&"github_token"));
    }

    #[test]
    fn secret_settings_is_non_empty() {
        // 防回归：未来谁误删空白名单，github_token 就会落 settings 表明文
        assert!(!SECRET_SETTINGS.is_empty());
    }

    // ── is_secret_setting：白名单严格匹配 ─────────────────────────

    #[test]
    fn is_secret_setting_known_keys_pass() {
        for &k in SECRET_SETTINGS {
            assert!(
                is_secret_setting(k),
                "whitelisted key {k:?} should be secret"
            );
        }
    }

    #[test]
    fn is_secret_setting_unknown_keys_rejected() {
        // 普通 settings 走 settings 表明文，不应被 is_secret_setting 误判
        assert!(!is_secret_setting("locale"));
        assert!(!is_secret_setting("appearance"));
        assert!(!is_secret_setting("theme"));
        assert!(!is_secret_setting("unknown"));
    }

    #[test]
    fn is_secret_setting_empty_rejected() {
        assert!(!is_secret_setting(""));
    }

    /// 必须**精确**匹配——不能被子串、前缀、后缀、含义相近的名字蒙混过关。
    /// 否则可能让某个看起来"像 token"的明文 setting 错走 secret 路径，
    /// 或者反过来让真 secret 漏判走明文。
    #[test]
    fn is_secret_setting_is_exact_match_not_substring() {
        // 子串：单独的 "github" 或 "_token" 不是 secret
        assert!(!is_secret_setting("github"));
        assert!(!is_secret_setting("_token"));
        assert!(!is_secret_setting("token"));

        // 前缀拓展
        assert!(!is_secret_setting("github_token_extra"));
        assert!(!is_secret_setting("github_token_v2"));

        // 后缀拓展
        assert!(!is_secret_setting("my_github_token"));
        assert!(!is_secret_setting("X_github_token"));

        // 含义相近但不在白名单
        assert!(!is_secret_setting("api_token"));
        assert!(!is_secret_setting("auth_token"));
        assert!(!is_secret_setting("github_pat"));
    }

    #[test]
    fn is_secret_setting_case_sensitive() {
        // 白名单字面 "github_token"——大小写变体一律不算
        assert!(!is_secret_setting("GITHUB_TOKEN"));
        assert!(!is_secret_setting("Github_Token"));
        assert!(!is_secret_setting("GitHub_Token"));
        assert!(!is_secret_setting("github_Token"));
    }

    #[test]
    fn is_secret_setting_whitespace_sensitive() {
        // " github_token" / "github_token " — 含前后空白都算不在白名单
        assert!(!is_secret_setting(" github_token"));
        assert!(!is_secret_setting("github_token "));
        assert!(!is_secret_setting("github_token\n"));
        assert!(!is_secret_setting("\tgithub_token"));
    }

    // ── 跨 helper 的不变量 ────────────────────────────────────────

    #[test]
    fn setting_key_for_secret_setting_routes_via_setting_namespace() {
        // SECRET_SETTINGS 里的 key 也走 setting:* 命名空间，
        // 不会变成 cred:* — 名空间不会因"是 secret"而切换。
        for &k in SECRET_SETTINGS {
            let full = setting_key(k);
            assert!(full.starts_with("setting:"));
            assert!(!full.starts_with("cred:"));
        }
    }

    #[test]
    fn key_helpers_are_pure_no_panic_on_typical_input() {
        // 跑一批典型 input，仅确认不 panic；具体形状由前面专测断言
        let ids = ["", "a", "550e8400-e29b-41d4-a716-446655440000", "with-dashes"];
        for id in ids {
            let _ = cred_secret_key(id);
            let _ = cred_passphrase_key(id);
        }
        let names = ["", "github_token", "locale", "theme"];
        for n in names {
            let _ = setting_key(n);
            let _ = is_secret_setting(n);
        }
    }
}
