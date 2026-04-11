use tauri::State;

use crate::error::{AppError, AppResult};
use crate::models::{HighlightRule, Snippet};
use crate::state::AppState;

#[tauri::command]
pub fn get_setting(state: State<AppState>, key: String) -> Result<Option<String>, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::settings::get(&conn, &key)
}

#[tauri::command]
pub fn set_setting(
    state: State<AppState>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::settings::set(&conn, &key, &value)
}

#[tauri::command]
pub fn list_highlights(state: State<AppState>) -> Result<Vec<HighlightRule>, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::highlight::list(&conn)
}

#[tauri::command]
pub fn add_highlight(state: State<AppState>, rule: HighlightRule) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::highlight::insert(&conn, &rule)
}

#[tauri::command]
pub fn remove_highlight(state: State<AppState>, keyword: String) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::highlight::delete_by_keyword(&conn, &keyword)
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
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::highlight::reset_defaults(&conn)
}

#[tauri::command]
pub fn list_recordings(state: State<AppState>) -> AppResult<Vec<String>> {
    let conn = state.db.lock().map_err(|_| AppError::Other("lock".into()))?;
    let dir_str = crate::db::settings::get(&conn, "recording_dir")?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            dirs::document_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("rssh-recordings")
                .to_string_lossy()
                .into_owned()
        });
    let dir = std::path::PathBuf::from(&dir_str);
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
    let conn = state.db.lock().map_err(|_| AppError::Other("lock".into()))?;
    let dir_str = crate::db::settings::get(&conn, "recording_dir")?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            dirs::document_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("rssh-recordings")
                .to_string_lossy()
                .into_owned()
        });
    let path = std::path::PathBuf::from(&dir_str).join(&name);
    std::fs::read_to_string(&path).map_err(|e| AppError::Other(e.to_string()))
}
