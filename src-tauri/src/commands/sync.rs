use serde_json::json;
use tauri::State;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::models::Credential;
use crate::secret::{cred_secret_key, setting_key, SecretStore};
use crate::state::AppState;
use std::sync::Arc;

/// Run a blocking DB closure off the tokio async runtime. The four async
/// GitHub sync commands all wrap a multi-statement DB step (the worst is
/// `replace_import`, which runs a full BEGIN IMMEDIATE / clear / re-insert
/// transaction — can hold the SQLite writer for tens to hundreds of ms).
/// Without spawn_blocking, the async runtime worker that handled
/// `github_push` / `github_pull` stalls for that whole window and any
/// other tab's async command on the same worker stalls with it.
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

/// Build the full export payload (profiles + credentials + forwards + groups
/// + skills). Used by both `export_config` (sync CLI/string path) and
/// `export_config_to_file` (async GUI path via spawn_blocking) so the JSON
/// shape can't drift between them. Takes `&Db` / `&dyn SecretStore` instead
/// of `State` so it works in both contexts.
fn build_export_json_blocking(db: &Db, ss: &dyn SecretStore) -> AppResult<String> {
    let profiles = crate::db::profile::list(db)?;
    let credentials = collect_credentials_with_secrets(db, ss)?;
    let forwards = crate::db::forward::list(db)?;
    let groups = crate::db::group::list(db)?;
    let skills = crate::ai::skills::list_user(db)?;
    serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "profiles": profiles,
        "credentials": credentials,
        "forwards": forwards,
        "groups": groups,
        "skills": skills,
    }))
    .map_err(|e| AppError::other("serde_failed", json!({ "err": e.to_string() })))
}

#[tauri::command]
pub fn export_config(state: State<'_, AppState>) -> AppResult<String> {
    export_config_impl(&state)
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
/// Sync — the caller runs it on a blocking-safe context.
pub fn export_config_impl(state: &AppState) -> AppResult<String> {
    build_export_json_blocking(&state.db, state.secret_store.as_ref())
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
    crate::sync::config::merge_import(&state.db, state.secret_store.as_ref(), &data)
}

/// 弹原生 Save 对话框选路径，把当前完整配置写入该文件。
/// 用户取消返回 None；写盘成功返回路径字符串。
/// Android 无 rfd 依赖，硬阻碍。
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn export_config_to_file(
    state: State<'_, AppState>,
) -> AppResult<Option<String>> {
    // Build payload on the blocking pool — same rationale as the GitHub
    // commands. After this point everything is either user-driven IO
    // (the native file dialog) or a single file write.
    let payload =
        run_db_blocking(&state, |db, ss| build_export_json_blocking(&db, ss.as_ref())).await?;

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
pub async fn import_config_from_file(
    state: State<'_, AppState>,
) -> AppResult<Option<String>> {
    let pick = rfd::AsyncFileDialog::new()
        .add_filter("JSON", &["json"])
        .pick_file()
        .await;

    let Some(handle) = pick else { return Ok(None) };
    let path = handle.path().to_path_buf();
    let path_for_return = path.clone();

    // merge_import walks profiles + credentials + forwards + groups + skills
    // and writes each through a transaction — keep that off the async worker.
    run_db_blocking(&state, move |db, ss| {
        let json = std::fs::read_to_string(&path)?;
        let data: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
        crate::sync::config::merge_import(&db, ss.as_ref(), &data)
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
    let (token, repo, branch, json) = run_db_blocking(state, move |db, ss| {
        let token = ss
            .get(&setting_key("github_token"))?
            .ok_or_else(|| AppError::config("github_token_missing", json!({})))?;
        let repo = crate::db::settings::get(&db, "github_repo")?
            .ok_or_else(|| AppError::config("github_repo_missing", json!({})))?;
        let branch =
            crate::db::settings::get(&db, "github_branch")?.unwrap_or("main".into());

        let profiles = crate::db::profile::list(&db)?;
        let mut credentials = collect_credentials_with_secrets(&db, ss.as_ref())?;
        let forwards = crate::db::forward::list(&db)?;
        let groups = crate::db::group::list(&db)?;
        let skills = crate::ai::skills::list_user(&db)?;

        // Honor save_to_remote: scrub secret on credentials marked local-only.
        for c in credentials.iter_mut() {
            if !c.save_to_remote {
                c.secret = None;
            }
        }

        let payload = serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "exported_at": chrono::Utc::now().to_rfc3339(),
            "profiles": profiles,
            "credentials": credentials,
            "forwards": forwards,
            "groups": groups,
            "skills": skills,
        }))
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
    // The decrypt + replace_import block — `replace_import` runs a
    // BEGIN IMMEDIATE transaction touching every config table — is the
    // expensive part and goes to spawn_blocking below.
    let (token, repo, branch) = run_db_blocking(state, |db, ss| {
        let token = ss
            .get(&setting_key("github_token"))?
            .ok_or_else(|| AppError::config("github_token_missing", json!({})))?;
        let repo = crate::db::settings::get(&db, "github_repo")?
            .ok_or_else(|| AppError::config("github_repo_missing", json!({})))?;
        let branch =
            crate::db::settings::get(&db, "github_branch")?.unwrap_or("main".into());
        Ok((token, repo, branch))
    })
    .await?;

    let sync = GitHubSync::from_settings(&token, &repo, &branch)?;
    let encrypted = sync.pull().await?;

    // decrypt + JSON parse + full-replace transaction: all blocking work.
    let password = password;
    run_db_blocking(state, move |db, ss| {
        let json = crate::crypto::decrypt(&encrypted, &password)?;
        let data: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
        crate::sync::config::replace_import(&db, ss.as_ref(), &data)
    })
    .await
}
