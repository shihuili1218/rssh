use tauri::State;

use crate::error::AppError;
use crate::models::{Credential, Profile};
use crate::secret::{cred_passphrase_key, cred_secret_key};
use crate::state::AppState;

#[tauri::command]
pub fn list_profiles(state: State<AppState>) -> Result<Vec<Profile>, AppError> {
    crate::db::profile::list(&state.db)
}

#[tauri::command]
pub fn get_profile(state: State<AppState>, id: String) -> Result<Profile, AppError> {
    crate::db::profile::get(&state.db, &id)
}

#[tauri::command]
pub fn create_profile(state: State<AppState>, profile: Profile) -> Result<(), AppError> {
    crate::db::profile::insert(&state.db, &profile)
}

#[tauri::command]
pub fn update_profile(state: State<AppState>, profile: Profile) -> Result<(), AppError> {
    crate::db::profile::update(&state.db, &profile)
}

#[tauri::command]
pub fn delete_profile(state: State<AppState>, id: String) -> Result<(), AppError> {
    crate::db::profile::delete(&state.db, &id)
}

// ---------------------------------------------------------------------------
// Credentials — secret/passphrase 走 SecretStore，metadata 走 DB
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_credentials(state: State<AppState>) -> Result<Vec<Credential>, AppError> {
    // 列表场景不返回 secret，避免无谓 keychain 查询
    crate::db::credential::list(&state.db)
}

#[tauri::command]
pub fn get_credential(state: State<AppState>, id: String) -> Result<Credential, AppError> {
    let mut cred = crate::db::credential::get(&state.db, &id)?;
    cred.secret = state.secret_store.get(&cred_secret_key(&id))?;
    cred.passphrase = state.secret_store.get(&cred_passphrase_key(&id))?;
    Ok(cred)
}

#[tauri::command]
pub fn create_credential(state: State<AppState>, credential: Credential) -> Result<(), AppError> {
    crate::db::credential::insert(&state.db, &credential)?;
    save_credential_secrets(&state, &credential)
}

#[tauri::command]
pub fn update_credential(state: State<AppState>, credential: Credential) -> Result<(), AppError> {
    crate::db::credential::update(&state.db, &credential)?;
    save_credential_secrets(&state, &credential)
}

#[tauri::command]
pub fn delete_credential(state: State<AppState>, id: String) -> Result<(), AppError> {
    crate::db::credential::delete(&state.db, &id)?;
    state.secret_store.delete(&cred_secret_key(&id))?;
    state.secret_store.delete(&cred_passphrase_key(&id))?;
    Ok(())
}

fn save_credential_secrets(state: &State<AppState>, c: &Credential) -> Result<(), AppError> {
    let secret_key = cred_secret_key(&c.id);
    let passphrase_key = cred_passphrase_key(&c.id);
    match c.secret.as_deref() {
        Some(s) if !s.is_empty() => state.secret_store.set(&secret_key, s)?,
        _ => state.secret_store.delete(&secret_key)?,
    }
    match c.passphrase.as_deref() {
        Some(s) if !s.is_empty() => state.secret_store.set(&passphrase_key, s)?,
        _ => state.secret_store.delete(&passphrase_key)?,
    }
    Ok(())
}

#[tauri::command]
pub fn import_ssh_config(content: String) -> Vec<crate::ssh::config::SshConfigEntry> {
    crate::ssh::config::parse(&content)
}
