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
use crate::secret::crypto::is_encrypted_v1;
use crate::secret::{cred_passphrase_key, cred_secret_key, setting_key, SecretStore};

/// "全部完成"marker：含 keychain 迁移。两个阶段都跑过才写。
const MIGRATION_MARKER: &str = "migration_v1_unified_secret_storage";
/// 仅 DB 明文重加密完成 marker：不依赖 keyring，headless / Android / keychain
/// 临时不可用场景下也能写。避免每次启动全表扫 `secrets`。
const REENCRYPT_MARKER: &str = "migration_v1_db_plaintext_reencrypted";

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

    let mut report = Report::default();

    // ── 阶段 1：旧 DbStore 明文 → HybridStore 加密 ──
    // v0.1.x 时代 keychain 不可用的用户（Android / Linux headless）走 DbStore
    // fallback，把 secret 明文写进 DB.secrets。新 HybridStore 把同一张表当密文
    // 库用，明文条目 decrypt 会撞 "format_unknown"。一次性扫一遍重加密回去。
    //
    // 桌面用户 keychain 一直可用：DB.secrets 此前为空，扫描 0 条零开销。
    // **独立 marker**：headless 平台 raw_keyring 永远 None，全局 MIGRATION_MARKER
    // 不会写，否则每次启动都得 list_all 全表扫。本 marker 单独写在阶段 1 完成时，
    // 之后启动看见就跳过整个 list_all。
    if db::settings::get(db, REENCRYPT_MARKER)?.is_none() {
        for (key, value) in db::secret::list_all(db)? {
            if !is_encrypted_v1(&value) {
                // new_store.set 走 HybridStore：触发 master key lazy 生成 + 加密回写
                new_store.set(&key, &value)?;
                report.db_plaintext_reencrypted += 1;
            }
        }
        db::settings::set(db, REENCRYPT_MARKER, "1")?;
    }

    // keychain 不可用时（桌面 keychain 临时挂 / Android）：没旧 keychain 数据
    // 可迁，但**不写 MIGRATION_MARKER** —— 桌面 keychain 后续恢复时还能跑这次
    // 迁移。但 REENCRYPT_MARKER 已经在上面写过了，所以这次返回的成本是 2 次
    // settings::get 查询（~0.1ms），不再触发全表扫描。
    let Some(kr) = raw_keyring else {
        if report.any() {
            log::info!(
                "secret migration v1 partial (no keyring): db_plaintext_reencrypted={}",
                report.db_plaintext_reencrypted
            );
        }
        return Ok(());
    };

    // ── 不覆盖更新过的新值 ──
    // raw_keyring=None 那次启动用户可能已经在 HybridStore / DB.settings 上手动设
    // 过新值（API key 改了、token 重发了……）。后续启动 keychain 恢复，本迁移再
    // 跑时若无脑 `set()`，就把用户的新值覆盖成 keychain 里的旧值。
    // 策略：destination 已有值 → 跳过 set，但仍删 keychain 旧值清理残留。

    // ── 1+3. cred:<id>:secret 加密迁；cred:<id>:passphrase 清理 ──
    for cred in credential::list(db)? {
        let secret_key = cred_secret_key(&cred.id);
        if let Some(plaintext) = kr.get(&secret_key)? {
            if new_store.get(&secret_key)?.is_none() {
                // new_store.set 内部走 HybridStore：第一次写入时 lazy 触发主密钥生成
                new_store.set(&secret_key, &plaintext)?;
                report.creds_migrated += 1;
            } else {
                report.creds_skipped_newer += 1;
            }
            kr.delete(&secret_key)?;
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
            if new_store.get(&kc_key)?.is_none() {
                new_store.set(&kc_key, &plaintext)?;
                report.settings_secret_migrated += 1;
            } else {
                report.settings_secret_skipped_newer += 1;
            }
            kr.delete(&kc_key)?;
        }
    }

    // ── 2. 明文迁：行为偏好 → DB.settings 表（裸 key，跟 PR #59 后写入路径一致）──
    let plain_global = ["ai_provider", "ai_danger_mode"];
    let plain_per_provider_suffixes = ["model", "endpoint"];
    for raw in &plain_global {
        let kc_key = setting_key(raw);
        if let Some(value) = kr.get(&kc_key)? {
            if db::settings::get(db, raw)?.is_none() {
                db::settings::set(db, raw, &value)?;
                report.settings_plain_migrated += 1;
            } else {
                report.settings_plain_skipped_newer += 1;
            }
            kr.delete(&kc_key)?;
        }
    }
    for p in PROVIDERS_V1_10 {
        for suffix in &plain_per_provider_suffixes {
            let raw = format!("ai_{p}_{suffix}");
            let kc_key = setting_key(&raw);
            if let Some(value) = kr.get(&kc_key)? {
                if db::settings::get(db, &raw)?.is_none() {
                    db::settings::set(db, &raw, &value)?;
                    report.settings_plain_migrated += 1;
                } else {
                    report.settings_plain_skipped_newer += 1;
                }
                kr.delete(&kc_key)?;
            }
        }
    }

    if report.any() {
        log::info!(
            "secret migration v1 done: creds={} (skipped_newer={}) passphrases_cleared={} settings_secret={} (skipped_newer={}) settings_plain={} (skipped_newer={}) db_plaintext_reencrypted={}",
            report.creds_migrated,
            report.creds_skipped_newer,
            report.passphrases_cleared,
            report.settings_secret_migrated,
            report.settings_secret_skipped_newer,
            report.settings_plain_migrated,
            report.settings_plain_skipped_newer,
            report.db_plaintext_reencrypted,
        );
    }

    db::settings::set(db, MIGRATION_MARKER, "1")?;
    Ok(())
}

