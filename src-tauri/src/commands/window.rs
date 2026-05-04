use std::process::Command;

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

/// Open an external http(s) URL in the user's default browser.
/// Refuses non-http(s) schemes to prevent abuse (file://, javascript:, …).
#[tauri::command]
pub fn open_external_url(url: String) -> AppResult<()> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::config("window_non_https_url", serde_json::json!({ "url": url })));
    }

    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(&url).spawn();
    #[cfg(target_os = "linux")]
    let result = Command::new("xdg-open").arg(&url).spawn();
    #[cfg(target_os = "windows")]
    let result = Command::new("cmd").args(["/C", "start", "", &url]).spawn();

    result
        .map(|_| ())
        .map_err(|e| AppError::other("window_open_url_failed", serde_json::json!({ "err": e.to_string() })))
}

/// Fetch the latest release tag from a GitHub repo.
///
/// Hits the HTML page `https://github.com/{repo}/releases/latest` rather than
/// the JSON API. GitHub responds with a 302 redirect whose Location is
/// `/{repo}/releases/tag/<tag>` — we parse the tag from there.
///
/// Why not the API: `api.github.com` enforces a 60 req/h per-IP limit for
/// unauthenticated calls. Behind shared NAT (offices, VPNs) the quota is
/// burned by other users and we get HTTP 403. The HTML redirect path has no
/// such limit and no auth requirement.
///
/// `repo` must be of the form "owner/name". Returns the raw tag (e.g. "v1.2.3").
#[tauri::command]
pub async fn fetch_latest_release_tag(repo: String) -> AppResult<String> {
    if repo.is_empty() || !repo.contains('/') || repo.contains(char::is_whitespace) {
        return Err(AppError::config("window_invalid_repo", serde_json::json!({ "repo": repo })));
    }
    let url = format!("https://github.com/{repo}/releases/latest");
    let client = reqwest::Client::builder()
        .user_agent(concat!("rssh/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::other("window_http_failed", serde_json::json!({ "op": "client", "err": e.to_string() })))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::other("window_http_failed", serde_json::json!({ "op": "request", "err": e.to_string() })))?;

    let status = resp.status();
    if !status.is_redirection() {
        return Err(AppError::other(
            "window_redirect_status",
            serde_json::json!({ "status": status.to_string(), "body": "expected redirect" }),
        ));
    }
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::other("window_redirect_no_location", serde_json::json!({})))?;

    // Location is like "/owner/repo/releases/tag/v1.2.3" or full URL.
    location
        .rsplit_once("/releases/tag/")
        .map(|(_, tag)| tag.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| AppError::other("window_unexpected_redirect", serde_json::json!({ "location": location })))
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
