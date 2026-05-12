use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};

/// Open an external http(s) URL in the user's default browser/app.
///
/// Goes through `tauri-plugin-opener` so the same call works on desktop,
/// Android and iOS — on mobile the old `Command::new("open" | "xdg-open" | "cmd")`
/// route had no implementation and the invoke silently failed for users.
///
/// Refuses non-http(s) schemes to prevent abuse (file://, javascript:, …).
#[tauri::command]
pub fn open_external_url(app: AppHandle, url: String) -> AppResult<()> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::config("window_non_https_url", serde_json::json!({ "url": url })));
    }
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| AppError::other("window_open_url_failed", serde_json::json!({ "err": e.to_string() })))
}
