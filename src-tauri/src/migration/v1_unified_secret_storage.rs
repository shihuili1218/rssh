//! Migration v1：统一 secret 存储 keychain → HybridStore（加密 DB）+ 顺便清旧 passphrase。
//!
//! 触发：所有从 v0.1.10 / PR #58 / PR #59 等旧版本升上来的用户。
//! Marker：`migration_v1_unified_secret_storage`（settings 表）。
//!
//! 迁移内容：
//!   1. 加密迁移（旧 keychain → 新 HybridStore，密文入 DB.secrets 表）：
//!        cred:<id>:secret              SSH 私钥 / 密码（解决 Windows 2560B 限制的真正动因）
//!        setting:github_token          GitHub PAT
//!        setting:ai_<p>_key            BYOK API key × 4 provider
//!   2. 明文迁移（旧 keychain → DB.settings 表，跟 PR #59 后的写入路径对齐）：
//!        setting:ai_provider           当前激活 provider
//!        setting:ai_danger_mode        危险模式总闸
//!        setting:ai_<p>_model          各 provider 模型 × 4
//!        setting:ai_<p>_endpoint       各 provider 自定义 endpoint × 4
//!   3. 清理：cred:<id>:passphrase     已废弃多版本，无条件 delete
//!
//! 不迁：setting:ai_auto_*（PR #58 才引入，v0.1.10 用户没设过；本次发布的参数）
//!
//! keychain 不可用平台（Android / headless）：本就没旧 keychain 数据，
//! 写 marker 跳过即可；HybridStore 走 FileMasterKey 路径正常工作。

use crate::db::{self, credential, Db};
use crate::error::AppResult;
use crate::secret::{cred_passphrase_key, cred_secret_key, setting_key, SecretStore};

const MIGRATION_MARKER: &str = "migration_v1_unified_secret_storage";

/// v0.1.10 时代支持的 BYOK provider 命名。命名跟 `ai_<provider>_*` 的 provider
/// 段一致；新增 provider 已经走新存储路径不需要迁。
const PROVIDERS_V1_10: &[&str] = &["anthropic", "openai", "deepseek", "glm"];

pub fn run(
    db: &Db,
    raw_keyring: Option<&dyn SecretStore>,
    new_store: &dyn SecretStore,
) -> AppResult<()> {
    if db::settings::get(db, MIGRATION_MARKER)?.is_some() {
        return Ok(());
    }

    // keychain 不可用：直接写 marker。这种用户 v0.1.10 时代就走 DbStore（明文）
    // 没旧 keychain 数据，本次 HybridStore 切换无感知。
    let Some(kr) = raw_keyring else {
        db::settings::set(db, MIGRATION_MARKER, "1")?;
        return Ok(());
    };

    let mut report = Report::default();

    // ── 1+3. cred:<id>:secret 加密迁；cred:<id>:passphrase 清理 ──
    for cred in credential::list(db)? {
        let secret_key = cred_secret_key(&cred.id);
        if let Some(plaintext) = kr.get(&secret_key)? {
            // new_store.set 内部走 HybridStore：第一次写入时 lazy 触发主密钥生成
            new_store.set(&secret_key, &plaintext)?;
            kr.delete(&secret_key)?;
            report.creds_migrated += 1;
        }
        let pp_key = cred_passphrase_key(&cred.id);
        if kr.get(&pp_key)?.is_some() {
            kr.delete(&pp_key)?;
            report.passphrases_cleared += 1;
        }
    }

    // ── 1. 其他 secret 加密迁：github_token + 各 provider api_key ──
    let mut secret_settings: Vec<String> = vec!["github_token".into()];
    for p in PROVIDERS_V1_10 {
        secret_settings.push(format!("ai_{p}_key"));
    }
    for raw in &secret_settings {
        let kc_key = setting_key(raw);
        if let Some(plaintext) = kr.get(&kc_key)? {
            new_store.set(&kc_key, &plaintext)?;
            kr.delete(&kc_key)?;
            report.settings_secret_migrated += 1;
        }
    }

    // ── 2. 明文迁：行为偏好 → DB.settings 表（裸 key，跟 PR #59 后写入路径一致）──
    let plain_global = ["ai_provider", "ai_danger_mode"];
    let plain_per_provider_suffixes = ["model", "endpoint"];
    for raw in &plain_global {
        let kc_key = setting_key(raw);
        if let Some(value) = kr.get(&kc_key)? {
            db::settings::set(db, raw, &value)?;
            kr.delete(&kc_key)?;
            report.settings_plain_migrated += 1;
        }
    }
    for p in PROVIDERS_V1_10 {
        for suffix in &plain_per_provider_suffixes {
            let raw = format!("ai_{p}_{suffix}");
            let kc_key = setting_key(&raw);
            if let Some(value) = kr.get(&kc_key)? {
                db::settings::set(db, &raw, &value)?;
                kr.delete(&kc_key)?;
                report.settings_plain_migrated += 1;
            }
        }
    }

    if report.any() {
        log::info!(
            "secret migration v1 done: creds={} passphrases_cleared={} settings_secret={} settings_plain={}",
            report.creds_migrated,
            report.passphrases_cleared,
            report.settings_secret_migrated,
            report.settings_plain_migrated,
        );
    }

    db::settings::set(db, MIGRATION_MARKER, "1")?;
    Ok(())
}

