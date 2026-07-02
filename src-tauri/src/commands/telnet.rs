use tauri::{AppHandle, Emitter, State};

use crate::error::{locked, AppError, AppResult};
use crate::state::AppState;
use crate::terminal::telnet;

/// Async on purpose: DNS resolution + TCP connect can block for up to 10s per
/// address. A sync command would sit on the main thread and freeze the UI, so
/// the blocking connect runs on a worker via spawn_blocking.
#[tauri::command]
pub async fn telnet_open(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    host: String,
    port: u16,
) -> AppResult<String> {
    // Turn transport-agnostic telnet output into Tauri events. The headless ws
    // server builds a different sink over the same `telnet::open`.
    let sink: telnet::TelnetSink =
        std::sync::Arc::new(move |id: &str, out: telnet::TelnetOut| match out {
            telnet::TelnetOut::Data(b) => {
                let _ = app.emit(&format!("telnet:data:{id}"), b);
            }
            telnet::TelnetOut::Close => {
                let _ = app.emit(&format!("telnet:close:{id}"), ());
            }
        });
    let (id, handle) =
        tauri::async_runtime::spawn_blocking(move || telnet::open(&host, port, sink))
            .await
            .map_err(|e| {
                AppError::other(
                    "task_join_failed",
                    serde_json::json!({ "err": e.to_string() }),
                )
            })??;
    locked(&state.telnet_sessions)?.insert(id.clone(), handle);
    crate::commands::lifecycle::register_window_session(&state, window.label(), &id);
    Ok(id)
}

/// Look up an open telnet session's handle (cloned — `TelnetHandle` is Arc-backed).
fn telnet_handle(state: &State<'_, AppState>, session_id: &str) -> AppResult<telnet::TelnetHandle> {
    locked(&state.telnet_sessions)?
        .get(session_id)
        .cloned()
        .ok_or_else(|| AppError::not_found("telnet_not_found", serde_json::json!({})))
}

#[tauri::command]
pub fn telnet_write(
    state: State<'_, AppState>,
    session_id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    telnet_handle(&state, &session_id)?.write(&data)
}

/// Report the terminal size to the server (NAWS). Unlike serial, telnet HAS
/// rows/cols; before the server activates NAWS this is a silent no-op.
#[tauri::command]
pub fn telnet_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> AppResult<()> {
    telnet_handle(&state, &session_id)?.resize(cols, rows)
}

#[tauri::command]
pub fn telnet_close(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    crate::commands::lifecycle::unregister_window_session(&state, &session_id);
    locked(&state.telnet_sessions)?.remove(&session_id);
    Ok(())
}
