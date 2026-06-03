use tauri::State;

use crate::error::{AppError, AppResult};
use crate::models::{HighlightRule, Snippet};
use crate::state::AppState;

#[tauri::command]
pub fn get_setting(state: State<AppState>, key: String) -> Result<Option<String>, AppError> {
    if crate::secret::is_secret_setting(&key) {
        return state.secret_store.get(&crate::secret::setting_key(&key));
    }
    crate::db::settings::get(&state.db, &key)
}

#[tauri::command]
pub fn set_setting(state: State<AppState>, key: String, value: String) -> Result<(), AppError> {
    if crate::secret::is_secret_setting(&key) {
        return if value.is_empty() {
            state.secret_store.delete(&crate::secret::setting_key(&key))
        } else {
            state
                .secret_store
                .set(&crate::secret::setting_key(&key), &value)
        };
    }
    crate::db::settings::set(&state.db, &key, &value)
}

#[tauri::command]
pub fn list_highlights(state: State<AppState>) -> Result<Vec<HighlightRule>, AppError> {
    crate::db::highlight::list(&state.db)
}

#[tauri::command]
pub fn add_highlight(state: State<AppState>, rule: HighlightRule) -> Result<(), AppError> {
    crate::db::highlight::insert(&state.db, &rule)
}

#[tauri::command]
pub fn remove_highlight(state: State<AppState>, keyword: String) -> Result<(), AppError> {
    crate::db::highlight::delete_by_keyword(&state.db, &keyword)
}

#[tauri::command]
pub fn update_highlight(
    state: State<AppState>,
    old_keyword: String,
    rule: HighlightRule,
) -> Result<(), AppError> {
    crate::db::highlight::update(&state.db, &old_keyword, &rule)
}

#[tauri::command]
pub fn load_snippets(state: State<AppState>) -> Result<Vec<Snippet>, AppError> {
    crate::db::snippet::load(&state.data_dir)
}

#[tauri::command]
pub fn save_snippets(state: State<AppState>, snippets: Vec<Snippet>) -> Result<(), AppError> {
    crate::db::snippet::save(&state.data_dir, &snippets)
}

#[tauri::command]
pub fn reset_highlights(state: State<AppState>) -> Result<(), AppError> {
    crate::db::highlight::reset_defaults(&state.db)
}

/// 当前 secret 存储后端的名字（"macos-keychain" / "windows-credential-manager" /
/// "linux-secret-service" / "db"）。前端用来显示"凭证存哪儿"。
#[tauri::command]
pub fn secret_backend(state: State<AppState>) -> String {
    state.secret_store.backend_name().to_string()
}

/// One installed font family + whether it is monospaced. The frontend uses
/// `monospaced` as a client-side filter (the "monospace only" toggle) and
/// prepends the chosen family to the terminal's base font stack.
#[derive(serde::Serialize)]
pub struct FontInfo {
    pub family: String,
    pub monospaced: bool,
}

/// List installed font families for the terminal-font picker. Collapses faces
/// to families; a family counts as monospaced if any of its faces reports the
/// fixed-pitch flag. Sorted + deduped via BTreeMap. Pure system query — no
/// state, no persistence. WKWebView has no Local Font Access API, so font
/// enumeration must happen here in Rust rather than in the webview.
#[tauri::command]
pub async fn list_fonts() -> Vec<FontInfo> {
    // Enumeration scans the system font dirs and parses face headers — blocking
    // work, so run it off the async runtime's worker threads (keeps other
    // commands responsive). Sync Tauri commands already run off the UI thread,
    // so this is correctness/tidiness, not a UI-freeze fix.
    tauri::async_runtime::spawn_blocking(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut families: std::collections::BTreeMap<String, bool> =
            std::collections::BTreeMap::new();
        for face in db.faces() {
            if let Some((name, _)) = face.families.first() {
                *families.entry(name.clone()).or_insert(false) |= face.monospaced;
            }
        }
        families
            .into_iter()
            .map(|(family, monospaced)| FontInfo { family, monospaced })
            .collect()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
pub fn list_recordings(state: State<AppState>) -> AppResult<Vec<String>> {
    list_recordings_impl(&state)
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub fn list_recordings_impl(state: &AppState) -> AppResult<Vec<String>> {
    let dir = recording_dir(state)?;
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut files: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "cast").unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    files.sort_by(|a, b| b.cmp(a)); // newest first
    Ok(files)
}

#[tauri::command]
pub fn read_recording(state: State<AppState>, name: String) -> AppResult<String> {
    read_recording_impl(&state, name)
}

/// A valid recording name is exactly what `list_recordings_impl` hands back: a
/// bare filename living directly in the recordings dir. Anything carrying path
/// separators, `..`, or an absolute prefix differs from its own `file_name()`,
/// so this one comparison rejects every traversal/escape shape with no special
/// cases.
fn is_safe_recording_name(name: &str) -> bool {
    std::path::Path::new(name).file_name() == Some(std::ffi::OsStr::new(name))
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub fn read_recording_impl(state: &AppState, name: String) -> AppResult<String> {
    // Confine reads to the recordings dir. The headless server routes client
    // requests here verbatim, so an unchecked `name` ("../../etc/passwd", an
    // absolute path, …) would read arbitrary files the process can reach.
    if !is_safe_recording_name(&name) {
        return Err(AppError::config(
            "invalid_recording_name",
            serde_json::json!({ "name": name }),
        ));
    }
    let path = recording_dir(state)?.join(&name);
    std::fs::read_to_string(&path).map_err(|e| AppError::other("settings_read_failed", serde_json::json!({ "err": e.to_string() })))
}

fn recording_dir(state: &AppState) -> AppResult<std::path::PathBuf> {
    let dir_str = crate::db::settings::get(&state.db, "recording_dir")?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            dirs::document_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("rssh-recordings")
                .to_string_lossy()
                .into_owned()
        });
    Ok(std::path::PathBuf::from(dir_str))
}

#[cfg(test)]
mod tests {
    use super::is_safe_recording_name;

    #[test]
    fn accepts_bare_recording_filenames() {
        assert!(is_safe_recording_name("session_20260603_120000.cast"));
        assert!(is_safe_recording_name("my profile_1.cast"));
        assert!(is_safe_recording_name("会话.cast"));
    }

    #[test]
    fn rejects_traversal_and_escapes() {
        // `/`, `..`, and absolute paths escape on every platform. (Backslash is a
        // plain filename char on Unix, so it's intentionally not asserted here.)
        for bad in [
            "",
            ".",
            "..",
            "../secret.cast",
            "../../etc/passwd",
            "sub/dir.cast",
            "/etc/passwd",
            "/abs.cast",
        ] {
            assert!(!is_safe_recording_name(bad), "should reject {bad:?}");
        }
    }
}
