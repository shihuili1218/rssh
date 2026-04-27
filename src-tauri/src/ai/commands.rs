//! AI 模块的 Tauri 命令入口。仅前端 ↔ Rust 桥，不引入新 IPC 协议。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use serde_json::json;

use crate::error::{locked, AppError, AppResult};
use crate::secret::setting_key;
use crate::state::AppState;

use super::llm;
use super::sanitize;
use super::session::{self, DiagnoseSession, UserAction};
use super::skills::{self, SkillRecord};

// ─── BYOK 设置存储键 ────────────────────────────────────────────────

fn key_provider() -> String {
    setting_key("ai_provider")
}
fn key_model() -> String {
    setting_key("ai_model")
}
fn key_endpoint(provider: &str) -> String {
    setting_key(&format!("ai_{provider}_endpoint"))
}
fn key_api_key(provider: &str) -> String {
    setting_key(&format!("ai_{provider}_key"))
}

// ─── 命令 ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct AiSessionInfo {
    pub session_id: String,
    pub target_id: String,
    pub skill: String,
    pub model: String,
    pub provider: String,
}

impl From<&DiagnoseSession> for AiSessionInfo {
    fn from(s: &DiagnoseSession) -> Self {
        Self {
            session_id: s.session_id.clone(),
            target_id: s.target_id.clone(),
            skill: s.skill.clone(),
            model: s.model.clone(),
            provider: s.provider.clone(),
        }
    }
}

#[tauri::command]
pub async fn ai_list_skills(state: State<'_, AppState>) -> AppResult<Vec<SkillRecord>> {
    skills::list_all(&state.db)
}

#[tauri::command]
pub async fn ai_get_skill(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<SkillRecord>> {
    skills::get(&state.db, &id)
}

#[tauri::command]
pub async fn ai_save_skill(
    state: State<'_, AppState>,
    id: String,
    name: String,
    description: String,
    content: String,
) -> AppResult<()> {
    skills::save_user(
        &state.db,
        &SkillRecord {
            id,
            name,
            description,
            content,
            builtin: false,
        },
    )
}

#[tauri::command]
pub async fn ai_delete_skill(state: State<'_, AppState>, id: String) -> AppResult<()> {
    skills::delete_user(&state.db, &id)
}

/// 把前端 locale code 映射为给 LLM 的语言名称（用于 prompt 末尾的 "Respond in X"）。
fn locale_label(locale: &str) -> &'static str {
    match locale {
        "zh" | "zh-CN" | "zh-Hans" => "Chinese (Simplified)",
        "zh-TW" | "zh-Hant" => "Chinese (Traditional)",
        _ => "English",
    }
}

