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
    // `clone` is a JSON string from the frontend; embed it as a literal
    // inside the init script by re-serializing to produce a valid JS string.
    let json_literal = serde_json::to_string(&clone)
        .map_err(|e| AppError::Other(format!("Failed to encode clone payload: {e}")))?;
    let init_script = format!("window.__rssh_clone = JSON.parse({});", json_literal);

    let label = format!("rssh-{}", Uuid::new_v4().simple());
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title("RSSH")
        .inner_size(1200.0, 800.0)
        .initialization_script(&init_script)
        .build()
        .map_err(|e| AppError::Other(format!("Failed to open window: {e}")))?;
    Ok(())
}
