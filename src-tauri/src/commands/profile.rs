use tauri::State;

use crate::error::AppError;
use crate::models::{Credential, Profile};
use crate::state::AppState;

#[tauri::command]
pub fn list_profiles(state: State<AppState>) -> Result<Vec<Profile>, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::profile::list(&conn)
}

#[tauri::command]
pub fn get_profile(state: State<AppState>, id: String) -> Result<Profile, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::profile::get(&conn, &id)
}

#[tauri::command]
pub fn create_profile(state: State<AppState>, profile: Profile) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::profile::insert(&conn, &profile)
}

#[tauri::command]
pub fn update_profile(state: State<AppState>, profile: Profile) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::profile::update(&conn, &profile)
}

#[tauri::command]
pub fn delete_profile(state: State<AppState>, id: String) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::profile::delete(&conn, &id)
}

#[tauri::command]
pub fn list_credentials(state: State<AppState>) -> Result<Vec<Credential>, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::credential::list(&conn)
}

#[tauri::command]
pub fn get_credential(state: State<AppState>, id: String) -> Result<Credential, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::credential::get(&conn, &id)
}

#[tauri::command]
pub fn create_credential(state: State<AppState>, credential: Credential) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::credential::insert(&conn, &credential)
}

#[tauri::command]
pub fn update_credential(state: State<AppState>, credential: Credential) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::credential::update(&conn, &credential)
}

#[tauri::command]
pub fn delete_credential(state: State<AppState>, id: String) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Other(e.to_string()))?;
    crate::db::credential::delete(&conn, &id)
}

#[tauri::command]
pub fn import_ssh_config(content: String) -> Vec<crate::ssh::config::SshConfigEntry> {
    crate::ssh::config::parse(&content)
}
