use serde::Serialize;
use serde_json::json;
use tauri::State;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::models::{Credential, CredentialType, Profile};
use crate::secret::{cred_secret_key, SecretStore};
use crate::ssh::config::SshConfigEntry;
use crate::state::AppState;

#[tauri::command]
pub fn list_profiles(state: State<AppState>) -> Result<Vec<Profile>, AppError> {
    crate::db::profile::list(&state.db)
}

#[tauri::command]
pub fn get_profile(state: State<AppState>, id: String) -> Result<Profile, AppError> {
    crate::db::profile::get(&state.db, &id)
}

#[tauri::command]
pub fn create_profile(state: State<AppState>, profile: Profile) -> Result<(), AppError> {
    crate::db::profile::insert(&state.db, &profile)
}

#[tauri::command]
pub fn update_profile(state: State<AppState>, profile: Profile) -> Result<(), AppError> {
    crate::db::profile::update(&state.db, &profile)
}

#[tauri::command]
pub fn delete_profile(state: State<AppState>, id: String) -> Result<(), AppError> {
    crate::db::profile::delete(&state.db, &id)
}

// ---------------------------------------------------------------------------
// Credentials — secret 走 SecretStore，metadata 走 DB
// 私钥 passphrase 不再持久化：连接时终端内交互输入，仅进程内缓存。
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_credentials(state: State<AppState>) -> Result<Vec<Credential>, AppError> {
    // 列表场景不返回 secret，避免无谓 keychain 查询
    crate::db::credential::list(&state.db)
}

#[tauri::command]
pub fn get_credential(state: State<AppState>, id: String) -> Result<Credential, AppError> {
    let mut cred = crate::db::credential::get(&state.db, &id)?;
    cred.secret = state.secret_store.get(&cred_secret_key(&id))?;
    Ok(cred)
}

#[tauri::command]
pub fn create_credential(state: State<AppState>, credential: Credential) -> Result<(), AppError> {
    crate::db::credential::insert(&state.db, &credential)?;
    save_credential_secrets(&state, &credential)
}

#[tauri::command]
pub fn update_credential(state: State<AppState>, credential: Credential) -> Result<(), AppError> {
    crate::db::credential::update(&state.db, &credential)?;
    save_credential_secrets(&state, &credential)
}

#[tauri::command]
pub fn delete_credential(state: State<AppState>, id: String) -> Result<(), AppError> {
    crate::db::credential::delete(&state.db, &id)?;
    state.secret_store.delete(&cred_secret_key(&id))?;
    Ok(())
}

fn save_credential_secrets(state: &State<AppState>, c: &Credential) -> Result<(), AppError> {
    let secret_key = cred_secret_key(&c.id);
    match c.secret.as_deref() {
        Some(s) if !s.is_empty() => state.secret_store.set(&secret_key, s)?,
        _ => state.secret_store.delete(&secret_key)?,
    }
    Ok(())
}

#[tauri::command]
pub fn import_ssh_config(content: String) -> Vec<SshConfigEntry> {
    crate::ssh::config::parse(&content)
}

// ---------------------------------------------------------------------------
// SSH config 自动读取 + 选择性导入
// ---------------------------------------------------------------------------

/// 读 `~/.ssh/config` 并解析。文件不存在 → 返回空 Vec（不是 error，正常场景）。
/// IO 错误（权限不足等）→ 报 AppError::Io，让前端显示。
#[tauri::command]
pub fn read_ssh_config_default() -> AppResult<Vec<SshConfigEntry>> {
    let home =
        dirs::home_dir().ok_or_else(|| AppError::other("home_dir_unavailable", json!({})))?;
    let path = home.join(".ssh").join("config");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(crate::ssh::config::parse(&content))
}

/// 单条导入失败信息：哪个 host 的哪步出错。前端按表格展示。
#[derive(Debug, Clone, Serialize)]
pub struct SshImportError {
    pub host_alias: String,
    pub kind: &'static str, // "read_key" | "credential" | "credential_secret" | "profile"
    pub code: String,
}

#[derive(Debug, Default, Serialize)]
pub struct SshImportResult {
    pub profiles_created: usize,
    pub credentials_created: usize,
    pub errors: Vec<SshImportError>,
}

