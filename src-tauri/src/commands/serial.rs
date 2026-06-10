use tauri::{AppHandle, Emitter, State};

use crate::error::{locked, AppError, AppResult};
use crate::models::SerialProfile;
use crate::state::AppState;
use crate::terminal::serial;

#[tauri::command]
pub fn serial_list_ports() -> AppResult<Vec<String>> {
    Ok(serial::available_ports())
}

#[tauri::command]
pub fn serial_open(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    port: String,
    config: serial::SerialConfig,
) -> AppResult<String> {
    // Turn transport-agnostic serial output into Tauri events. The headless ws
    // server builds a different sink over the same `serial::open`.
    let sink: serial::SerialSink =
        std::sync::Arc::new(move |id: &str, out: serial::SerialOut| match out {
            serial::SerialOut::Data(b) => {
                let _ = app.emit(&format!("serial:data:{id}"), b);
            }
            serial::SerialOut::Close => {
                let _ = app.emit(&format!("serial:close:{id}"), ());
            }
        });
    let (id, handle) = serial::open(&port, config, sink)?;
    locked(&state.serial_sessions)?.insert(id.clone(), handle);
    crate::commands::lifecycle::register_window_session(&state, window.label(), &id);
    Ok(id)
}

/// Look up an open serial session's handle (cloned — `SerialHandle` is Arc-backed).
fn serial_handle(state: &State<'_, AppState>, session_id: &str) -> AppResult<serial::SerialHandle> {
    locked(&state.serial_sessions)?
        .get(session_id)
        .cloned()
        .ok_or_else(|| AppError::not_found("serial_not_found", serde_json::json!({})))
}

#[tauri::command]
pub fn serial_write(
    state: State<'_, AppState>,
    session_id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    serial_handle(&state, &session_id)?.write(&data)
}

/// Drive the DTR control line (`true` = asserted). Manual line control for
/// MCU reset / bootloader entry / modem signalling.
#[tauri::command]
pub fn serial_set_dtr(
    state: State<'_, AppState>,
    session_id: String,
    level: bool,
) -> AppResult<()> {
    serial_handle(&state, &session_id)?.set_dtr(level)
}

/// Drive the RTS control line (`true` = asserted).
#[tauri::command]
pub fn serial_set_rts(
    state: State<'_, AppState>,
    session_id: String,
    level: bool,
) -> AppResult<()> {
    serial_handle(&state, &session_id)?.set_rts(level)
}

/// Send a serial BREAK pulse (~250ms) — attention/interrupt signal for U-Boot,
/// kernel SysRq-over-serial, telco gear.
#[tauri::command]
pub fn serial_send_break(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    serial_handle(&state, &session_id)?.send_break()
}

// No serial_resize: a serial line has no rows/cols. The frontend's transport
// table maps serial's resize entry to null, so it simply never calls it.

#[tauri::command]
pub fn serial_close(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    crate::commands::lifecycle::unregister_window_session(&state, &session_id);
    locked(&state.serial_sessions)?.remove(&session_id);
    Ok(())
}

// ── Saved serial profiles (peer of profile/forward; SQLite-persisted CRUD) ──

#[tauri::command]
pub fn list_serial_profiles(state: State<'_, AppState>) -> AppResult<Vec<SerialProfile>> {
    crate::db::serial_profile::list(&state.db)
}

#[tauri::command]
pub fn get_serial_profile(state: State<'_, AppState>, id: String) -> AppResult<SerialProfile> {
    crate::db::serial_profile::get(&state.db, &id)
}

#[tauri::command]
pub fn create_serial_profile(state: State<'_, AppState>, profile: SerialProfile) -> AppResult<()> {
    crate::db::serial_profile::insert(&state.db, &profile)
}

#[tauri::command]
pub fn update_serial_profile(state: State<'_, AppState>, profile: SerialProfile) -> AppResult<()> {
    crate::db::serial_profile::update(&state.db, &profile)
}

#[tauri::command]
pub fn delete_serial_profile(state: State<'_, AppState>, id: String) -> AppResult<()> {
    crate::db::serial_profile::delete(&state.db, &id)
}
