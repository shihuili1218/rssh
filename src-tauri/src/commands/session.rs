use tauri::{AppHandle, State};

use crate::error::{locked, AppError, AppResult};
use crate::models::{Credential, CredentialType, Profile};
use crate::secret::cred_secret_key;
use crate::ssh::client;
use crate::state::AppState;

/// 把 SecretStore 里的 secret 灌到 Credential 上。
fn load_secrets(state: &State<'_, AppState>, c: &mut Credential) -> AppResult<()> {
    if c.id.is_empty() {
        return Ok(()); // 临时直连凭证，secret 已由前端传入
    }
    c.secret = state.secret_store.get(&cred_secret_key(&c.id))?;
    Ok(())
}

/// 通用 SSH 连接 — 支持直连和堡垒机。
/// 前端可传原始参数（host/port/username），也可传 profile_id 从 DB 查。
#[tauri::command]
pub async fn ssh_connect(
    app: AppHandle,
    window: tauri::Window,
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
    let (profile, credential, chain) = if let Some(pid) = profile_id {
        let p = crate::db::profile::get(&state.db, &pid)?;
        let cred_id = p.credential_id.as_deref().unwrap_or("");
        let mut c = crate::db::credential::get(&state.db, cred_id)
            .map_err(|_| AppError::NotFound("Profile 关联的凭证不存在".into()))?;
        load_secrets(&state, &mut c)?;

        // 解析整条堡垒机链 + 给每一跳加载凭证（含 secret）
        let chain_profiles = crate::ssh::bastion::resolve_chain(&state.db, &p)?;
        let mut chain: Vec<(Profile, Credential)> = Vec::with_capacity(chain_profiles.len());
        for hop in chain_profiles {
            let bcid = hop.credential_id.as_deref().unwrap_or("");
            let mut bc = crate::db::credential::get(&state.db, bcid)
                .map_err(|_| AppError::NotFound(format!("堡垒机 '{}' 凭证不存在", hop.name)))?;
            load_secrets(&state, &mut bc)?;
            chain.push((hop, bc));
        }
        (p, c, chain)
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
            group_id: None,
        };
        let c = Credential {
            id: String::new(),
            name: String::new(),
            username: username.ok_or_else(|| AppError::Config("缺少 username".into()))?,
            credential_type: CredentialType::from_str(&auth_type.unwrap_or("password".into())),
            secret,
            save_to_remote: false,
        };
        (p, c, Vec::new())
    };

    // 检查 verbose log + 录制设置 + 连接超时
    let verbose_log = crate::db::settings::get(&state.db, "verbose_log")?
        .map(|v| v == "true")
        .unwrap_or(true);
    let timeout_secs: u64 = crate::db::settings::get(&state.db, "connect_timeout")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(crate::ssh::client::DEFAULT_CONNECT_TIMEOUT);
    let recording_enabled = crate::db::settings::get(&state.db, "recording_enabled")?
        .map(|v| v == "true")
        .unwrap_or(false);
    let recording_path = if recording_enabled {
        let dir_str = crate::db::settings::get(&state.db, "recording_dir")?
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

    // Only pass log_session_id if verbose logging is enabled
    let effective_log_id = if verbose_log { log_session_id } else { None };

    let known_hosts_path = crate::ssh::known_hosts::path_for(&state.data_dir);
    let init_command = profile.init_command.clone();
    let result = client::run_blocking_ssh(move || async move {
        client::connect(
            profile,
            credential,
            chain,
            cols,
            rows,
            app,
            recording_path,
            effective_log_id,
            known_hosts_path,
            timeout_secs,
        )
        .await
    })
    .await?;

    // 执行初始命令（shell 已就绪，直接写入）
    if let Some(ref cmd) = init_command {
        if !cmd.is_empty() {
            result.handle.write(format!("{}\n", cmd).as_bytes())?;
        }
    }

    locked(&state.sessions)?.insert(result.session_id.clone(), result.handle);
    crate::commands::lifecycle::register_window_session(&state, window.label(), &result.session_id);

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
pub async fn ssh_disconnect(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    crate::commands::lifecycle::unregister_window_session(&state, &session_id);

    // 1) 先把挂在这条 SSH 上的 SFTP children 清掉。Drop Arc 让传输任务下次
    //    访问 channel 时立刻 IO error 退出 —— 不依赖 frontend 的 finally。
    {
        let mut sftp = locked(&state.sftp_sessions)?;
        sftp.retain(|_, h| h.parent_ssh_id() != Some(&session_id));
    }

    // 2) 拿走 SessionHandle 并强切 TCP（不只是 shell channel）。
    let session = locked(&state.sessions)?
        .remove(&session_id)
        .ok_or_else(|| AppError::NotFound("会话不存在".into()))?;
    session.force_disconnect();
    Ok(())
}

#[tauri::command]
pub async fn ssh_auth_respond(
    state: State<'_, AppState>,
    tab_id: String,
    responses: Vec<String>,
) -> AppResult<()> {
    let tx = locked(&state.auth_waiters)?
        .remove(&tab_id)
        .ok_or_else(|| AppError::NotFound("无等待中的认证请求".into()))?;
    tx.send(responses)
        .map_err(|_| AppError::Other("认证通道已关闭".into()))?;
    Ok(())
}

/// 终端内输完私钥 passphrase 后调用，把结果回传给等待中的 decode_key_with_prompt。
#[tauri::command]
pub async fn ssh_passphrase_respond(
    state: State<'_, AppState>,
    tab_id: String,
    passphrase: String,
) -> AppResult<()> {
    let tx = locked(&state.passphrase_waiters)?
        .remove(&tab_id)
        .ok_or_else(|| AppError::NotFound("无等待中的 passphrase 请求".into()))?;
    tx.send(passphrase)
        .map_err(|_| AppError::Other("passphrase 通道已关闭".into()))?;
    Ok(())
}

/// 用户在终端弹窗里点取消时调用，让 decode_key_with_prompt 立即报错退出。
#[tauri::command]
pub async fn ssh_passphrase_cancel(state: State<'_, AppState>, tab_id: String) -> AppResult<()> {
    // 直接 drop sender 即可触发等待端的 RecvError
    locked(&state.passphrase_waiters)?.remove(&tab_id);
    Ok(())
}

fn get_session(state: &State<'_, AppState>, session_id: &str) -> AppResult<client::SessionHandle> {
    locked(&state.sessions)?
        .get(session_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound("会话不存在".into()))
}