/// 把用户勾选的 SSH config entries 落库为 Profile + Credential。
///
/// 凭证去重：按 `(username, identity_file)` 二元组。同一私钥被不同 user 共享时
/// 会建多个 Credential（rssh 数据模型里 username 是 Credential 固有字段）。
///
/// 映射规则（每个 profile 必须引用一个真实 Credential —— 不再有 credential_id 空串占位）：
/// - `User=X, IdentityFile=Y`  → CredentialType::Key，secret = 读 Y 的 PEM 内容
/// - `User=X, IdentityFile=∅`  → CredentialType::Agent（让 ssh-agent / 默认私钥处理）
/// - `User=∅, IdentityFile=Y`  → CredentialType::Key，username = 系统当前用户
/// - `User=∅, IdentityFile=∅`  → CredentialType::Agent，username = 系统当前用户
///
/// 凭证解析失败（如私钥文件不存在）→ 不建 profile，错误记入 `errors`。
/// 数据一致性优先于"尽量多建 profile"——避免 DB 里出现引用不到 cred 的脏 profile。
///
/// ProxyJump 暂不自动连接（需要解析跳板别名 → profile_id 的双 pass，留给后续）。
#[tauri::command]
pub fn import_ssh_entries(
    state: State<'_, AppState>,
    entries: Vec<SshConfigEntry>,
) -> Result<SshImportResult, AppError> {
    do_import_ssh_entries(&state.db, state.secret_store.as_ref(), entries)
}

/// `import_ssh_entries` 的核心实现，独立于 tauri State 以便单元测试。
pub fn do_import_ssh_entries(
    db: &Db,
    ss: &dyn SecretStore,
    entries: Vec<SshConfigEntry>,
) -> Result<SshImportResult, AppError> {
    use std::collections::HashMap;

    let mut cred_cache: HashMap<(String, String), String> = HashMap::new();
    let mut result = SshImportResult::default();

    for entry in entries {
        // 凭证 resolve 失败（私钥读不到等）→ 不建 profile。错误已记到 result.errors，
        // 用户看 errors 报告决定怎么修——避免在 DB 里留下引用不到 cred 的脏 profile。
        let cred_id = match resolve_credential_id(db, ss, &entry, &mut cred_cache, &mut result) {
            Some(id) => id,
            None => continue,
        };

        let host = if entry.hostname.is_empty() {
            entry.host_alias.clone()
        } else {
            entry.hostname.clone()
        };
        let profile = Profile {
            id: uuid::Uuid::new_v4().to_string(),
            name: entry.host_alias.clone(),
            host,
            port: entry.port,
            credential_id: cred_id,
            bastion_profile_id: None,
            init_command: None,
            group_id: None,
        };
        if let Err(e) = crate::db::profile::insert(db, &profile) {
            result.errors.push(SshImportError {
                host_alias: entry.host_alias.clone(),
                kind: "profile",
                code: e.code().to_string(),
            });
            continue;
        }
        result.profiles_created += 1;
    }

    Ok(result)
}

/// 推导/创建 entry 对应的 credential id；按 (username, identity_file) 去重。
/// 失败时记录 errors 并返回 None，调用方据此跳过 profile 创建。
/// 任何 entry 都至少建一个 cred（最少 Agent + 系统当前用户）—— 不留 credential_id 空串。
fn resolve_credential_id(
    db: &Db,
    ss: &dyn SecretStore,
    entry: &SshConfigEntry,
    cache: &mut std::collections::HashMap<(String, String), String>,
    result: &mut SshImportResult,
) -> Option<String> {
    let username = entry.user.clone().unwrap_or_else(current_system_user);

    let cache_key = (
        username.clone(),
        entry.identity_file.clone().unwrap_or_default(),
    );
    if let Some(id) = cache.get(&cache_key) {
        return Some(id.clone());
    }

    let cred_id = match entry.identity_file.as_deref() {
        Some(file) => {
            let pem = match std::fs::read_to_string(file) {
                Ok(s) => s,
                Err(e) => {
                    result.errors.push(SshImportError {
                        host_alias: entry.host_alias.clone(),
                        kind: "read_key",
                        code: format!("io:{}", e.kind()),
                    });
                    return None;
                }
            };
            let cred = Credential {
                id: uuid::Uuid::new_v4().to_string(),
                name: format!("{}@{}", username, file_basename(file)),
                username: username.clone(),
                credential_type: CredentialType::Key,
                secret: Some(pem.clone()),
                save_to_remote: false,
            };
            if let Err(e) = crate::db::credential::insert(db, &cred) {
                result.errors.push(SshImportError {
                    host_alias: entry.host_alias.clone(),
                    kind: "credential",
                    code: e.code().to_string(),
                });
                return None;
            }
            if let Err(e) = ss.set(&cred_secret_key(&cred.id), &pem) {
                result.errors.push(SshImportError {
                    host_alias: entry.host_alias.clone(),
                    kind: "credential_secret",
                    code: e.code().to_string(),
                });
                // secret 写失败 → 凭证元数据已落，但运行时会 ssh_privkey_missing。
                // 仍然返回 cred id，让 profile 关联——用户能在凭证编辑器里补救。
            }
            cred.id
        }
        None => {
            // 有 user 没 file：Agent 凭证
            let cred = Credential {
                id: uuid::Uuid::new_v4().to_string(),
                name: format!("{}@ssh-agent", username),
                username: username.clone(),
                credential_type: CredentialType::Agent,
                secret: None,
                save_to_remote: false,
            };
            if let Err(e) = crate::db::credential::insert(db, &cred) {
                result.errors.push(SshImportError {
                    host_alias: entry.host_alias.clone(),
                    kind: "credential",
                    code: e.code().to_string(),
                });
                return None;
            }
            cred.id
        }
    };

    cache.insert(cache_key, cred_id.clone());
    result.credentials_created += 1;
    Some(cred_id)
}