#[derive(Default)]
struct Report {
    creds_migrated: u32,
    /// keychain 旧值存在但 HybridStore 已有新值 → 不覆盖用户更新，仅清 keychain。
    creds_skipped_newer: u32,
    passphrases_cleared: u32,
    settings_secret_migrated: u32,
    settings_secret_skipped_newer: u32,
    settings_plain_migrated: u32,
    settings_plain_skipped_newer: u32,
    /// v0.1.x 时代走 DbStore fallback 的用户：DB.secrets 里有明文，这次重加密。
    db_plaintext_reencrypted: u32,
}

impl Report {
    fn any(&self) -> bool {
        self.creds_migrated
            + self.creds_skipped_newer
            + self.passphrases_cleared
            + self.settings_secret_migrated
            + self.settings_secret_skipped_newer
            + self.settings_plain_migrated
            + self.settings_plain_skipped_newer
            + self.db_plaintext_reencrypted
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
        let new_store = Arc::new(HybridStore::new(real_db_store, mk, "file"));
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
    fn no_keyring_skips_without_marker() {
        // keychain 不可用 + 没旧 DbStore 明文 → 无操作，**不写 marker**。
        // 这样桌面 keychain 临时挂的下次启动还能跑迁移，不会丢老 keychain 数据。
        let (db, _kr, ns, _tmp) = make_env();
        run(&db, None, ns.as_ref()).unwrap();
        // marker 不写 — 让下次启动再试
        assert!(db::settings::get(&db, MIGRATION_MARKER).unwrap().is_none());
    }

    #[test]
    fn db_plaintext_reencrypted_then_reencrypt_marker_written() {
        // 模拟 v0.1.x 走 DbStore fallback 的用户：DB.secrets 直接有明文
        let (db, _kr, ns, _tmp) = make_env();
        // 绕过 HybridStore 直接写明文进 DB.secrets，模拟旧 DbStore 行为
        db::secret::set(&db, "cred:legacy:secret", "raw-pem-plaintext").unwrap();

        run(&db, None, ns.as_ref()).unwrap();

        // 明文已被加密重写：DB.secrets 里的 raw 值是 enc:v1: 密文，HybridStore 读出明文
        let raw = db::secret::get(&db, "cred:legacy:secret").unwrap().unwrap();
        assert!(raw.starts_with("enc:v1:"), "DB.secrets 必须是密文，实际: {raw}");
        assert_eq!(
            ns.get("cred:legacy:secret").unwrap().as_deref(),
            Some("raw-pem-plaintext")
        );
        // no-keyring：REENCRYPT_MARKER 写入（阶段 1 已完成），但 MIGRATION_MARKER
        // 不写（阶段 2 keychain 迁移没跑）。下次启动跳过 list_all 全表扫。
        assert_eq!(
            db::settings::get(&db, REENCRYPT_MARKER).unwrap().as_deref(),
            Some("1")
        );
        assert!(db::settings::get(&db, MIGRATION_MARKER).unwrap().is_none());
    }

    #[test]
    fn reencrypt_marker_skips_second_full_scan() {
        // 头部 marker 已写后，后续启动**不再扫**全表 —— 即使 DB.secrets 里又被
        // 写进明文（理论上不该有，但作为优化的护栏测试）
        let (db, _kr, ns, _tmp) = make_env();
        run(&db, None, ns.as_ref()).unwrap();
        assert_eq!(
            db::settings::get(&db, REENCRYPT_MARKER).unwrap().as_deref(),
            Some("1")
        );

        // 第二次 run 之前手工塞一条明文进 DB.secrets：如果阶段 1 仍在跑就会被
        // 加密；marker 起作用就被跳过保留原样
        db::secret::set(&db, "leaked", "should-stay-plaintext").unwrap();
        run(&db, None, ns.as_ref()).unwrap();
        let raw = db::secret::get(&db, "leaked").unwrap().unwrap();
        assert_eq!(
            raw, "should-stay-plaintext",
            "REENCRYPT_MARKER 应让阶段 1 跳过，明文不被重写"
        );
    }

    #[test]
    fn db_plaintext_reencrypted_with_keyring() {
        // 有 keychain 的情况下，DB.secrets 明文也得 re-encrypt
        let (db, kr, ns, _tmp) = make_env();
        db::secret::set(&db, "setting:github_token", "ghp_plain_old").unwrap();

        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();

        // 重加密：DB raw 值是密文，HybridStore 读出原明文
        let raw = db::secret::get(&db, "setting:github_token").unwrap().unwrap();
        assert!(raw.starts_with("enc:v1:"));
        assert_eq!(
            ns.get("setting:github_token").unwrap().as_deref(),
            Some("ghp_plain_old")
        );
        // 有 keyring → marker 写入
        assert_eq!(db::settings::get(&db, MIGRATION_MARKER).unwrap().as_deref(), Some("1"));
    }

    #[test]
    fn already_encrypted_db_values_left_alone() {
        // 之前已经加密过的（同 PR 内 hybrid.set 写入）再跑迁移不重复加密
        let (db, kr, ns, _tmp) = make_env();
        ns.set("setting:github_token", "ghp_new").unwrap();
        let raw_before = db::secret::get(&db, "setting:github_token").unwrap().unwrap();

        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();

        // raw 没变（is_encrypted_v1 跳过 re-encrypt）
        let raw_after = db::secret::get(&db, "setting:github_token").unwrap().unwrap();
        assert_eq!(raw_before, raw_after);
        assert_eq!(ns.get("setting:github_token").unwrap().as_deref(), Some("ghp_new"));
    }

    #[test]
    fn empty_keyring_writes_marker_only() {
        // keychain 可用但里面啥也没有（新装用户）
        let (db, kr, ns, _tmp) = make_env();
        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();
        assert_eq!(db::settings::get(&db, MIGRATION_MARKER).unwrap().as_deref(), Some("1"));
    }

    #[test]
    fn does_not_overwrite_user_new_cred_secret() {
        // 场景：raw_keyring=None 时用户用 HybridStore 改过 cred 密码（new value 在
        // new_store）；之后 keychain 恢复，迁移再跑，绝不能用 keychain 旧值覆盖。
        let (db, kr, ns, _tmp) = make_env();
        add_cred(&db, "x");
        // keychain 残留旧值
        kr.set(&cred_secret_key("x"), "old-keychain-pem").unwrap();
        // 用户已经在 HybridStore 设了新值
        ns.set(&cred_secret_key("x"), "new-user-pem").unwrap();

        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();

        // new_store 仍是新值，不被覆盖
        assert_eq!(ns.get(&cred_secret_key("x")).unwrap().as_deref(), Some("new-user-pem"));
        // keychain 旧值被清掉（清理依然进行）
        assert!(kr.get(&cred_secret_key("x")).unwrap().is_none());
    }

    #[test]
    fn does_not_overwrite_user_new_github_token() {
        let (db, kr, ns, _tmp) = make_env();
        kr.set(&setting_key("github_token"), "ghp_old").unwrap();
        ns.set(&setting_key("github_token"), "ghp_new").unwrap();

        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();

        assert_eq!(ns.get(&setting_key("github_token")).unwrap().as_deref(), Some("ghp_new"));
        assert!(kr.get(&setting_key("github_token")).unwrap().is_none());
    }

    #[test]
    fn does_not_overwrite_user_new_plain_setting() {
        let (db, kr, ns, _tmp) = make_env();
        kr.set(&setting_key("ai_provider"), "openai").unwrap();
        db::settings::set(&db, "ai_provider", "deepseek").unwrap();

        run(&db, Some(kr.as_ref()), ns.as_ref()).unwrap();

        // 用户后改的 deepseek 留下
        assert_eq!(db::settings::get(&db, "ai_provider").unwrap().as_deref(), Some("deepseek"));
        assert!(kr.get(&setting_key("ai_provider")).unwrap().is_none());
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
