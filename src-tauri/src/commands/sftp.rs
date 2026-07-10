use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde_json::json;
use tauri::State;

use crate::error::{locked, AppError, AppResult};
use crate::models::{Credential, CredentialType};
use crate::ssh::sftp::{FileStat, RemoteEntry, SftpHandle, WalkEntry};
use crate::state::AppState;
use crate::state::{SessionKind, SessionOwner};

#[cfg(not(target_os = "android"))]
use tauri_plugin_dialog::{DialogExt, FilePath};

/// Maximum recursion depth for the local walker. Mirrors the remote-side cap.
const LOCAL_WALK_DEPTH_CAP: u32 = 32;

/// RAII：注册 cancel flag 并在 drop 时自动 unregister，无论 streaming 正常返回、
/// 早 `?`、还是 panic。替代旧的手写 register/unregister 配对。
pub struct CancelGuard<'a> {
    state: &'a AppState,
    transfer_id: String,
}

impl<'a> CancelGuard<'a> {
    /// 注册 flag。返回 (guard, flag)：guard 控生命周期，flag 喂给 streaming 函数。
    /// `pub` 让 headless server 复用同一套 RAII 清理（drop 时 unregister，覆盖
    /// 正常返回 / 早 `?` / panic 三种路径），避免手写 register/remove 漏删。
    pub fn register(
        state: &'a AppState,
        transfer_id: String,
    ) -> AppResult<(Self, Arc<AtomicBool>)> {
        let flag = Arc::new(AtomicBool::new(false));
        locked(&state.transfer_cancels)?.insert(transfer_id.clone(), flag.clone());
        Ok((Self { state, transfer_id }, flag))
    }
}

impl Drop for CancelGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut m) = locked(&self.state.transfer_cancels) {
            m.remove(&self.transfer_id);
        }
    }
}

#[tauri::command]
pub async fn sftp_connect(
    window: tauri::Window,
    state: State<'_, AppState>,
    host: String,
    port: u16,
    username: String,
    auth_type: String,
    secret: Option<String>,
) -> AppResult<String> {
    let reservation = crate::commands::lifecycle::reserve_generated_resource(
        &state,
        SessionKind::Sftp,
        SessionOwner::Window(window.label().to_owned()),
    )?;
    let id = reservation.id().to_owned();
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
    reservation.activate(crate::commands::lifecycle::ReadySession::Sftp(Arc::new(
        handle,
    )))?;

    Ok(id)
}