#[tauri::command]
pub async fn ai_session_start(
    app: AppHandle,
    state: State<'_, AppState>,
    target_kind: String, // "ssh" | "local"
    target_id: String,
    skill: String,
    provider: String,
    model: String,
    locale: Option<String>,
) -> AppResult<AiSessionInfo> {
    {
        let g = locked(&state.ai_sessions)?;
        if g.values().any(|s| s.target_id == target_id) {
            return Err(AppError::coded(
                "session_already_exists",
                json!({ "target": target_id }),
            ));
        }
    }

    // 1. API key
    let api_key = state
        .secret_store
        .get(&key_api_key(&provider))?
        .ok_or_else(|| {
            AppError::coded("api_key_missing", json!({ "provider": provider }))
        })?;
    let endpoint = state.secret_store.get(&key_endpoint(&provider))?;

    // 2. 校验 target 存在 + 抓 SSH handle 给 download_file 工具复用
    let ssh_handle = match target_kind.as_str() {
        "ssh" => {
            let g = locked(&state.sessions)?;
            let h = g
                .get(&target_id)
                .ok_or_else(|| AppError::coded("ssh_session_not_found", json!({})))?;
            Some(h.ssh_handle().clone())
        }
        #[cfg(not(target_os = "android"))]
        "local" => {
            if !locked(&state.pty_sessions)?.contains_key(&target_id) {
                return Err(AppError::coded("local_pty_not_found", json!({})));
            }
            None
        }
        _ => {
            return Err(AppError::coded(
                "unknown_target_kind",
                json!({ "kind": target_kind }),
            ))
        }
    };

    let client = llm::build_client(&provider, api_key, endpoint)?;

    // system prompt = 内置 general 规则集 + user-skill 目录（id + description）。
    // user-skill 详细内容走 load_skill 工具按需加载（claude skills 模式），
    // 用户写多个 skill 也不会让启动 prompt 爆炸。
    let _ = skill; // 前端不再选；保留参数兼容
    let locale_lbl = locale_label(locale.as_deref().unwrap_or("en"));
    let system_prompt = skills::build_catalog_prompt(&state.db, locale_lbl)?;
    let user_skills_cache = skills::list_user(&state.db)?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let cfg = session::SessionConfig {
        session_id: session_id.clone(),
        target_id,
        skill: "general".to_string(),
        system_prompt,
        user_skills_cache,
        model,
        client,
        redact_rules: sanitize::default_rules(),
        max_output_bytes: sanitize::DEFAULT_MAX_OUTPUT_BYTES,
        ssh_handle,
        data_dir: state.data_dir.clone(),
    };

    let session = session::start(cfg, app)?;
    let info = AiSessionInfo::from(&session);
    locked(&state.ai_sessions)?.insert(session_id, session);
    Ok(info)
}