#[derive(Default)]
struct Report {
    creds_migrated: u32,
    passphrases_cleared: u32,
    settings_secret_migrated: u32,
    settings_plain_migrated: u32,
}

impl Report {
    fn any(&self) -> bool {
        self.creds_migrated
            + self.passphrases_cleared
            + self.settings_secret_migrated
            + self.settings_plain_migrated
            > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Credential, CredentialType};
    use crate::secret::{DbStore, FileMasterKey, HybridStore, MasterKeyBackend};
    use std::sync::Arc;

    /// 建测试环境：in-memory DB + 用 DbStore 模拟"旧 keychain"（持有遗留明文数据）+
    /// HybridStore 作新 store（master key 落临时文件，避免触碰真 keychain）。
    fn make_env() -> (Arc<Db>, Arc<DbStore>, Arc<HybridStore>, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = Arc::new(Db::open_in_memory().unwrap());
        // mock keychain：用另一个独立 DbStore 模拟旧 keychain 的明文键值存储
        // 真实 KeyringStore 在桌面平台跟系统 keychain 通信，单测不碰它
        let mock_keyring_db = Arc::new(Db::open_in_memory().unwrap());
        let mock_keyring = Arc::new(DbStore::new(mock_keyring_db));
        // 新 store：HybridStore，主密钥落临时文件 (FileMasterKey)
        let real_db_store = Arc::new(DbStore::new(db.clone()));
        let mk: Arc<dyn MasterKeyBackend> =
            Arc::new(FileMasterKey::with_path(tmp.path().join("mk")));
        let new_store = Arc::new(HybridStore::new(real_db_store, mk));
        (db, mock_keyring, new_store, tmp)
    }

    fn add_cred(db: &Db, id: &str) {
        credential::insert(
            db,
            &Credential {
                id: id.into(),
                name: format!("cred-{id}"),
                username: "u".into(),
                credential_type: CredentialType::Password,
                secret: None,
                save_to_remote: false,
            },
        )
        .unwrap();
    }

    #[test]
    fn idempotent_marker_skips_second_run() {
        let (db, kr, ns, _tmp) = make_env();
        // 给 mock keychain 喂一个 cred secret
        add_cred(&db, "id1");
        kr.set(&cred_secret_key("id1"), "pem-v1").unwrap();

        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();
        // 第一次跑完：新 store 拿到 + 老 keychain 没了 + marker 写入
        assert_eq!(ns.get(&cred_secret_key("id1")).unwrap().as_deref(), Some("pem-v1"));
        assert!(kr.get(&cred_secret_key("id1")).unwrap().is_none());
        assert_eq!(db::settings::get(&db, MIGRATION_MARKER).unwrap().as_deref(), Some("1"));

        // 第二次跑：mock keychain 再放一条新数据，迁移应该跳过不动它（已 marker）
        kr.set(&cred_secret_key("id1"), "should-not-be-touched").unwrap();
        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();
        assert_eq!(
            kr.get(&cred_secret_key("id1")).unwrap().as_deref(),
            Some("should-not-be-touched"),
            "二次调用必须 skip，不动新喂的数据"
        );
    }

