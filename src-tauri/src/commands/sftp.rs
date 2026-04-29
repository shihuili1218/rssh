use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tauri::State;

use crate::error::{locked, AppError, AppResult};
use crate::models::{Credential, CredentialType};
use crate::ssh::sftp::{RemoteEntry, SftpHandle};
use crate::state::AppState;

/// 注册一个 cancel flag 给 transfer_id；返回需要传给 streaming 函数的 Arc。
/// 调用方负责在结束（成功 / 失败 / 取消）时 unregister，否则会泄漏到下次重启。
fn register_cancel_flag(state: &State<'_, AppState>, transfer_id: &str) -> AppResult<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    locked(&state.transfer_cancels)?.insert(transfer_id.to_string(), flag.clone());
    Ok(flag)
}

fn unregister_cancel_flag(state: &State<'_, AppState>, transfer_id: &str) {
    if let Ok(mut m) = locked(&state.transfer_cancels) {
        m.remove(transfer_id);
    };
}

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

    let timeout_secs: u64 = crate::db::settings::get(&state.db, "connect_timeout")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(crate::ssh::client::DEFAULT_CONNECT_TIMEOUT);

    let known_hosts_path = crate::ssh::known_hosts::path_for(&state.data_dir);
    let handle = crate::ssh::client::run_blocking_ssh(move || async move {
        SftpHandle::connect(host, port, cred, known_hosts_path, timeout_secs).await
    })
    .await?;
    let id = uuid::Uuid::new_v4().to_string();

    locked(&state.sftp_sessions)?.insert(id.clone(), Arc::new(handle));

    Ok(id)
}

/// Connect SFTP by reusing an active SSH session (no re-authentication).
#[tauri::command]
pub async fn sftp_connect_session(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<String> {
    let ssh_handle = {
        let sessions = locked(&state.sessions)?;
        sessions
            .get(&session_id)
            .ok_or_else(|| AppError::NotFound("SSH 会话不存在".into()))?
            .ssh_handle()
            .clone()
    };

    let parent = session_id.clone();
    let handle = crate::ssh::client::run_blocking_ssh(move || async move {
        SftpHandle::from_handle(&ssh_handle, parent).await
    })
    .await?;
    let id = uuid::Uuid::new_v4().to_string();

    locked(&state.sftp_sessions)?.insert(id.clone(), Arc::new(handle));

    Ok(id)
}

/// 从 Mutex 中 clone 出 Arc<SftpHandle>，释放锁后再 await。
fn get_sftp(state: &State<'_, AppState>, sftp_id: &str) -> AppResult<Arc<SftpHandle>> {
    locked(&state.sftp_sessions)?
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
    locked(&state.sftp_sessions)?.remove(&sftp_id);
    Ok(())
}

/// Download a remote file via native Save As dialog with streaming + progress.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_save_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    sftp_id: String,
    remote_path: String,
    default_name: String,
) -> AppResult<Option<String>> {
    let save_path = rfd::AsyncFileDialog::new()
        .set_file_name(&default_name)
        .save_file()
        .await;

    let Some(handle) = save_path else {
        return Ok(None);
    };
    let local = handle.path().to_path_buf();

    let sftp = get_sftp(&state, &sftp_id)?;
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let cancel = register_cancel_flag(&state, &transfer_id)?;
    let result = sftp
        .download_streaming(&remote_path, &local, &app, &transfer_id, cancel)
        .await;
    unregister_cancel_flag(&state, &transfer_id);
    result?;
    Ok(Some(local.display().to_string()))
}

/// Pick a local file via native Open dialog and upload with streaming + progress.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_pick_and_upload(
    app: tauri::AppHandle,
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

    let sftp = get_sftp(&state, &sftp_id)?;
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let cancel = register_cancel_flag(&state, &transfer_id)?;
    let result = sftp
        .upload_streaming(&local, &remote_path, &app, &transfer_id, cancel)
        .await;
    unregister_cancel_flag(&state, &transfer_id);
    result?;
    Ok(Some(name))
}

/// Open native Save-As dialog and return the chosen path. No transfer happens here.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_pick_save_path(default_name: String) -> AppResult<Option<String>> {
    let handle = rfd::AsyncFileDialog::new()
        .set_file_name(&default_name)
        .save_file()
        .await;
    Ok(handle.map(|h| h.path().display().to_string()))
}

/// Open native Open dialog and return the chosen path. No transfer happens here.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_pick_open_path() -> AppResult<Option<String>> {
    let handle = rfd::AsyncFileDialog::new().pick_file().await;
    Ok(handle.map(|h| h.path().display().to_string()))
}

/// Stream-download to a caller-supplied local path. transfer_id is used as the
/// `sftp:progress` event id so the frontend can multiplex concurrent transfers.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_download_to(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    sftp_id: String,
    remote_path: String,
    local_path: String,
    transfer_id: String,
) -> AppResult<()> {
    let sftp = get_sftp(&state, &sftp_id)?;
    let local = std::path::PathBuf::from(&local_path);
    let cancel = register_cancel_flag(&state, &transfer_id)?;
    let result = sftp
        .download_streaming(&remote_path, &local, &app, &transfer_id, cancel)
        .await;
    unregister_cancel_flag(&state, &transfer_id);
    result.map(|_| ())
}

/// Stream-upload from a caller-supplied local path. transfer_id mirrors above.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_upload_from(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    sftp_id: String,
    local_path: String,
    remote_path: String,
    transfer_id: String,
) -> AppResult<()> {
    let sftp = get_sftp(&state, &sftp_id)?;
    let local = std::path::PathBuf::from(&local_path);
    let cancel = register_cancel_flag(&state, &transfer_id)?;
    let result = sftp
        .upload_streaming(&local, &remote_path, &app, &transfer_id, cancel)
        .await;
    unregister_cancel_flag(&state, &transfer_id);
    result.map(|_| ())
}

/// 用户在传输页点"取消"调用：把 transfer_id 对应的 cancel flag 置 1，
/// streaming 循环下一次 chunk 检查时退出。
#[tauri::command]
pub fn sftp_cancel_transfer(state: State<'_, AppState>, transfer_id: String) -> AppResult<()> {
    use std::sync::atomic::Ordering;
    if let Some(flag) = locked(&state.transfer_cancels)?.get(&transfer_id) {
        flag.store(true, Ordering::SeqCst);
    }
    Ok(())
}
