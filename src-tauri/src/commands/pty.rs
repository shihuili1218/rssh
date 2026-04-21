use tauri::{AppHandle, State};

use crate::error::{locked, AppError, AppResult};
use crate::state::AppState;
use crate::terminal::pty;

#[tauri::command]
pub fn pty_spawn(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    cols: u16,
    rows: u16,
) -> AppResult<String> {
    let shell = crate::db::settings::get(&state.db, "local_shell")?.filter(|s| !s.is_empty());
    let (id, handle) = pty::spawn(cols, rows, app, shell)?;
    locked(&state.pty_sessions)?.insert(id.clone(), handle);
    crate::commands::lifecycle::register_window_session(&state, window.label(), &id);
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
    let handle = locked(&state.pty_sessions)?
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
    let handle = locked(&state.pty_sessions)?
        .get(&session_id)
        .cloned()
        .ok_or(AppError::NotFound("PTY 不存在".into()))?;
    handle.resize(cols, rows)
}

#[tauri::command]
pub fn pty_close(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    crate::commands::lifecycle::unregister_window_session(&state, &session_id);
    locked(&state.pty_sessions)?.remove(&session_id);
    Ok(())
}