/// Connect SFTP by reusing an active SSH session (no re-authentication).
#[tauri::command]
pub async fn sftp_connect_session(
    window: tauri::Window,
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<String> {
    let (reservation, ssh_handle) = crate::commands::lifecycle::reserve_sftp_child(
        &state,
        &session_id,
        &SessionOwner::Window(window.label().to_owned()),
    )?;
    let id = reservation.id().to_owned();

    let parent = session_id.clone();
    let handle = crate::ssh::client::run_blocking_ssh(move || async move {
        SftpHandle::from_handle(&ssh_handle, parent).await
    })
    .await?;
    reservation.activate(crate::commands::lifecycle::ReadySession::Sftp(Arc::new(
        handle,
    )))?;

    Ok(id)
}

/// 从 Mutex 中 clone 出 Arc<SftpHandle>，释放锁后再 await。
fn get_sftp(state: &State<'_, AppState>, sftp_id: &str) -> AppResult<Arc<SftpHandle>> {
    locked(&state.sftp_sessions)?
        .get(sftp_id)
        .cloned()
        .ok_or_else(|| AppError::not_found("sftp_session_not_found", json!({})))
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

/// Recursively list every file under a remote directory (symlink-to-file is
/// followed, symlink-to-dir is skipped to prevent cycles). The frontend queues
/// each returned entry as an independent Transfer; the directory abstraction
/// exists only inside this command.
#[tauri::command]
pub async fn sftp_walk_remote_dir(
    state: State<'_, AppState>,
    sftp_id: String,
    remote_root: String,
) -> AppResult<Vec<WalkEntry>> {
    let h = get_sftp(&state, &sftp_id)?;
    h.walk_files(&remote_root).await
}

/// Recursively list every file under a local directory; the local-side
/// counterpart of `sftp_walk_remote_dir`. `rel_path` always uses '/'; the
/// frontend swaps the separator when rebuilding the local physical path.
#[tauri::command]
pub async fn walk_local_dir(local_root: String) -> AppResult<Vec<WalkEntry>> {
    let root = PathBuf::from(&local_root);
    let mut queue: VecDeque<(PathBuf, u32)> = VecDeque::new();
    queue.push_back((root.clone(), 0));
    let mut result: Vec<WalkEntry> = Vec::new();

    while let Some((dir, depth)) = queue.pop_front() {
        if depth >= LOCAL_WALK_DEPTH_CAP {
            return Err(AppError::other(
                "local_tree_too_deep",
                json!({
                    "path": dir.display().to_string(),
                    "depth": depth,
                    "limit": LOCAL_WALK_DEPTH_CAP,
                }),
            ));
        }
        let mut rd = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = rd.next_entry().await? {
            // `entry.metadata()` does not traverse symlinks — single syscall
            // covers both type discrimination and size for regular files,
            // replacing the previous file_type() + metadata() double-stat.
            let path = entry.path();
            let meta = entry.metadata().await?;
            if meta.is_dir() {
                queue.push_back((path, depth + 1));
            } else if meta.is_file() {
                result.push(WalkEntry {
                    rel_path: rel_unix(&path, &root),
                    size: meta.len(),
                });
            } else if meta.is_symlink() {
                // Follow once to learn what the target is. Skip symlink-to-dir
                // to avoid cycles, and silently skip broken symlinks.
                if let Ok(target_meta) = tokio::fs::metadata(&path).await {
                    if target_meta.is_file() {
                        result.push(WalkEntry {
                            rel_path: rel_unix(&path, &root),
                            size: target_meta.len(),
                        });
                    }
                }
            }
            // Anything else (block/char/fifo): skip.
        }
    }
    Ok(result)
}

/// Convert the portion of `full` relative to `root` into a '/'-separated string.
/// On Windows std::path::Component uses '\'; we normalise here and the frontend
/// converts back to the platform separator when joining.
fn rel_unix(full: &Path, root: &Path) -> String {
    let stripped = full.strip_prefix(root).unwrap_or(full);
    stripped
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
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
pub async fn sftp_close(
    window: tauri::Window,
    state: State<'_, AppState>,
    sftp_id: String,
) -> AppResult<()> {
    crate::commands::lifecycle::close_resource(
        &state,
        &sftp_id,
        SessionKind::Sftp,
        &SessionOwner::Window(window.label().to_owned()),
    )
}

/// dialog plugin 的 FilePath → 本地 PathBuf。SFTP 命令全是 `cfg(not(android))`，
/// dialog 在桌面总返回真实路径，移动端的 content URI 不会出现在这里。
#[cfg(not(target_os = "android"))]
fn dialog_to_path(fp: FilePath) -> AppResult<PathBuf> {
    fp.into_path()
        .map_err(|e| AppError::other("file_path_invalid", json!({ "err": e.to_string() })))
}

/// `spawn_blocking` 的 JoinError → AppError。
#[cfg(not(target_os = "android"))]
fn dialog_join_err(e: tokio::task::JoinError) -> AppError {
    AppError::other("dialog_task_failed", json!({ "err": e.to_string() }))
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
    let dialog_app = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        dialog_app
            .dialog()
            .file()
            .set_file_name(&default_name)
            .blocking_save_file()
    })
    .await
    .map_err(dialog_join_err)?;
    let Some(fp) = picked else { return Ok(None) };
    let local = dialog_to_path(fp)?;

    let sftp = get_sftp(&state, &sftp_id)?;
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let (_guard, cancel) = CancelGuard::register(&state, transfer_id.clone())?;
    let host = crate::emitter::Host::Tauri(app);
    sftp.download_streaming(&remote_path, &local, &host, &transfer_id, cancel)
        .await?;
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
    let dialog_app = app.clone();
    let picked =
        tokio::task::spawn_blocking(move || dialog_app.dialog().file().blocking_pick_file())
            .await
            .map_err(dialog_join_err)?;
    let Some(fp) = picked else { return Ok(None) };
    let local = dialog_to_path(fp)?;

    let name = local
        .file_name()
        .ok_or_else(|| AppError::other("sftp_invalid_filename", json!({})))?
        .to_string_lossy()
        .into_owned();
    let remote_path = if remote_dir == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", remote_dir.trim_end_matches('/'), name)
    };

    let sftp = get_sftp(&state, &sftp_id)?;
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let (_guard, cancel) = CancelGuard::register(&state, transfer_id.clone())?;
    let host = crate::emitter::Host::Tauri(app);
    let mut reader = tokio::fs::File::open(&local).await?;
    let total = reader.metadata().await?.len();
    sftp.upload_streaming(
        &mut reader,
        total,
        &remote_path,
        &host,
        &transfer_id,
        cancel,
    )
    .await?;
    Ok(Some(name))
}

