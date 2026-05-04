use serde_json::json;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::models::Credential;
use crate::secret::{cred_secret_key, setting_key};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// 取/写凭证 secret —— DB 不存 secret，统一走 SecretStore
// ---------------------------------------------------------------------------

/// 列出所有 credentials 并把 SecretStore 中的 secret 灌进去。
fn list_credentials_with_secrets(state: &State<'_, AppState>) -> AppResult<Vec<Credential>> {
    let mut creds = crate::db::credential::list(&state.db)?;
    for c in creds.iter_mut() {
        c.secret = state.secret_store.get(&cred_secret_key(&c.id))?;
    }
    Ok(creds)
}

/// 把一个反序列化出来的 Credential 完整写入（DB + SecretStore）。
fn upsert_credential(state: &State<'_, AppState>, c: &Credential) -> AppResult<()> {
    crate::db::credential::insert(&state.db, c)?;
    let sk = cred_secret_key(&c.id);
    match c.secret.as_deref() {
        Some(s) if !s.is_empty() => state.secret_store.set(&sk, s)?,
        _ => state.secret_store.delete(&sk)?,
    }
    Ok(())
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

#[tauri::command]
pub fn import_config(state: State<'_, AppState>, json: String) -> AppResult<()> {
    let data: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
    apply_import(&state, &data)
}

fn apply_import(state: &State<'_, AppState>, data: &serde_json::Value) -> AppResult<()> {
    // 清空旧数据（凭证连带 secret 一起清）
    if let Ok(old) = crate::db::credential::list(&state.db) {
        for c in old {
            let _ = state.secret_store.delete(&cred_secret_key(&c.id));
        }
    }
    crate::db::credential::clear_all(&state.db)?;
    crate::db::profile::clear_all(&state.db)?;
    crate::db::forward::clear_all(&state.db)?;
    crate::db::group::clear_all(&state.db)?;

    // 收集每条失败的结构化记录，避免把内层 AppError.to_string() 协议串塞进外层 params。
    let mut errors: Vec<serde_json::Value> = Vec::new();

    if let Some(arr) = data["credentials"].as_array() {
        for item in arr {
            match serde_json::from_value::<crate::models::Credential>(item.clone()) {
                Ok(c) => {
                    if let Err(e) = upsert_credential(state, &c) {
                        errors.push(json!({ "kind": "credential", "name": c.name, "code": e.code() }));
                    }
                }
                Err(_) => errors.push(json!({ "kind": "credential", "code": "parse_failed" })),
            }
        }
    }
    if let Some(arr) = data["profiles"].as_array() {
        for item in arr {
            match serde_json::from_value::<crate::models::Profile>(item.clone()) {
                Ok(p) => {
                    if let Err(e) = crate::db::profile::insert(&state.db, &p) {
                        errors.push(json!({ "kind": "profile", "name": p.name, "code": e.code() }));
                    }
                }
                Err(_) => errors.push(json!({ "kind": "profile", "code": "parse_failed" })),
            }
        }
    }
    if let Some(arr) = data["forwards"].as_array() {
        for item in arr {
            match serde_json::from_value::<crate::models::Forward>(item.clone()) {
                Ok(f) => {
                    if let Err(e) = crate::db::forward::insert(&state.db, &f) {
                        errors.push(json!({ "kind": "forward", "name": f.name, "code": e.code() }));
                    }
                }
                Err(_) => errors.push(json!({ "kind": "forward", "code": "parse_failed" })),
            }
        }
    }
    if let Some(arr) = data["groups"].as_array() {
        for item in arr {
            match serde_json::from_value::<crate::models::Group>(item.clone()) {
                Ok(g) => {
                    if let Err(e) = crate::db::group::insert(&state.db, &g) {
                        errors.push(json!({ "kind": "group", "name": g.name, "code": e.code() }));
                    }
                }
                Err(_) => errors.push(json!({ "kind": "group", "code": "parse_failed" })),
            }
        }
    }

    // user skills：仅当 payload **显式带** "skills" 字段时才覆盖。
    // 老 v1 payload（无字段）→ data.get 返回 None → 跳过，保留本地 user skills。
    // 新 payload "skills": [] → 显式空覆盖 = 清空本地。
    // builtin "general" 不入表，clear_all 不会影响它。
    if let Some(skills_val) = data.get("skills").filter(|v| !v.is_null()) {
        if let Some(arr) = skills_val.as_array() {
            crate::db::ai_skill::clear_all(&state.db)?;
            for item in arr {
                match serde_json::from_value::<crate::ai::skills::SkillRecord>(item.clone()) {
                    Ok(s) if !s.builtin => {
                        let user = crate::db::ai_skill::UserSkill {
                            id: s.id.clone(),
                            name: s.name,
                            description: s.description,
                            content: s.content,
                        };
                        if let Err(e) = crate::db::ai_skill::upsert(&state.db, &user) {
                            errors.push(json!({ "kind": "skill", "name": user.id, "code": e.code() }));
                        }
                    }
                    Ok(_) => {} // builtin 跳过
                    Err(_) => errors.push(json!({ "kind": "skill", "code": "parse_failed" })),
                }
            }
        } else {
            errors.push(json!({ "kind": "skills", "code": "field_not_array" }));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        // 把第一条失败的标量字段提到顶层，前端 i18n 模板能直接渲染；
        // 全量数组在结构上保留但不进文案（前端不渲染嵌套结构）。
        let first = errors.first().cloned().unwrap_or(json!({}));
        Err(AppError::other(
            "import_partial_failed",
            json!({
                "count": errors.len(),
                "first_kind": first.get("kind").cloned().unwrap_or(json!("?")),
                "first_name": first.get("name").cloned().unwrap_or(json!("?")),
                "first_code": first.get("code").cloned().unwrap_or(json!("?")),
            }),
        ))
    }
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

    let data: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
    apply_import(&state, &data)
}
