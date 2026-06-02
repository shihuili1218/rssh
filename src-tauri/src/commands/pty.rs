use tauri::{AppHandle, Emitter, State};

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
    // Turn transport-agnostic PTY output into Tauri events. The headless ws
    // server builds a different sink over the same `pty::spawn`.
    let sink: pty::PtySink = std::sync::Arc::new(move |id: &str, out: pty::PtyOut| match out {
        pty::PtyOut::Data(b) => {
            let _ = app.emit(&format!("pty:data:{id}"), b);
        }
        pty::PtyOut::Close => {
            let _ = app.emit(&format!("pty:close:{id}"), ());
        }
    });
    let (id, handle) = pty::spawn(cols, rows, sink, shell)?;
    locked(&state.pty_sessions)?.insert(id.clone(), handle);
    crate::commands::lifecycle::register_window_session(&state, window.label(), &id);
    Ok(id)
}

// list_shells / refresh_shells 当前没有可失败的内部操作（lock poison 在
// pty 模块内部静默回退到现场扫描），返 AppResult 是为了遵循项目"所有 tauri
// command 走 AppResult"的统一约定 —— 将来要往里塞会失败的扫描器时，签名
// 不用动，前端 error 处理路径已通。
#[tauri::command]
pub fn list_shells() -> AppResult<Vec<String>> {
    Ok(pty::available_shells())
}

/// Shell 设置页"刷新"按钮触发：用户装了新 shell 后不必重启 app。
/// 同步重扫一遍（< 1ms），返回最新列表，前端直接覆盖下拉。
#[tauri::command]
pub fn refresh_shells() -> AppResult<Vec<String>> {
    pty::refresh_available_shells();
    Ok(pty::available_shells())
}

#[tauri::command]
pub fn pty_write(state: State<'_, AppState>, session_id: String, data: Vec<u8>) -> AppResult<()> {
    let handle = locked(&state.pty_sessions)?
        .get(&session_id)
        .cloned()
        .ok_or_else(|| AppError::not_found("pty_not_found", serde_json::json!({})))?;
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
        .ok_or_else(|| AppError::not_found("pty_not_found", serde_json::json!({})))?;
    handle.resize(cols, rows)
}

#[tauri::command]
pub fn pty_close(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    crate::commands::lifecycle::unregister_window_session(&state, &session_id);
    locked(&state.pty_sessions)?.remove(&session_id);
    Ok(())
}