/// Open native Save-As dialog and return the chosen path. No transfer happens here.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_pick_save_path(
    app: tauri::AppHandle,
    default_name: String,
) -> AppResult<Option<String>> {
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_file_name(&default_name)
            .blocking_save_file()
    })
    .await
    .map_err(dialog_join_err)?;
    match picked {
        Some(fp) => Ok(Some(dialog_to_path(fp)?.display().to_string())),
        None => Ok(None),
    }
}

/// Open native Open dialog and return the chosen path. No transfer happens here.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_pick_open_path(app: tauri::AppHandle) -> AppResult<Option<String>> {
    let picked = tokio::task::spawn_blocking(move || app.dialog().file().blocking_pick_file())
        .await
        .map_err(dialog_join_err)?;
    match picked {
        Some(fp) => Ok(Some(dialog_to_path(fp)?.display().to_string())),
        None => Ok(None),
    }
}

/// Pick a folder via the native dialog. Used both as the destination root
/// (multi-select download) and the source root (recursive upload) — both
/// flows want the same `blocking_pick_folder()` call, so a single command suffices.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_pick_folder(app: tauri::AppHandle) -> AppResult<Option<String>> {
    let picked = tokio::task::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(dialog_join_err)?;
    match picked {
        Some(fp) => Ok(Some(dialog_to_path(fp)?.display().to_string())),
        None => Ok(None),
    }
}

/// Pick multiple source files for upload. `blocking_pick_files` supports
/// multi-selection on every desktop platform we ship to.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn sftp_pick_open_files(app: tauri::AppHandle) -> AppResult<Option<Vec<String>>> {
    let picked = tokio::task::spawn_blocking(move || app.dialog().file().blocking_pick_files())
        .await
        .map_err(dialog_join_err)?;
    let Some(fps) = picked else { return Ok(None) };
    let paths = fps
        .into_iter()
        .map(|fp| dialog_to_path(fp).map(|p| p.display().to_string()))
        .collect::<AppResult<Vec<_>>>()?;
    Ok(Some(paths))
}

/// `sftp_io_failed` for a local open failure — same code/i18n as other SFTP IO.
fn open_err(e: std::io::Error) -> AppError {
    AppError::sftp(
        "sftp_io_failed",
        json!({ "op": "open", "err": e.to_string() }),
    )
}

