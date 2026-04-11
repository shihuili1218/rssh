use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::models::{Credential, CredentialType, Profile};
use crate::ssh::client;
use crate::state::AppState;

/// 通用 SSH 连接 — 支持直连和堡垒机。
/// 前端可传原始参数（host/port/username），也可传 profile_id 从 DB 查。
#[tauri::command]
pub async fn ssh_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    // 直连参数
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    auth_type: Option<String>,
    secret: Option<String>,
    // 或 profile_id（从 DB 查，自动带堡垒机）
    profile_id: Option<String>,
    // 前端 tab ID，用于发送连接日志
    log_session_id: Option<String>,
    cols: u32,
    rows: u32,
) -> AppResult<String> {
    let (profile, credential, bastion) = if let Some(pid) = profile_id {
        // 从 DB 查 profile + credential + bastion
        let conn = state.db.lock().map_err(|_| AppError::Other("lock".into()))?;
        let p = crate::db::profile::get(&conn, &pid)?;
        let cred_id = p.credential_id.as_deref().unwrap_or("");
        let c = crate::db::credential::get(&conn, cred_id)
            .map_err(|_| AppError::NotFound("Profile 关联的凭证不存在".into()))?;

        // 查堡垒机
        let bastion = if let Some(ref bid) = p.bastion_profile_id {
            let bp = crate::db::profile::get(&conn, bid)
                .map_err(|_| AppError::NotFound("堡垒机 Profile 不存在".into()))?;
            let bcid = bp.credential_id.as_deref().unwrap_or("");
            let bc = crate::db::credential::get(&conn, bcid)
                .map_err(|_| AppError::NotFound("堡垒机凭证不存在".into()))?;
            Some((bp, bc))
        } else {
            None
        };
        (p, c, bastion)
    } else {
        // 原始参数直连（无堡垒机）
        let p = Profile {
            id: String::new(),
            name: String::new(),
            host: host.ok_or_else(|| AppError::Config("缺少 host".into()))?,
            port: port.unwrap_or(22),
            credential_id: None,
            bastion_profile_id: None,
            init_command: None,
        };
        let c = Credential {
            id: String::new(),
            name: String::new(),
            username: username.ok_or_else(|| AppError::Config("缺少 username".into()))?,
            credential_type: CredentialType::from_str(&auth_type.unwrap_or("password".into())),
            secret,
            save_to_remote: false,
        };
        (p, c, None)
    };

    // 检查 verbose log + 录制设置
    let (verbose_log, recording_path) = {
        let conn = state.db.lock().map_err(|_| AppError::Other("lock".into()))?;
        let verbose = crate::db::settings::get(&conn, "verbose_log")?
            .map(|v| v == "true")
            .unwrap_or(true);
        let enabled = crate::db::settings::get(&conn, "recording_enabled")?
            .map(|v| v == "true")
            .unwrap_or(false);
        let rec = if enabled {
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
            std::fs::create_dir_all(&dir).ok();
            let name = format!(
                "{}_{}.cast",
                profile.name.replace(' ', "_"),
                chrono::Local::now().format("%Y%m%d_%H%M%S")
            );
            Some(dir.join(name))
        } else {
            None
        };
        (verbose, rec)
    };

    // Only pass log_session_id if verbose logging is enabled
    let effective_log_id = if verbose_log { log_session_id.as_deref() } else { None };

    let known_hosts_path = state.data_dir.join("known_hosts");
    let bastion_refs = bastion.as_ref().map(|(bp, bc)| (bp, bc));
    let result = client::connect(&profile, &credential, bastion_refs, cols, rows, app, recording_path, effective_log_id, known_hosts_path).await?;

    // 执行初始命令（shell 已就绪，直接写入）
    if let Some(ref cmd) = profile.init_command {
        if !cmd.is_empty() {
            result.handle.write(format!("{}\n", cmd).as_bytes())?;
        }
    }

    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| AppError::Other("lock".into()))?;
        sessions.insert(result.session_id.clone(), result.handle);
    }

    Ok(result.session_id)
}

#[tauri::command]
pub async fn ssh_write(
    state: State<'_, AppState>,
    session_id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    get_session(&state, &session_id)?.write(&data)
}

#[tauri::command]
pub async fn ssh_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> AppResult<()> {
    get_session(&state, &session_id)?.resize(cols, rows)
}

#[tauri::command]
pub async fn ssh_disconnect(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<()> {
    let session = {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| AppError::Other("lock".into()))?;
        sessions
            .remove(&session_id)
            .ok_or_else(|| AppError::NotFound("会话不存在".into()))?
    };
    session.close()
}

fn get_session(
    state: &State<'_, AppState>,
    session_id: &str,
) -> AppResult<client::SessionHandle> {
    state
        .sessions
        .lock()
        .map_err(|_| AppError::Other("lock".into()))?
        .get(session_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound("会话不存在".into()))
}
