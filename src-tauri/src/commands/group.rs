use tauri::State;

use crate::error::AppError;
use crate::models::Group;
use crate::state::AppState;

#[tauri::command]
pub fn list_groups(state: State<AppState>) -> Result<Vec<Group>, AppError> {
    crate::db::group::list(&state.db)
}

#[tauri::command]
pub fn create_group(state: State<AppState>, group: Group) -> Result<(), AppError> {
    crate::db::group::insert(&state.db, &group)
}

#[tauri::command]
pub fn update_group(state: State<AppState>, group: Group) -> Result<(), AppError> {
    crate::db::group::update(&state.db, &group)
}

#[tauri::command]
pub fn delete_group(state: State<AppState>, id: String) -> Result<(), AppError> {
    crate::db::group::delete(&state.db, &id)
}
