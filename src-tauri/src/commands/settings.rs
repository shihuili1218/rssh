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
pub fn set_setting(
    state: State<AppState>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    if crate::secret::is_secret_setting(&key) {
        return if value.is_empty() {
            state.secret_store.delete(&crate::secret::setting_key(&key))
        } else {
            state.secret_store.set(&crate::secret::setting_key(&key), &value)
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

#[tauri::command]
pub fn list_recordings(state: State<AppState>) -> AppResult<Vec<String>> {
    let dir = recording_dir(&state)?;
    if !dir.exists() { return Ok(vec![]); }
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
    let path = recording_dir(&state)?.join(&name);
    std::fs::read_to_string(&path).map_err(|e| AppError::Other(e.to_string()))
}

fn recording_dir(state: &State<AppState>) -> AppResult<std::path::PathBuf> {
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
