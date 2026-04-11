use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// 本地导入导出（全平台）
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn export_config(state: State<'_, AppState>) -> AppResult<String> {
    let conn = state.db.lock().map_err(|_| AppError::Other("lock".into()))?;
    let profiles = crate::db::profile::list(&conn)?;
    let credentials = crate::db::credential::list(&conn)?;
    let forwards = crate::db::forward::list(&conn)?;
    serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "profiles": profiles,
        "credentials": credentials,
        "forwards": forwards,
    }))
    .map_err(|e| AppError::Other(e.to_string()))
}

#[tauri::command]
pub fn import_config(state: State<'_, AppState>, json: String) -> AppResult<()> {
    let data: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| AppError::Config(format!("JSON 解析失败: {e}")))?;
    let conn = state.db.lock().map_err(|_| AppError::Other("lock".into()))?;

    crate::db::credential::clear_all(&conn)?;
    crate::db::profile::clear_all(&conn)?;
    crate::db::forward::clear_all(&conn)?;

    let mut errors = Vec::new();

    if let Some(arr) = data["credentials"].as_array() {
        for item in arr {
            match serde_json::from_value::<crate::models::Credential>(item.clone()) {
                Ok(c) => { if let Err(e) = crate::db::credential::insert(&conn, &c) { errors.push(format!("credential {}: {e}", c.name)); } }
                Err(e) => errors.push(format!("credential parse: {e}")),
            }
        }
    }
    if let Some(arr) = data["profiles"].as_array() {
        for item in arr {
            match serde_json::from_value::<crate::models::Profile>(item.clone()) {
                Ok(p) => { if let Err(e) = crate::db::profile::insert(&conn, &p) { errors.push(format!("profile {}: {e}", p.name)); } }
                Err(e) => errors.push(format!("profile parse: {e}")),
            }
        }
    }
    if let Some(arr) = data["forwards"].as_array() {
        for item in arr {
            match serde_json::from_value::<crate::models::Forward>(item.clone()) {
                Ok(f) => { if let Err(e) = crate::db::forward::insert(&conn, &f) { errors.push(format!("forward {}: {e}", f.name)); } }
                Err(e) => errors.push(format!("forward parse: {e}")),
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Other(format!("部分导入失败: {}", errors.join("; "))))
    }
}

// ---------------------------------------------------------------------------
// GitHub sync（桌面专用）
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn github_push(state: State<'_, AppState>, password: String) -> AppResult<()> {
    use crate::sync::github::GitHubSync;

    let (token, repo, branch, json) = {
        let conn = state.db.lock().map_err(|_| AppError::Other("lock".into()))?;
        let token = crate::db::settings::get(&conn, "github_token")?
            .ok_or_else(|| AppError::Config("未配置 GitHub Token".into()))?;
        let repo = crate::db::settings::get(&conn, "github_repo")?
            .ok_or_else(|| AppError::Config("未配置 GitHub Repo".into()))?;
        let branch = crate::db::settings::get(&conn, "github_branch")?.unwrap_or("main".into());

        let profiles = crate::db::profile::list(&conn)?;
        let mut credentials = crate::db::credential::list(&conn)?;
        let forwards = crate::db::forward::list(&conn)?;

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
        }))
        .map_err(|e| AppError::Other(e.to_string()))?;

        (token, repo, branch, json)
    };

    let encrypted = crate::crypto::encrypt(&json, &password)?;
    let sync = GitHubSync::from_settings(&token, &repo, &branch)?;
    sync.push(&encrypted).await
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn github_pull(state: State<'_, AppState>, password: String) -> AppResult<()> {
    use crate::sync::github::GitHubSync;

    let (token, repo, branch) = {
        let conn = state.db.lock().map_err(|_| AppError::Other("lock".into()))?;
        let token = crate::db::settings::get(&conn, "github_token")?
            .ok_or_else(|| AppError::Config("未配置 GitHub Token".into()))?;
        let repo = crate::db::settings::get(&conn, "github_repo")?
            .ok_or_else(|| AppError::Config("未配置 GitHub Repo".into()))?;
        let branch = crate::db::settings::get(&conn, "github_branch")?.unwrap_or("main".into());
        (token, repo, branch)
    };

    let sync = GitHubSync::from_settings(&token, &repo, &branch)?;
    let encrypted = sync.pull().await?;
    let json = crate::crypto::decrypt(&encrypted, &password)?;

    let data: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| AppError::Config(format!("JSON 解析失败: {e}")))?;
    let conn = state.db.lock().map_err(|_| AppError::Other("lock".into()))?;

    crate::db::credential::clear_all(&conn)?;
    crate::db::profile::clear_all(&conn)?;
    crate::db::forward::clear_all(&conn)?;

    let mut errors = Vec::new();

    if let Some(arr) = data["credentials"].as_array() {
        for item in arr {
            match serde_json::from_value::<crate::models::Credential>(item.clone()) {
                Ok(c) => { if let Err(e) = crate::db::credential::insert(&conn, &c) { errors.push(format!("credential {}: {e}", c.name)); } }
                Err(e) => errors.push(format!("credential parse: {e}")),
            }
        }
    }
    if let Some(arr) = data["profiles"].as_array() {
        for item in arr {
            match serde_json::from_value::<crate::models::Profile>(item.clone()) {
                Ok(p) => { if let Err(e) = crate::db::profile::insert(&conn, &p) { errors.push(format!("profile {}: {e}", p.name)); } }
                Err(e) => errors.push(format!("profile parse: {e}")),
            }
        }
    }
    if let Some(arr) = data["forwards"].as_array() {
        for item in arr {
            match serde_json::from_value::<crate::models::Forward>(item.clone()) {
                Ok(f) => { if let Err(e) = crate::db::forward::insert(&conn, &f) { errors.push(format!("forward {}: {e}", f.name)); } }
                Err(e) => errors.push(format!("forward parse: {e}")),
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Other(format!("部分导入失败: {}", errors.join("; "))))
    }
}