/// Resolve a `FilePath` (a desktop path or a mobile SAF `content://` URI) to a
/// real `std::fs::File` for reading, via plugin-fs. Desktop paths open directly;
/// Android URIs resolve to an fd through the ContentResolver bridge.
fn fs_open_read(app: &tauri::AppHandle, fp: tauri_plugin_fs::FilePath) -> AppResult<std::fs::File> {
    use tauri_plugin_fs::{FsExt, OpenOptions};
    let mut opts = OpenOptions::new();
    opts.read(true);
    app.fs().open(fp, opts).map_err(open_err)
}

/// Same as [`fs_open_read`] but opens (create + truncate) for writing — the
/// mobile download target.
fn fs_open_write(
    app: &tauri::AppHandle,
    fp: tauri_plugin_fs::FilePath,
) -> AppResult<std::fs::File> {
    use tauri_plugin_fs::{FsExt, OpenOptions};
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    app.fs().open(fp, opts).map_err(open_err)
}

/// Stream-download to a caller-supplied local target. transfer_id is used as the
/// `sftp:progress:{transfer_id}` event suffix (R1) so the frontend listens
/// per-transfer instead of multiplexing one global stream.
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
    let (_guard, cancel) = CancelGuard::register(&state, transfer_id.clone())?;
    let host = crate::emitter::Host::Tauri(app.clone());
    // Desktop sends a filesystem path → keep the atomic `.part` + rename.
    // Mobile sends a SAF `content://` URI → open it through plugin-fs (which
    // resolves the URI to a real fd) and stream straight in. There's no path to
    // rename through on mobile, so that download has no local atomicity (agreed).
    match local_path
        .parse::<tauri_plugin_fs::FilePath>()
        .expect("FilePath::from_str is infallible")
    {
        tauri_plugin_fs::FilePath::Path(p) => sftp
            .download_streaming(&remote_path, &p, &host, &transfer_id, cancel)
            .await
            .map(|_| ()),
        fp @ tauri_plugin_fs::FilePath::Url(_) => {
            let mut dst = tokio::fs::File::from_std(fs_open_write(&app, fp)?);
            sftp.download_streaming_to_writer(&remote_path, &mut dst, &host, &transfer_id, cancel)
                .await
                .map(|_| ())
        }
    }
}

/// Stream-upload from a caller-supplied local source. transfer_id mirrors above.
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
    let (_guard, cancel) = CancelGuard::register(&state, transfer_id.clone())?;
    let host = crate::emitter::Host::Tauri(app.clone());
    // Local end is just a reader, so desktop (path) and mobile (content:// URI)
    // share one path — plugin-fs resolves either to a real fd.
    let fp = local_path
        .parse::<tauri_plugin_fs::FilePath>()
        .expect("FilePath::from_str is infallible");
    let mut reader = tokio::fs::File::from_std(fs_open_read(&app, fp)?);
    // content:// fds may not support fstat; fall back to 0 (indeterminate bar).
    let total = reader.metadata().await.map(|m| m.len()).unwrap_or(0);
    sftp.upload_streaming(
        &mut reader,
        total,
        &remote_path,
        &host,
        &transfer_id,
        cancel,
    )
    .await
    .map(|_| ())
}

#[tauri::command]
pub async fn sftp_remove(
    state: State<'_, AppState>,
    sftp_id: String,
    path: String,
) -> AppResult<()> {
    let h = get_sftp(&state, &sftp_id)?;
    h.remove(&path).await
}

#[tauri::command]
pub async fn sftp_rename(
    state: State<'_, AppState>,
    sftp_id: String,
    old_path: String,
    new_path: String,
) -> AppResult<()> {
    let h = get_sftp(&state, &sftp_id)?;
    h.rename(&old_path, &new_path).await
}

#[tauri::command]
pub async fn sftp_stat(
    state: State<'_, AppState>,
    sftp_id: String,
    path: String,
) -> AppResult<FileStat> {
    let h = get_sftp(&state, &sftp_id)?;
    h.stat(&path).await
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
