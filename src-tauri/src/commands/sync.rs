use serde_json::json;
use tauri::State;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::models::Credential;
use crate::secret::{cred_secret_key, setting_key, SecretStore};
use crate::state::AppState;
use std::sync::Arc;

/// Run a blocking DB closure off the tokio async runtime. The async GitHub
/// sync commands all wrap a multi-statement DB step (the worst is
/// `merge_import`, which upserts every config category and can hold the SQLite
/// writer for tens to hundreds of ms). Without spawn_blocking, the async
/// runtime worker that handled `github_push` / `github_pull` stalls for that
/// whole window and any other tab's async command on the same worker stalls
/// with it.
///
/// `ssh_connect` and `forward_start` also touch the DB inside an `async fn`,
/// but each call is a fast SELECT (<1 ms) and the total per-connect run is
/// ~10 ms — well below the threshold where wrapping pays for the closure
/// boilerplate. Leave them alone; this helper is targeted at sync's heavy paths.
async fn run_db_blocking<F, T>(state: &AppState, f: F) -> AppResult<T>
where
    F: FnOnce(Arc<Db>, Arc<dyn SecretStore>) -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    let db = state.db.clone();
    let ss = state.secret_store.clone();
    tauri::async_runtime::spawn_blocking(move || f(db, ss))
        .await
        .map_err(|e| AppError::other("blocking_join_failed", json!({ "err": e.to_string() })))?
}

fn collect_credentials_with_secrets(db: &Db, ss: &dyn SecretStore) -> AppResult<Vec<Credential>> {
    let mut creds = crate::db::credential::list(db)?;
    for c in creds.iter_mut() {
        c.secret = ss.get(&cred_secret_key(&c.id))?;
    }
    Ok(creds)
}

// ---------------------------------------------------------------------------
// Local import/export (cross-platform)
// ---------------------------------------------------------------------------

/// Per-category sync toggles + the profile group filter. All booleans default
/// to ON (absent setting = included) so turning on sync keeps today's
/// "sync everything" behavior; the user opts OUT per category.
#[derive(Debug)]
struct SyncPrefs {
    credentials: bool,
    forwards: bool,
    groups: bool,
    serial: bool,
    skills: bool,
    highlights: bool,
    snippets: bool,
    ai_redact: bool,
    ai_blacklist: bool,
    ai: bool,
    ai_key: bool,
    /// `None` = all profiles; `Some(ids)` = only profiles in those groups.
    profile_group_ids: Option<Vec<String>>,
}

/// What flavor of payload to build.
enum ExportMode {
    /// Full local backup: every category, every secret, no toggles.
    LocalBackup,
    /// GitHub push: apply per-category toggles + group filter; scrub the secret
    /// of credentials flagged local-only.
    GitHubPush(SyncPrefs),
}

fn read_sync_prefs(db: &Db) -> AppResult<SyncPrefs> {
    // Absent or any value other than "0" → on. Only an explicit "0" disables.
    let flag = |key: &str| -> AppResult<bool> {
        Ok(crate::db::settings::get(db, key)?.is_none_or(|v| v != "0"))
    };
    // Empty string / absent → None → all profiles (incl. ungrouped); this is
    // the "all groups selected" default. A JSON array → that exact set
    // (an empty array means sync no profiles). Malformed → error, never None:
    // silently falling back to None would widen a deliberately-narrowed export
    // back to every profile (a privacy leak), which is fail-OPEN, not safe.
    let profile_group_ids = match crate::db::settings::get(db, "sync_profile_group_ids")? {
        Some(s) if !s.trim().is_empty() => Some(serde_json::from_str::<Vec<String>>(&s).map_err(
            |e| AppError::config("sync_profile_group_ids_invalid", json!({ "err": e.to_string() })),
        )?),
        _ => None,
    };
    Ok(SyncPrefs {
        credentials: flag("sync_include_credentials")?,
        forwards: flag("sync_include_forwards")?,
        groups: flag("sync_include_groups")?,
        serial: flag("sync_include_serial")?,
        skills: flag("sync_include_skills")?,
        highlights: flag("sync_include_highlights")?,
        snippets: flag("sync_include_snippets")?,
        ai_redact: flag("sync_include_ai_redact")?,
        ai_blacklist: flag("sync_include_ai_blacklist")?,
        ai: flag("sync_include_ai")?,
        ai_key: flag("sync_include_ai_key")?,
        profile_group_ids,
    })
}

