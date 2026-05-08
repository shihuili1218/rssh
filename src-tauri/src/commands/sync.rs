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

#[tauri::command]
pub fn export_config(state: State<'_, AppState>) -> AppResult<String> {
    let profiles = crate::db::profile::list(&state.db)?;
    let credentials = list_credentials_with_secrets(&state)?;
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

/// 文件 import：增量合并语义。本地已有数据保留；同 id 的实体被覆盖；
/// 解析或写入失败逐项收集，不影响其他条目。
#[tauri::command]
pub fn import_config(state: State<'_, AppState>, json: String) -> AppResult<()> {
    let data: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
    crate::sync::config::merge_import(&state.db, state.secret_store.as_ref(), &data)
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
