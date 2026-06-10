//! Update / release-tag check.
//!
//! Cross-platform: only uses `reqwest`, no system integration. About-screen
//! and any future "new version available" UI lives here. Kept separate from
//! `commands::window` (desktop-only system-window plumbing).

use crate::error::{AppError, AppResult};

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
        return Err(AppError::config(
            "update_invalid_repo",
            serde_json::json!({ "repo": repo }),
        ));
    }
    let url = format!("https://github.com/{repo}/releases/latest");
    let client = reqwest::Client::builder()
        .user_agent(concat!("rssh/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| {
            AppError::other(
                "update_http_failed",
                serde_json::json!({ "op": "client", "err": e.to_string() }),
            )
        })?;

    let resp = client.get(&url).send().await.map_err(|e| {
        AppError::other(
            "update_http_failed",
            serde_json::json!({ "op": "request", "err": e.to_string() }),
        )
    })?;

    let status = resp.status();
    if !status.is_redirection() {
        return Err(AppError::other(
            "update_redirect_status",
            serde_json::json!({ "status": status.to_string(), "body": "expected redirect" }),
        ));
    }
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::other("update_redirect_no_location", serde_json::json!({})))?;

    // Location is like "/owner/repo/releases/tag/v1.2.3" or full URL.
    location
        .rsplit_once("/releases/tag/")
        .map(|(_, tag)| tag.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            AppError::other(
                "update_unexpected_redirect",
                serde_json::json!({ "location": location }),
            )
        })
}