fn to_val<T: serde::Serialize>(v: T) -> AppResult<serde_json::Value> {
    serde_json::to_value(v).map_err(|e| AppError::other("serde_failed", json!({ "err": e.to_string() })))
}

/// Build the export payload as a JSON value — the single source of truth for
/// the sync shape, shared by local export AND GitHub push so the JSON can't
/// drift between them. On push, a disabled category's key is simply omitted
/// (absence = "not synced"); merge_import then leaves that local table alone.
/// `data_dir` feeds the file-backed `snippets` category.
fn build_payload(
    db: &Db,
    ss: &dyn SecretStore,
    data_dir: &std::path::Path,
    mode: &ExportMode,
) -> AppResult<serde_json::Value> {
    let prefs = match mode {
        ExportMode::GitHubPush(p) => Some(p),
        ExportMode::LocalBackup => None,
    };
    let on = |pick: fn(&SyncPrefs) -> bool| prefs.is_none_or(pick);

    let mut out = serde_json::Map::new();
    out.insert("version".into(), json!(1));
    out.insert(
        "exported_at".into(),
        json!(chrono::Utc::now().to_rfc3339()),
    );

    // profiles — always present, filtered to the selected groups on push.
    let mut profiles = crate::db::profile::list(db)?;
    if let Some(gids) = prefs.and_then(|p| p.profile_group_ids.as_ref()) {
        let set: std::collections::HashSet<&str> = gids.iter().map(String::as_str).collect();
        profiles.retain(|pr| pr.group_id.as_deref().is_some_and(|g| set.contains(g)));
    }
    out.insert("profiles".into(), to_val(profiles)?);

    if on(|p| p.credentials) {
        let mut credentials = collect_credentials_with_secrets(db, ss)?;
        if prefs.is_some() {
            for c in credentials.iter_mut() {
                if !c.save_to_remote {
                    c.secret = None;
                }
            }
        }
        out.insert("credentials".into(), to_val(credentials)?);
    }
    if on(|p| p.forwards) {
        out.insert("forwards".into(), to_val(crate::db::forward::list(db)?)?);
    }
    if on(|p| p.groups) {
        out.insert("groups".into(), to_val(crate::db::group::list(db)?)?);
    }
    if on(|p| p.serial) {
        out.insert(
            "serial_profiles".into(),
            to_val(crate::db::serial_profile::list(db)?)?,
        );
    }
    if on(|p| p.skills) {
        out.insert("skills".into(), to_val(crate::ai::skills::list_user(db)?)?);
    }
    if on(|p| p.highlights) {
        out.insert("highlights".into(), to_val(crate::db::highlight::list(db)?)?);
    }
    if on(|p| p.snippets) {
        out.insert("snippets".into(), to_val(crate::db::snippet::load(data_dir)?)?);
    }
    if on(|p| p.ai_redact) {
        out.insert(
            "ai_redact_rules".into(),
            to_val(crate::db::ai_redact_rule::list(db)?)?,
        );
    }
    if on(|p| p.ai_blacklist) {
        out.insert(
            "ai_command_blacklist".into(),
            to_val(crate::db::ai_command_blacklist::list(db)?)?,
        );
    }
    if on(|p| p.ai) {
        let include_keys = on(|p| p.ai_key);
        out.insert(
            "ai".into(),
            crate::ai::commands::export_ai_settings(db, ss, include_keys)?,
        );
    }

    Ok(serde_json::Value::Object(out))
}

