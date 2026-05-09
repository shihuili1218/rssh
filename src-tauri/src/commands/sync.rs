use serde_json::json;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::models::Credential;
use crate::secret::{cred_secret_key, setting_key};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// 取凭证 secret 用于导出/同步 —— DB 不存 secret，统一走 SecretStore
// ---------------------------------------------------------------------------

/// 列出所有 credentials 并把 SecretStore 中的 secret 灌进去。
fn list_credentials_with_secrets(state: &State<'_, AppState>) -> AppResult<Vec<Credential>> {
    let mut creds = crate::db::credential::list(&state.db)?;
    for c in creds.iter_mut() {
        c.secret = state.secret_store.get(&cred_secret_key(&c.id))?;
    }
    Ok(creds)
}

// ---------------------------------------------------------------------------
// 本地导入导出（全平台）
// ---------------------------------------------------------------------------

/// 构造完整 export payload（profiles + credentials + forwards + groups + skills）。
/// 抽出为私有 helper：`export_config`（CLI / 字符串场景）和 `export_config_to_file`
/// （GUI / 文件场景）共用，避免两份不一样的 JSON schema 漂移。
fn build_export_json(state: &State<'_, AppState>) -> AppResult<String> {
    let profiles = crate::db::profile::list(&state.db)?;
    let credentials = list_credentials_with_secrets(state)?;
    let forwards = crate::db::forward::list(&state.db)?;
    let groups = crate::db::group::list(&state.db)?;
    let skills = crate::ai::skills::list_user(&state.db)?;
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
    build_export_json(&state)
}

/// 文件 import：增量合并语义。本地已有数据保留；同 id 的实体被覆盖；
/// 解析或写入失败逐项收集，不影响其他条目。
#[tauri::command]
pub fn import_config(state: State<'_, AppState>, json: String) -> AppResult<()> {
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
    let payload = build_export_json(&state)?;

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
    let json = std::fs::read_to_string(&path)?;
    let data: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
    crate::sync::config::merge_import(&state.db, state.secret_store.as_ref(), &data)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

// ---------------------------------------------------------------------------
// GitHub sync
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn github_push(state: State<'_, AppState>, password: String) -> AppResult<()> {
    use crate::sync::github::GitHubSync;

    let token = state
        .secret_store
        .get(&setting_key("github_token"))?
        .ok_or_else(|| AppError::config("github_token_missing", json!({})))?;
    let repo = crate::db::settings::get(&state.db, "github_repo")?
        .ok_or_else(|| AppError::config("github_repo_missing", json!({})))?;
    let branch = crate::db::settings::get(&state.db, "github_branch")?.unwrap_or("main".into());

    let profiles = crate::db::profile::list(&state.db)?;
    let mut credentials = list_credentials_with_secrets(&state)?;
    let forwards = crate::db::forward::list(&state.db)?;
    let groups = crate::db::group::list(&state.db)?;
    let skills = crate::ai::skills::list_user(&state.db)?;

    // 尊重 save_to_remote：不同步的凭证清空 secret
    for c in credentials.iter_mut() {
        if !c.save_to_remote {
            c.secret = None;
        }
    }

    let json = serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "profiles": profiles,
        "credentials": credentials,
        "forwards": forwards,
        "groups": groups,
        "skills": skills,
    }))
    .map_err(|e| AppError::other("serde_failed", json!({ "err": e.to_string() })))?;

    let encrypted = crate::crypto::encrypt(&json, &password)?;
    let sync = GitHubSync::from_settings(&token, &repo, &branch)?;
    sync.push(&encrypted).await
}

#[tauri::command]
pub async fn github_pull(state: State<'_, AppState>, password: String) -> AppResult<()> {
    use crate::sync::github::GitHubSync;

    let token = state
        .secret_store
        .get(&setting_key("github_token"))?
        .ok_or_else(|| AppError::config("github_token_missing", json!({})))?;
    let repo = crate::db::settings::get(&state.db, "github_repo")?
        .ok_or_else(|| AppError::config("github_repo_missing", json!({})))?;
    let branch = crate::db::settings::get(&state.db, "github_branch")?.unwrap_or("main".into());

    let sync = GitHubSync::from_settings(&token, &repo, &branch)?;
    let encrypted = sync.pull().await?;
    let json = crate::crypto::decrypt(&encrypted, &password)?;

    let data: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
    // pull = 全量替换语义：clear+insert 包在事务里，任何失败整体回滚。
    crate::sync::config::replace_import(&state.db, state.secret_store.as_ref(), &data)
}
