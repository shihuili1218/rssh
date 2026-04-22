use tauri::State;

use crate::error::{locked, AppError, AppResult};
use crate::models::{Forward, ForwardType};
use crate::ssh::forward as fwd;
use crate::state::AppState;

#[tauri::command]
pub fn list_forwards(state: State<AppState>) -> Result<Vec<Forward>, AppError> {
    crate::db::forward::list(&state.db)
}

#[tauri::command]
pub fn get_forward(state: State<AppState>, id: String) -> Result<Forward, AppError> {
    crate::db::forward::get(&state.db, &id)
}

#[tauri::command]
pub fn create_forward(state: State<AppState>, forward: Forward) -> Result<(), AppError> {
    crate::db::forward::insert(&state.db, &forward)
}

#[tauri::command]
pub fn update_forward(state: State<AppState>, forward: Forward) -> Result<(), AppError> {
    crate::db::forward::update(&state.db, &forward)
}

#[tauri::command]
pub fn delete_forward(state: State<AppState>, id: String) -> Result<(), AppError> {
    crate::db::forward::delete(&state.db, &id)
}

// ---------------------------------------------------------------------------
// 活跃端口转发 — 启动 / 停止
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn forward_start(
    state: State<'_, AppState>,
    forward_id: String,
) -> AppResult<String> {
    let f = crate::db::forward::get(&state.db, &forward_id)?;
    let p = crate::db::profile::get(&state.db, &f.profile_id)
        .map_err(|_| AppError::NotFound("转发关联的 Profile 不存在".into()))?;
    let cred_id = p.credential_id.as_deref().unwrap_or("");
    let mut c = crate::db::credential::get(&state.db, cred_id)
        .map_err(|_| AppError::NotFound("转发关联的凭证不存在".into()))?;
    if !c.id.is_empty() {
        c.secret = state.secret_store.get(&crate::secret::cred_secret_key(&c.id))?;
        c.passphrase = state.secret_store.get(&crate::secret::cred_passphrase_key(&c.id))?;
    }
    let timeout_secs: u64 = crate::db::settings::get(&state.db, "connect_timeout")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(crate::ssh::client::DEFAULT_CONNECT_TIMEOUT);

    let known_hosts_path = crate::ssh::known_hosts::path_for(&state.data_dir);
    let handle = match f.forward_type {
        ForwardType::Local => fwd::start_local(&f, &p.host, p.port, &c, known_hosts_path, timeout_secs).await?,
        ForwardType::Remote => fwd::start_remote(&f, &p.host, p.port, &c, known_hosts_path, timeout_secs).await?,
        ForwardType::Dynamic => fwd::start_dynamic(&f, &p.host, p.port, &c, known_hosts_path, timeout_secs).await?,
    };
    let active_id = uuid::Uuid::new_v4().to_string();

    locked(&state.active_forwards)?.insert(active_id.clone(), handle);

    Ok(active_id)
}

#[tauri::command]
pub fn forward_stats(
    state: State<'_, AppState>,
    active_id: String,
) -> AppResult<fwd::ForwardStats> {
    let forwards = locked(&state.active_forwards)?;
    let handle = forwards
        .get(&active_id)
        .ok_or(AppError::NotFound("转发不存在".into()))?;
    Ok(handle.stats())
}

#[tauri::command]
pub fn forward_stop(state: State<'_, AppState>, active_id: String) -> AppResult<()> {
    let handle = locked(&state.active_forwards)?
        .remove(&active_id)
        .ok_or(AppError::NotFound("转发不存在".into()))?;
    handle.stop();
    Ok(())
}