fn current_system_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "root".to_string())
}

fn file_basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    //! `do_import_ssh_entries` 的核心契约：
    //! - 凭证按 (username, identity_file) 二元组去重，**不**只按 file 去重
    //!   （Linus 拍板：不同 user 同私钥要建两个 cred —— rssh 数据模型里
    //!    username 是 Credential 的固有字段）。
    //! - 每个 profile 必须引用一个真实 Credential（无 credential_id 空串占位）。
    //! - 私钥读取失败 → 整个 entry 跳过（profile 不建），错误记入 errors。
    //! - 没 user 也没 file 的 entry → 建 Agent cred（username = 系统当前用户）+ profile。
    use super::*;
    use crate::db::Db;
    use crate::secret::SecretStore;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// 进程内 SecretStore，用于测试。不走 keychain，不写 DB 表。
    #[derive(Default)]
    struct MemStore {
        inner: Mutex<HashMap<String, String>>,
    }

    impl SecretStore for MemStore {
        fn get(&self, key: &str) -> AppResult<Option<String>> {
            Ok(self.inner.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> AppResult<()> {
            self.inner
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, key: &str) -> AppResult<()> {
            self.inner.lock().unwrap().remove(key);
            Ok(())
        }
        fn backend_name(&self) -> &'static str {
            "mem"
        }
    }

    fn fixture() -> (Db, MemStore, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        (db, MemStore::default(), dir)
    }

    fn entry(host: &str, user: Option<&str>, file: Option<&str>) -> SshConfigEntry {
        SshConfigEntry {
            host_alias: host.to_string(),
            hostname: format!("{host}.example"),
            port: 22,
            user: user.map(String::from),
            identity_file: file.map(String::from),
            proxy_jump: None,
        }
    }

    fn write_key(dir: &TempDir, name: &str, pem: &str) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, pem).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn same_user_same_file_share_one_credential() {
        let (db, ss, dir) = fixture();
        let key = write_key(&dir, "id_rsa", "PEM-A");
        let res = do_import_ssh_entries(
            &db,
            &ss,
            vec![
                entry("a", Some("alice"), Some(&key)),
                entry("b", Some("alice"), Some(&key)),
                entry("c", Some("alice"), Some(&key)),
            ],
        )
        .unwrap();
        assert_eq!(res.profiles_created, 3);
        assert_eq!(res.credentials_created, 1);
        assert!(res.errors.is_empty());
        assert_eq!(crate::db::credential::list(&db).unwrap().len(), 1);
        assert_eq!(crate::db::profile::list(&db).unwrap().len(), 3);
    }

    #[test]
    fn different_users_same_file_get_separate_credentials() {
        // Linus 的关键约束：username 在 Credential 上，不同 user 必须分开
        let (db, ss, dir) = fixture();
        let key = write_key(&dir, "shared_key", "PEM-shared");
        let res = do_import_ssh_entries(
            &db,
            &ss,
            vec![
                entry("h1", Some("alice"), Some(&key)),
                entry("h2", Some("bob"), Some(&key)),
                entry("h3", Some("alice"), Some(&key)),
            ],
        )
        .unwrap();
        assert_eq!(res.profiles_created, 3);
        assert_eq!(res.credentials_created, 2);
        let creds = crate::db::credential::list(&db).unwrap();
        let users: std::collections::HashSet<_> =
            creds.iter().map(|c| c.username.clone()).collect();
        assert_eq!(users, ["alice".into(), "bob".into()].into_iter().collect());
    }

    #[test]
    fn user_without_identity_file_creates_agent_credential() {
        let (db, ss, _dir) = fixture();
        let res = do_import_ssh_entries(
            &db,
            &ss,
            vec![
                entry("a", Some("alice"), None),
                entry("b", Some("alice"), None),
                entry("c", Some("bob"), None),
            ],
        )
        .unwrap();
        assert_eq!(res.profiles_created, 3);
        // 同 user 共用，不同 user 分开 → 2 个 cred
        assert_eq!(res.credentials_created, 2);
        let creds = crate::db::credential::list(&db).unwrap();
        assert!(creds
            .iter()
            .all(|c| c.credential_type == CredentialType::Agent));
    }

    #[test]
    fn no_user_no_file_creates_agent_credential_with_system_user() {
        // User=∅, IdentityFile=∅ —— 不再 import 成空 credential_id 的脏 profile，
        // 而是建 Agent cred（username = 系统当前用户），让 ssh-agent / 默认密钥发挥作用。
        let (db, ss, _dir) = fixture();
        let res = do_import_ssh_entries(&db, &ss, vec![entry("bare", None, None)]).unwrap();
        assert_eq!(res.profiles_created, 1);
        assert_eq!(res.credentials_created, 1);
        let profiles = crate::db::profile::list(&db).unwrap();
        assert!(!profiles[0].credential_id.is_empty());
        let creds = crate::db::credential::list(&db).unwrap();
        assert_eq!(creds.len(), 1);
        assert_eq!(creds[0].credential_type, CredentialType::Agent);
        assert_eq!(creds[0].username, current_system_user());
    }

    #[test]
    fn missing_key_file_collects_error_and_skips_profile() {
        // 私钥读不到 → 错误前置：profile 不建，errors 报告。
        // 不再在 DB 里留下引用空 credential_id 的脏 profile。
        let (db, ss, _dir) = fixture();
        let res = do_import_ssh_entries(
            &db,
            &ss,
            vec![entry(
                "ghost",
                Some("alice"),
                Some("/nonexistent/path/id_rsa"),
            )],
        )
        .unwrap();
        assert_eq!(res.profiles_created, 0);
        assert_eq!(res.credentials_created, 0);
        assert_eq!(res.errors.len(), 1);
        assert_eq!(res.errors[0].kind, "read_key");
        assert!(crate::db::profile::list(&db).unwrap().is_empty());
    }

    #[test]
    fn key_credential_secret_lands_in_secret_store() {
        let (db, ss, dir) = fixture();
        let key = write_key(&dir, "id_ed25519", "MY-PEM-CONTENT");
        let res =
            do_import_ssh_entries(&db, &ss, vec![entry("h", Some("alice"), Some(&key))]).unwrap();
        assert_eq!(res.credentials_created, 1);
        let cred = &crate::db::credential::list(&db).unwrap()[0];
        let stored = ss.get(&cred_secret_key(&cred.id)).unwrap();
        assert_eq!(stored.as_deref(), Some("MY-PEM-CONTENT"));
    }

    #[test]
    fn dedup_holds_across_mixed_user_and_file_combos() {
        // 6 entries 覆盖 5 种独立 (user, file) 组合：
        //   (alice, k1) ×2 → 1 Key cred
        //   (bob,   k1)    → 1 Key cred
        //   (alice, k2)    → 1 Key cred
        //   (alice, ∅)     → 1 Agent cred (alice)
        //   (∅,     ∅)     → 1 Agent cred (system user)
        // 假设 system user ≠ "alice"（CI 用户名是 runner / 类似），共 5 cred。
        let (db, ss, dir) = fixture();
        let k1 = write_key(&dir, "k1", "PEM-1");
        let k2 = write_key(&dir, "k2", "PEM-2");
        let res = do_import_ssh_entries(
            &db,
            &ss,
            vec![
                entry("p1", Some("alice"), Some(&k1)),
                entry("p2", Some("alice"), Some(&k1)),
                entry("p3", Some("bob"), Some(&k1)),
                entry("p4", Some("alice"), Some(&k2)),
                entry("p5", Some("alice"), None),
                entry("p6", None, None),
            ],
        )
        .unwrap();
        assert_eq!(res.profiles_created, 6);
        // system_user 恰好是 "alice" 时 p5 和 p6 共用一个 cred —— 退化到 4。
        let expected = if current_system_user() == "alice" {
            4
        } else {
            5
        };
        assert_eq!(res.credentials_created, expected);
    }
}
