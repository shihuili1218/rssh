use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// Open a new in-process Tauri window with a clone payload.
/// The new window boots the same frontend; `AppShell` reads
/// `window.__rssh_clone` on mount and auto-creates the cloned tab.
///
/// Windows share `AppState` (sessions, DB, PTY registry) via `Arc<Mutex<..>>`,
/// so spawning a new window is cheap and does not fork the backend.
#[tauri::command]
pub fn open_tab_in_new_window(app: AppHandle, clone: String) -> AppResult<()> {
    // `clone` is a JSON string from the frontend; embed it as a JS string literal.
    // Frontend reads window.__rssh_clone as a string and JSON.parses it once.
    // Do NOT JSON.parse here — that would store an object, and the frontend's
    // JSON.parse(object) would coerce to "[object Object]" and throw.
    let json_literal = serde_json::to_string(&clone)
        .map_err(|e| AppError::other("window_clone_encode_failed", serde_json::json!({ "err": e.to_string() })))?;
    let init_script = format!("window.__rssh_clone = {};", json_literal);

    let label = format!("rssh-{}", Uuid::new_v4().simple());
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title("RSSH")
        .inner_size(1200.0, 800.0)
        .initialization_script(&init_script)
        .build()
        .map_err(|e| AppError::other("window_open_failed", serde_json::json!({ "err": e.to_string() })))?;
    Ok(())
}

/// Read the system clipboard as text.
/// Goes through Rust (arboard) to bypass WebKit's permission prompt on
/// externally-sourced clipboard content — `navigator.clipboard.readText()`
/// pops a dialog every time on macOS unless the content was written by the
/// same page in this session.
#[tauri::command]
pub fn clipboard_read() -> AppResult<String> {
    let mut cb =
        arboard::Clipboard::new().map_err(|e| AppError::other("window_clipboard_failed", serde_json::json!({ "op": "init", "err": e.to_string() })))?;
    cb.get_text()
        .map_err(|e| AppError::other("window_clipboard_failed", serde_json::json!({ "op": "read", "err": e.to_string() })))
}