    #[test]
    fn migrates_cred_secret_to_new_store() {
        let (db, kr, ns, _tmp) = make_env();
        add_cred(&db, "a");
        add_cred(&db, "b");
        kr.set(&cred_secret_key("a"), "pem-A").unwrap();
        kr.set(&cred_secret_key("b"), "pass-B").unwrap();

        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();

        assert_eq!(ns.get(&cred_secret_key("a")).unwrap().as_deref(), Some("pem-A"));
        assert_eq!(ns.get(&cred_secret_key("b")).unwrap().as_deref(), Some("pass-B"));
        // 老 keychain 已清
        assert!(kr.get(&cred_secret_key("a")).unwrap().is_none());
        assert!(kr.get(&cred_secret_key("b")).unwrap().is_none());
    }

    #[test]
    fn clears_legacy_passphrase() {
        let (db, kr, ns, _tmp) = make_env();
        add_cred(&db, "c");
        kr.set(&cred_passphrase_key("c"), "stale-passphrase").unwrap();

        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();

        assert!(kr.get(&cred_passphrase_key("c")).unwrap().is_none());
        // passphrase 不迁到新 store，纯删除
        assert!(ns.get(&cred_passphrase_key("c")).unwrap().is_none());
    }

    #[test]
    fn migrates_github_token_and_api_keys_encrypted() {
        let (db, kr, ns, _tmp) = make_env();
        kr.set(&setting_key("github_token"), "ghp_abc").unwrap();
        kr.set(&setting_key("ai_anthropic_key"), "sk-ant-xxx").unwrap();
        kr.set(&setting_key("ai_openai_key"), "sk-openai-yyy").unwrap();

        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();

        // 这些是 secret，进新 store（加密 DB）
        assert_eq!(ns.get(&setting_key("github_token")).unwrap().as_deref(), Some("ghp_abc"));
        assert_eq!(
            ns.get(&setting_key("ai_anthropic_key")).unwrap().as_deref(),
            Some("sk-ant-xxx")
        );
        assert!(kr.get(&setting_key("github_token")).unwrap().is_none());
    }

    #[test]
    fn migrates_plain_settings_to_db_settings_table() {
        let (db, kr, ns, _tmp) = make_env();
        kr.set(&setting_key("ai_provider"), "anthropic").unwrap();
        kr.set(&setting_key("ai_danger_mode"), "1").unwrap();
        kr.set(&setting_key("ai_anthropic_model"), "claude-sonnet-4-6").unwrap();
        kr.set(&setting_key("ai_anthropic_endpoint"), "https://api.example.com").unwrap();

        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();

        // 这些是行为偏好，进 DB settings 表（明文裸 key）
        assert_eq!(db::settings::get(&db, "ai_provider").unwrap().as_deref(), Some("anthropic"));
        assert_eq!(db::settings::get(&db, "ai_danger_mode").unwrap().as_deref(), Some("1"));
        assert_eq!(
            db::settings::get(&db, "ai_anthropic_model").unwrap().as_deref(),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(
            db::settings::get(&db, "ai_anthropic_endpoint").unwrap().as_deref(),
            Some("https://api.example.com")
        );
        // 老 keychain 已清
        assert!(kr.get(&setting_key("ai_provider")).unwrap().is_none());
    }

    #[test]
    fn no_keyring_writes_marker_only() {
        // 模拟 keychain 不可用的平台（Android / headless）
        let (db, _kr, ns, _tmp) = make_env();
        run(&db, None, ns.as_ref()).unwrap();
        // marker 已写
        assert_eq!(db::settings::get(&db, MIGRATION_MARKER).unwrap().as_deref(), Some("1"));
    }

    #[test]
    fn empty_keyring_writes_marker_only() {
        // keychain 可用但里面啥也没有（新装用户）
        let (db, kr, ns, _tmp) = make_env();
        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();
        assert_eq!(db::settings::get(&db, MIGRATION_MARKER).unwrap().as_deref(), Some("1"));
    }

    #[test]
    fn auto_settings_not_migrated() {
        // PR #58 auto_* 不在迁移清单
        let (db, kr, ns, _tmp) = make_env();
        kr.set(&setting_key("ai_auto_run_command"), "1").unwrap();
        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();
        // 没迁到任何地方
        assert!(db::settings::get(&db, "ai_auto_run_command").unwrap().is_none());
        // 老 keychain 残留依然在（无害但脏）—— 这是 Lord 明确决定的：不迁
        assert_eq!(kr.get(&setting_key("ai_auto_run_command")).unwrap().as_deref(), Some("1"));
    }
}
