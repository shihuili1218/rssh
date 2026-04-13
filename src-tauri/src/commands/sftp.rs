use std::sync::Arc;

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::models::{Credential, CredentialType};
use crate::ssh::sftp::{RemoteEntry, SftpHandle};
use crate::state::AppState;

#[tauri::command]
pub async fn sftp_connect(
    state: State<'_, AppState>,
    host: String,
    port: u16,
    username: String,
    auth_type: String,
    secret: Option<String>,
) -> AppResult<String> {
    let cred = Credential {
        id: String::new(),
        name: String::new(),
        username,
        credential_type: CredentialType::from_str(&auth_type),
        secret,
        save_to_remote: false,
    };

    let known_hosts_path = state.data_dir.join("known_hosts");
    let handle = SftpHandle::connect(&host, port, &cred, known_hosts_path).await?;
    let id = uuid::Uuid::new_v4().to_string();

    state
        .sftp_sessions
        .lock()
        .map_err(|_| AppError::Other("sftp lock poisoned".into()))?
        .insert(id.clone(), Arc::new(handle));

    Ok(id)
}

/// 从 Mutex 中 clone 出 Arc<SftpHandle>，释放锁后再 await。
fn get_sftp(state: &State<'_, AppState>, sftp_id: &str) -> AppResult<Arc<SftpHandle>> {
    state
        .sftp_sessions
        .lock()
        .map_err(|_| AppError::Other("sftp lock poisoned".into()))?
        .get(sftp_id)
        .cloned()
        .ok_or(AppError::NotFound("SFTP 会话不存在".into()))
}

#[tauri::command]
pub async fn sftp_home(state: State<'_, AppState>, sftp_id: String) -> AppResult<String> {
    let h = get_sftp(&state, &sftp_id)?;
    h.home_dir().await
}

#[tauri::command]
pub async fn sftp_list(
    state: State<'_, AppState>,
    sftp_id: String,
    path: String,
) -> AppResult<Vec<RemoteEntry>> {
    let h = get_sftp(&state, &sftp_id)?;
    h.list_dir(&path).await
}

#[tauri::command]
pub async fn sftp_download(
    state: State<'_, AppState>,
    sftp_id: String,
    path: String,
) -> AppResult<Vec<u8>> {
    let h = get_sftp(&state, &sftp_id)?;
    h.download(&path).await
}

#[tauri::command]
pub async fn sftp_upload(
    state: State<'_, AppState>,
    sftp_id: String,
    path: String,
    data: Vec<u8>,
) -> AppResult<()> {
    let h = get_sftp(&state, &sftp_id)?;
    h.upload(&path, &data).await
}

#[tauri::command]
pub async fn sftp_mkdir(
    state: State<'_, AppState>,
    sftp_id: String,
    path: String,
) -> AppResult<()> {
    let h = get_sftp(&state, &sftp_id)?;
    h.mkdir(&path).await
}

#[tauri::command]
pub async fn sftp_close(state: State<'_, AppState>, sftp_id: String) -> AppResult<()> {
    state
        .sftp_sessions
        .lock()
        .map_err(|_| AppError::Other("sftp lock poisoned".into()))?
        .remove(&sftp_id);
    Ok(())
}

/// Download a remote file via a native Save As dialog.
/// Returns the saved local path, or None if the user cancelled.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_save_file(
    state: State<'_, AppState>,
    sftp_id: String,
    remote_path: String,
    default_name: String,
) -> AppResult<Option<String>> {
    let save_path = rfd::AsyncFileDialog::new()
        .set_file_name(&default_name)
        .save_file()
        .await;

    let Some(handle) = save_path else { return Ok(None) };
    let local = handle.path().to_path_buf();

    let sftp = get_sftp(&state, &sftp_id)?;
    let data = sftp.download(&remote_path).await?;
    std::fs::write(&local, &data)?;
    Ok(Some(local.display().to_string()))
}

/// Pick a local file via a native Open dialog and upload it into `remote_dir`.
/// Returns the uploaded filename, or None if cancelled.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_pick_and_upload(
    state: State<'_, AppState>,
    sftp_id: String,
    remote_dir: String,
) -> AppResult<Option<String>> {
    let pick = rfd::AsyncFileDialog::new().pick_file().await;
    let Some(handle) = pick else { return Ok(None) };
    let local = handle.path().to_path_buf();

    let name = local
        .file_name()
        .ok_or_else(|| AppError::Other("Invalid filename".into()))?
        .to_string_lossy()
        .into_owned();
    let remote_path = if remote_dir == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", remote_dir.trim_end_matches('/'), name)
    };

    let data = std::fs::read(&local)?;
    let sftp = get_sftp(&state, &sftp_id)?;
    sftp.upload(&remote_path, &data).await?;
    Ok(Some(name))
}
