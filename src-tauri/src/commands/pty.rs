use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::terminal::pty;

#[tauri::command]
pub fn pty_spawn(
    app: AppHandle,
    state: State<'_, AppState>,
    cols: u16,
    rows: u16,
) -> AppResult<String> {
    let shell = {
        let conn = state.db.lock().map_err(|_| AppError::Other("lock".into()))?;
        crate::db::settings::get(&conn, "local_shell")?.filter(|s| !s.is_empty())
    };
    let (id, handle) = pty::spawn(cols, rows, app, shell)?;
    state
        .pty_sessions
        .lock()
        .map_err(|_| AppError::Other("pty lock poisoned".into()))?
        .insert(id.clone(), handle);
    Ok(id)
}

#[tauri::command]
pub fn list_shells() -> Vec<String> {
    pty::available_shells().iter().map(|s| s.to_string()).collect()
}

#[tauri::command]
pub fn pty_write(
    state: State<'_, AppState>,
    session_id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    let handle = state
        .pty_sessions
        .lock()
        .map_err(|_| AppError::Other("pty lock poisoned".into()))?
        .get(&session_id)
        .cloned()
        .ok_or(AppError::NotFound("PTY 不存在".into()))?;
    handle.write(&data)
}

#[tauri::command]
pub fn pty_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> AppResult<()> {
    let handle = state
        .pty_sessions
        .lock()
        .map_err(|_| AppError::Other("pty lock poisoned".into()))?
        .get(&session_id)
        .cloned()
        .ok_or(AppError::NotFound("PTY 不存在".into()))?;
    handle.resize(cols, rows)
}

#[tauri::command]
pub fn pty_close(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    state
        .pty_sessions
        .lock()
        .map_err(|_| AppError::Other("pty lock poisoned".into()))?
        .remove(&session_id);
    Ok(())
}