/// Pretty-printed full local backup (every category + secret, no toggles).
/// Used by `export_config` and `export_config_to_file`.
fn build_export_json_blocking(
    db: &Db,
    ss: &dyn SecretStore,
    data_dir: &std::path::Path,
) -> AppResult<String> {
    let payload = build_payload(db, ss, data_dir, &ExportMode::LocalBackup)?;
    serde_json::to_string_pretty(&payload)
        .map_err(|e| AppError::other("serde_failed", json!({ "err": e.to_string() })))
}

#[tauri::command]
pub fn export_config(state: State<'_, AppState>) -> AppResult<String> {
    export_config_impl(&state)
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
/// Sync — the caller runs it on a blocking-safe context.
pub fn export_config_impl(state: &AppState) -> AppResult<String> {
    build_export_json_blocking(&state.db, state.secret_store.as_ref(), &state.data_dir)
}

/// File import: incremental merge. Local rows survive; same-id rows are
/// overwritten; parse/write failures are collected per row, not aborting
/// the whole import.
///
/// Sync command — Tauri runs this on the blocking pool, so the multi-table
/// transaction inside `merge_import` doesn't stall the async runtime.
#[tauri::command]
pub fn import_config(state: State<'_, AppState>, json: String) -> AppResult<()> {
    import_config_impl(&state, json)
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub fn import_config_impl(state: &AppState, json: String) -> AppResult<()> {
    let data: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
    crate::sync::config::merge_import(
        &state.db,
        state.secret_store.as_ref(),
        &state.data_dir,
        &data,
    )
}

/// 弹原生 Save 对话框选路径，把当前完整配置写入该文件。
/// 用户取消返回 None；写盘成功返回路径字符串。
/// Android 无 rfd 依赖，硬阻碍。
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn export_config_to_file(state: State<'_, AppState>) -> AppResult<Option<String>> {
    // Build payload on the blocking pool — same rationale as the GitHub
    // commands. After this point everything is either user-driven IO
    // (the native file dialog) or a single file write.
    let data_dir = state.data_dir.clone();
    let payload = run_db_blocking(&state, move |db, ss| {
        build_export_json_blocking(&db, ss.as_ref(), &data_dir)
    })
    .await?;

    let default_dir = dirs::document_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let default_name = format!(
        "rssh-config-{}.json",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );

    let pick = rfd::AsyncFileDialog::new()
        .set_directory(default_dir)
        .set_file_name(default_name)
        .add_filter("JSON", &["json"])
        .save_file()
        .await;

    let Some(handle) = pick else { return Ok(None) };
    let path = handle.path().to_path_buf();
    std::fs::write(&path, payload.as_bytes())?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// 弹原生 Open 对话框选 JSON 文件，按 merge_import 语义合并到本地配置。
/// 用户取消返回 None；导入成功返回文件路径。
/// Android 无 rfd 依赖，硬阻碍。
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn import_config_from_file(state: State<'_, AppState>) -> AppResult<Option<String>> {
    let pick = rfd::AsyncFileDialog::new()
        .add_filter("JSON", &["json"])
        .pick_file()
        .await;

    let Some(handle) = pick else { return Ok(None) };
    let path = handle.path().to_path_buf();
    let path_for_return = path.clone();
    let data_dir = state.data_dir.clone();

    // merge_import walks every config category and upserts each row — keep that
    // off the async worker.
    run_db_blocking(&state, move |db, ss| {
        let json = std::fs::read_to_string(&path)?;
        let data: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
        crate::sync::config::merge_import(&db, ss.as_ref(), &data_dir, &data)
    })
    .await?;

    Ok(Some(path_for_return.to_string_lossy().into_owned()))
}

// ---------------------------------------------------------------------------
// GitHub sync
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn github_push(state: State<'_, AppState>, password: String) -> AppResult<()> {
    github_push_impl(&state, password).await
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub async fn github_push_impl(state: &AppState, password: String) -> AppResult<()> {
    use crate::sync::github::GitHubSync;

    // Build the full JSON payload off the async runtime — list_*, secret-store
    // lookups, and serde all run in the blocking pool. See `run_db_blocking`
    // doc for why this matters for sync but not for ssh_connect.
    let data_dir = state.data_dir.clone();
    let (token, repo, branch, json) = run_db_blocking(state, move |db, ss| {
        let token = ss
            .get(&setting_key("github_token"))?
            .ok_or_else(|| AppError::config("github_token_missing", json!({})))?;
        let repo = crate::db::settings::get(&db, "github_repo")?
            .ok_or_else(|| AppError::config("github_repo_missing", json!({})))?;
        let branch = crate::db::settings::get(&db, "github_branch")?.unwrap_or("main".into());

        // Push path: apply per-category toggles + group filter, scrub
        // local-only secrets. Same builder as local export so the shape can't
        // drift; a disabled category is just absent from the JSON.
        let prefs = read_sync_prefs(&db)?;
        let payload = build_payload(&db, ss.as_ref(), &data_dir, &ExportMode::GitHubPush(prefs))?;
        let payload = serde_json::to_string_pretty(&payload)
            .map_err(|e| AppError::other("serde_failed", json!({ "err": e.to_string() })))?;

        Ok((token, repo, branch, payload))
    })
    .await?;

    let encrypted = crate::crypto::encrypt(&json, &password)?;
    let sync = GitHubSync::from_settings(&token, &repo, &branch)?;
    sync.push(&encrypted).await
}

#[tauri::command]
pub async fn github_pull(state: State<'_, AppState>, password: String) -> AppResult<()> {
    github_pull_impl(&state, password).await
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub async fn github_pull_impl(state: &AppState, password: String) -> AppResult<()> {
    use crate::sync::github::GitHubSync;

    // Settings reads are cheap; group them with the network-prep step.
    // The decrypt + merge_import block — `merge_import` upserts every config
    // category — is the expensive part and goes to spawn_blocking below.
    let (token, repo, branch) = run_db_blocking(state, |db, ss| {
        let token = ss
            .get(&setting_key("github_token"))?
            .ok_or_else(|| AppError::config("github_token_missing", json!({})))?;
        let repo = crate::db::settings::get(&db, "github_repo")?
            .ok_or_else(|| AppError::config("github_repo_missing", json!({})))?;
        let branch = crate::db::settings::get(&db, "github_branch")?.unwrap_or("main".into());
        Ok((token, repo, branch))
    })
    .await?;

    let sync = GitHubSync::from_settings(&token, &repo, &branch)?;
    let encrypted = sync.pull().await?;

    // decrypt + JSON parse + merge upsert: all blocking work.
    let data_dir = state.data_dir.clone();
    run_db_blocking(state, move |db, ss| {
        let json = crate::crypto::decrypt(&encrypted, &password)?;
        let data: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
        crate::sync::config::merge_import(&db, ss.as_ref(), &data_dir, &data)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::profile;
    use crate::models::Profile;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-process SecretStore for tests.
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

    fn fixture() -> (Db, MemStore, tempfile::TempDir) {
        (
            Db::open_in_memory().unwrap(),
            MemStore::default(),
            tempfile::tempdir().unwrap(),
        )
    }

    fn prof(id: &str, group: Option<&str>) -> Profile {
        Profile {
            id: id.into(),
            name: format!("name-{id}"),
            host: "h".into(),
            port: 22,
            credential_id: "c".into(),
            bastion_profile_id: None,
            init_command: None,
            group_id: group.map(String::from),
        }
    }

    #[test]
    fn local_backup_includes_all_categories() {
        let (db, ss, dir) = fixture();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::LocalBackup).unwrap();
        let obj = v.as_object().unwrap();
        for k in [
            "profiles",
            "credentials",
            "forwards",
            "groups",
            "serial_profiles",
            "skills",
            "highlights",
            "snippets",
            "ai_redact_rules",
            "ai_command_blacklist",
            "ai",
        ] {
            assert!(obj.contains_key(k), "local backup missing '{k}'");
        }
    }

    #[test]
    fn push_excludes_disabled_category() {
        let (db, ss, dir) = fixture();
        crate::db::settings::set(&db, "sync_include_highlights", "0").unwrap();
        crate::db::settings::set(&db, "sync_include_snippets", "0").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::GitHubPush(prefs)).unwrap();
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("highlights"), "disabled key omitted");
        assert!(!obj.contains_key("snippets"), "disabled key omitted");
        assert!(obj.contains_key("credentials"), "enabled key present");
        assert!(obj.contains_key("profiles"));
    }

    #[test]
    fn push_filters_profiles_by_group() {
        let (db, ss, dir) = fixture();
        profile::insert(&db, &prof("p1", Some("g1"))).unwrap();
        profile::insert(&db, &prof("p2", Some("g2"))).unwrap();
        profile::insert(&db, &prof("p3", None)).unwrap();
        crate::db::settings::set(&db, "sync_profile_group_ids", "[\"g1\"]").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::GitHubPush(prefs)).unwrap();
        let ids: Vec<&str> = v["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["p1"], "only the selected group is exported");
    }

    #[test]
    fn push_empty_group_list_syncs_no_profiles() {
        // Explicit empty array = user deselected every group → sync nothing.
        let (db, ss, dir) = fixture();
        profile::insert(&db, &prof("p1", Some("g1"))).unwrap();
        crate::db::settings::set(&db, "sync_profile_group_ids", "[]").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::GitHubPush(prefs)).unwrap();
        assert_eq!(v["profiles"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn malformed_group_filter_errors_not_fail_open() {
        // A corrupted setting must error, never silently fall back to None
        // (= "sync all"), which would widen a narrowed export to every profile.
        let (db, _ss, _dir) = fixture();
        crate::db::settings::set(&db, "sync_profile_group_ids", "{not json").unwrap();
        let err = read_sync_prefs(&db).unwrap_err();
        assert_eq!(err.code(), "sync_profile_group_ids_invalid");
    }

    #[test]
    fn push_empty_string_group_filter_syncs_all_profiles() {
        // Empty string = "all groups selected" sentinel → no filter (incl. ungrouped).
        let (db, ss, dir) = fixture();
        profile::insert(&db, &prof("p1", Some("g1"))).unwrap();
        profile::insert(&db, &prof("p2", None)).unwrap();
        crate::db::settings::set(&db, "sync_profile_group_ids", "").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::GitHubPush(prefs)).unwrap();
        assert_eq!(
            v["profiles"].as_array().unwrap().len(),
            2,
            "empty string = sync all, including ungrouped"
        );
    }

    #[test]
    fn push_omits_ai_key_when_disabled() {
        let (db, ss, dir) = fixture();
        crate::db::settings::set(&db, "ai_anthropic_model", "claude-x").unwrap();
        ss.set(&setting_key("ai_anthropic_key"), "sk-secret").unwrap();
        crate::db::settings::set(&db, "sync_include_ai_key", "0").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::GitHubPush(prefs)).unwrap();
        let anth = v["ai"]["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["provider"] == "anthropic")
            .expect("anthropic present (model configured)");
        assert!(anth.get("api_key").is_none(), "api_key gated off");
        assert_eq!(anth["model"], "claude-x", "non-secret fields still synced");
    }

    #[test]
    fn payload_never_contains_sync_toggles() {
        let (db, ss, dir) = fixture();
        crate::db::settings::set(&db, "sync_include_forwards", "0").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::GitHubPush(prefs)).unwrap();
        let s = serde_json::to_string(&v).unwrap();
        assert!(!s.contains("sync_include"), "toggle keys never leave the device");
        assert!(!s.contains("sync_profile_group_ids"));
    }
}
