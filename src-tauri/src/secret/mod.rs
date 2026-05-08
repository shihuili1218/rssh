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
