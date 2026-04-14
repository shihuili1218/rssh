use tauri::State;

use crate::error::{AppError, AppResult};
use crate::models::Forward;
use crate::ssh::forward as fwd;
use crate::state::AppState;

#[tauri::command]
pub fn list_forwards(state: State<AppState>) -> Result<Vec<Forward>, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::forward::list(&conn)
}

#[tauri::command]
pub fn get_forward(state: State<AppState>, id: String) -> Result<Forward, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::forward::get(&conn, &id)
}

#[tauri::command]
pub fn create_forward(state: State<AppState>, forward: Forward) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::forward::insert(&conn, &forward)
}

#[tauri::command]
pub fn update_forward(state: State<AppState>, forward: Forward) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::forward::update(&conn, &forward)
}

#[tauri::command]
pub fn delete_forward(state: State<AppState>, id: String) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::forward::delete(&conn, &id)
}

// ---------------------------------------------------------------------------
// 活跃端口转发 — 启动 / 停止
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn forward_start(
    state: State<'_, AppState>,
    forward_id: String,
) -> AppResult<String> {
    let (fwd_config, host, port, cred) = {
        let conn = state
            .db
            .lock()
            .map_err(|_| AppError::Other("db lock poisoned".into()))?;
        let f = crate::db::forward::get(&conn, &forward_id)?;
        let p = crate::db::profile::get(&conn, &f.profile_id)
            .map_err(|_| AppError::NotFound("转发关联的 Profile 不存在".into()))?;
        let cred_id = p.credential_id.as_deref().unwrap_or("");
        let c = crate::db::credential::get(&conn, cred_id)
            .map_err(|_| AppError::NotFound("转发关联的凭证不存在".into()))?;
        (f, p.host, p.port, c)
    };

    let known_hosts_path = state.data_dir.join("known_hosts");
    let handle = fwd::start_local(&fwd_config, &host, port, &cred, known_hosts_path).await?;
    let active_id = uuid::Uuid::new_v4().to_string();

    state
        .active_forwards
        .lock()
        .map_err(|_| AppError::Other("forward lock poisoned".into()))?
        .insert(active_id.clone(), handle);

    Ok(active_id)
}

#[tauri::command]
pub fn forward_stats(
    state: State<'_, AppState>,
    active_id: String,
) -> AppResult<fwd::ForwardStats> {
    let forwards = state
        .active_forwards
        .lock()
        .map_err(|_| AppError::Other("forward lock poisoned".into()))?;
    let handle = forwards
        .get(&active_id)
        .ok_or(AppError::NotFound("转发不存在".into()))?;
    Ok(handle.stats())
}

#[tauri::command]
pub fn forward_stop(state: State<'_, AppState>, active_id: String) -> AppResult<()> {
    let handle = state
        .active_forwards
        .lock()
        .map_err(|_| AppError::Other("forward lock poisoned".into()))?
        .remove(&active_id)
        .ok_or(AppError::NotFound("转发不存在".into()))?;
    handle.stop();
    Ok(())
}