#[tauri::command]
pub async fn ai_user_message(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> AppResult<()> {
    let tx = locked(&state.ai_sessions)?
        .get(&session_id)
        .map(|s| s.action_tx.clone())
        .ok_or_else(|| AppError::coded("ai_session_not_found", json!({})))?;
    tx.send(UserAction::Message(text))
        .map_err(|_| AppError::coded("ai_session_stopped", json!({})))?;
    Ok(())
}

#[tauri::command]
pub async fn ai_command_result(
    state: State<'_, AppState>,
    session_id: String,
    tool_call_id: String,
    exit_code: i32,
    output: String,
    timed_out: bool,
) -> AppResult<()> {
    let tx = locked(&state.ai_sessions)?
        .get(&session_id)
        .map(|s| s.action_tx.clone())
        .ok_or_else(|| AppError::coded("ai_session_not_found", json!({})))?;
    tx.send(UserAction::CommandResult {
        tool_call_id,
        exit_code,
        output,
        timed_out,
    })
    .map_err(|_| AppError::coded("ai_session_stopped", json!({})))?;
    Ok(())
}

#[tauri::command]
pub async fn ai_command_reject(
    state: State<'_, AppState>,
    session_id: String,
    tool_call_id: String,
    reason: String,
) -> AppResult<()> {
    let tx = locked(&state.ai_sessions)?
        .get(&session_id)
        .map(|s| s.action_tx.clone())
        .ok_or_else(|| AppError::coded("ai_session_not_found", json!({})))?;
    tx.send(UserAction::RejectCommand {
        tool_call_id,
        reason,
    })
    .map_err(|_| AppError::coded("ai_session_stopped", json!({})))?;
    Ok(())
}

#[tauri::command]
pub async fn ai_session_stop(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<()> {
    let session = locked(&state.ai_sessions)?
        .remove(&session_id)
        .ok_or_else(|| AppError::coded("ai_session_not_found", json!({})))?;
    let _ = session.action_tx.send(UserAction::Stop);
    Ok(())
}

#[tauri::command]
pub async fn ai_audit_save(
    state: State<'_, AppState>,
    session_id: String,
    file_path: String,
) -> AppResult<()> {
    let audit = locked(&state.ai_sessions)?
        .get(&session_id)
        .map(|s| s.audit.clone())
        .ok_or_else(|| AppError::coded("ai_session_not_found", json!({})))?;
    let g = audit.lock().map_err(|_| AppError::Lock)?;
    g.save_to_file(&PathBuf::from(file_path))
        .map_err(AppError::Io)?;
    Ok(())
}

/// 弹原生 Save 对话框选路径，再保存。返回保存的路径；用户取消返回 None。
/// Android 无 rfd 依赖，返回未实现错误。
#[tauri::command]
pub async fn ai_audit_save_pick(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<Option<String>> {
    #[cfg(target_os = "android")]
    {
        let _ = (state, session_id);
        Err(AppError::coded("android_no_dialog", json!({})))
    }
    #[cfg(not(target_os = "android"))]
    {
        let audit = locked(&state.ai_sessions)?
            .get(&session_id)
            .map(|s| s.audit.clone())
            .ok_or_else(|| AppError::coded("ai_session_not_found", json!({})))?;

        let default_dir = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        let default_name = format!(
            "rssh-diagnose-{}-{}.log",
            &session_id[..session_id.len().min(8)],
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );

        let pick = rfd::AsyncFileDialog::new()
            .set_directory(default_dir)
            .set_file_name(default_name)
            .add_filter("Log", &["log", "txt"])
            .save_file()
            .await;

        let Some(handle) = pick else { return Ok(None) };
        let path = handle.path().to_path_buf();
        let g = audit.lock().map_err(|_| AppError::Lock)?;
        g.save_to_file(&path).map_err(AppError::Io)?;
        Ok(Some(path.to_string_lossy().into_owned()))
    }
}

#[tauri::command]
pub async fn ai_audit_get(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<super::audit::AuditLog> {
    let audit = locked(&state.ai_sessions)?
        .get(&session_id)
        .map(|s| s.audit.clone())
        .ok_or_else(|| AppError::coded("ai_session_not_found", json!({})))?;
    let g = audit.lock().map_err(|_| AppError::Lock)?;
    Ok(g.clone())
}

#[tauri::command]
pub async fn ai_list_sessions(state: State<'_, AppState>) -> AppResult<Vec<AiSessionInfo>> {
    let g = locked(&state.ai_sessions)?;
    Ok(g.values().map(AiSessionInfo::from).collect())
}

// ─── 设置（BYOK） ──────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
pub struct AiSettings {
    pub provider: String,
    pub model: String,
    pub endpoint: Option<String>,
    pub has_api_key: bool,
}

#[tauri::command]
pub async fn ai_settings_get(state: State<'_, AppState>) -> AppResult<AiSettings> {
    let provider = state
        .secret_store
        .get(&key_provider())?
        .unwrap_or_else(|| "anthropic".into());
    let model = state
        .secret_store
        .get(&key_model())?
        .unwrap_or_else(|| default_model_for(&provider).into());
    let endpoint = state.secret_store.get(&key_endpoint(&provider))?;
    let has_api_key = state
        .secret_store
        .get(&key_api_key(&provider))?
        .filter(|s| !s.is_empty())
        .is_some();
    Ok(AiSettings {
        provider,
        model,
        endpoint,
        has_api_key,
    })
}

fn default_model_for(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-sonnet-4-6",
        _ => "gpt-4o-mini",
    }
}

#[tauri::command]
pub async fn ai_settings_set(
    state: State<'_, AppState>,
    provider: Option<String>,
    model: Option<String>,
    endpoint: Option<String>,
    api_key: Option<String>,
) -> AppResult<()> {
    if let Some(p) = provider.as_ref() {
        state.secret_store.set(&key_provider(), p)?;
    }
    if let Some(m) = model.as_ref() {
        state.secret_store.set(&key_model(), m)?;
    }
    let active_provider = provider
        .clone()
        .or_else(|| state.secret_store.get(&key_provider()).ok().flatten())
        .unwrap_or_else(|| "anthropic".into());
    if let Some(e) = endpoint {
        state.secret_store.set(&key_endpoint(&active_provider), &e)?;
    }
    if let Some(k) = api_key {
        if k.is_empty() {
            state.secret_store.delete(&key_api_key(&active_provider))?;
        } else {
            state.secret_store.set(&key_api_key(&active_provider), &k)?;
        }
    }
    Ok(())
}
