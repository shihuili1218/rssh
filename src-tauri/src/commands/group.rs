use tauri::State;

use crate::error::AppError;
use crate::models::Group;
use crate::state::AppState;

#[tauri::command]
pub fn list_groups(state: State<AppState>) -> Result<Vec<Group>, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::group::list(&conn)
}

#[tauri::command]
pub fn create_group(state: State<AppState>, group: Group) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::group::insert(&conn, &group)
}

#[tauri::command]
pub fn update_group(state: State<AppState>, group: Group) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::group::update(&conn, &group)
}

#[tauri::command]
pub fn delete_group(state: State<AppState>, id: String) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::group::delete(&conn, &id)
}
