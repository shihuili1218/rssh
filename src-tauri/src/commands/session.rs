use serde_json::json;
use tauri::{AppHandle, State};

use crate::error::{locked, AppError, AppResult};
use crate::models::{Credential, Profile};
use crate::secret::cred_secret_key;
use crate::ssh::client;
use crate::state::AppState;

/// 把 SecretStore 里的 secret 灌到 Credential 上。
fn load_secrets(state: &State<'_, AppState>, c: &mut Credential) -> AppResult<()> {
    c.secret = state.secret_store.get(&cred_secret_key(&c.id))?;
    Ok(())
}

/// 经 profile_id 建立 SSH 连接（自动带堡垒机链）。
#[tauri::command]
pub async fn ssh_connect(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    profile_id: String,
    // 前端 tab ID，用于发送连接日志
    log_session_id: Option<String>,
    cols: u32,
    rows: u32,
) -> AppResult<String> {
    let profile = crate::db::profile::get(&state.db, &profile_id)?;
    let mut credential = crate::db::credential::get(&state.db, &profile.credential_id).map_err(|e| match e {
        AppError::NotFound(_) => AppError::not_found("profile_cred_not_found", json!({})),
        other => other,
    })?;
    load_secrets(&state, &mut credential)?;

    // 解析整条堡垒机链 + 给每一跳加载凭证（含 secret）
    let chain_profiles = crate::ssh::bastion::resolve_chain(&state.db, &profile)?;
    let mut chain: Vec<(Profile, Credential)> = Vec::with_capacity(chain_profiles.len());
    for hop in chain_profiles {
        let mut bc = crate::db::credential::get(&state.db, &hop.credential_id).map_err(|e| match e {
            AppError::NotFound(_) => {
                AppError::not_found("bastion_cred_not_found", json!({ "name": hop.name.clone() }))
            }
            other => other,
        })?;
        load_secrets(&state, &mut bc)?;
        chain.push((hop, bc));
    }

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
            crate::emitter::Host::Tauri(app),
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

/// `tab_id`（== log_session_id）选传。已建连后三张 waiters 理应空，传 tab_id
/// 只作防御性 belt-and-suspenders 清理；漏传不致命。
#[tauri::command]
pub async fn ssh_disconnect(
    state: State<'_, AppState>,
    session_id: String,
    tab_id: Option<String>,
) -> AppResult<()> {
    crate::commands::lifecycle::unregister_window_session(&state, &session_id);

    // 0) 防御性清理三张 waiters，避免任何遗留 sender 永挂。
    if let Some(tid) = tab_id.as_deref() {
        let _ = locked(&state.auth_waiters).map(|mut m| m.remove(tid));
        let _ = locked(&state.passphrase_waiters).map(|mut m| m.remove(tid));
        let _ = locked(&state.host_key_waiters).map(|mut m| m.remove(tid));
    }

    // 1) 先把挂在这条 SSH 上的 SFTP children 清掉。Drop Arc 让传输任务下次
    //    访问 channel 时立刻 IO error 退出 —— 不依赖 frontend 的 finally。
    {
        let mut sftp = locked(&state.sftp_sessions)?;
        sftp.retain(|_, h| h.parent_ssh_id() != Some(&session_id));
    }

    // 2) 拿走 SessionHandle 并强切 TCP（不只是 shell channel）。
    let session = locked(&state.sessions)?
        .remove(&session_id)
        .ok_or_else(|| AppError::not_found("session_not_found", json!({})))?;
    session.force_disconnect();
    Ok(())
}

/// 用户在 keyboard-interactive prompt 前关 tab 时调用，drop sender 让
/// authenticate_interactive 立即报错退出。与 ssh_passphrase_cancel /
/// ssh_host_key_cancel 对称。
#[tauri::command]
pub async fn ssh_auth_cancel(state: State<'_, AppState>, tab_id: String) -> AppResult<()> {
    locked(&state.auth_waiters)?.remove(&tab_id);
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
        .ok_or_else(|| AppError::other("no_pending_auth", json!({})))?;
    tx.send(responses)
        .map_err(|_| AppError::other("auth_channel_closed", json!({})))?;
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
        .ok_or_else(|| AppError::other("no_pending_passphrase", json!({})))?;
    tx.send(passphrase)
        .map_err(|_| AppError::other("passphrase_channel_closed", json!({})))?;
    Ok(())
}

/// 用户在终端弹窗里点取消时调用，让 decode_key_with_prompt 立即报错退出。
#[tauri::command]
pub async fn ssh_passphrase_cancel(state: State<'_, AppState>, tab_id: String) -> AppResult<()> {
    // 直接 drop sender 即可触发等待端的 RecvError
    locked(&state.passphrase_waiters)?.remove(&tab_id);
    Ok(())
}

/// 终端中输完 host key 确认（yes / no / 指纹）后调用，把字符串送回 check_server_key。
#[tauri::command]
pub async fn ssh_host_key_respond(
    state: State<'_, AppState>,
    tab_id: String,
    answer: String,
) -> AppResult<()> {
    let tx = locked(&state.host_key_waiters)?
        .remove(&tab_id)
        .ok_or_else(|| AppError::other("no_pending_hostkey", json!({})))?;
    tx.send(answer)
        .map_err(|_| AppError::other("hostkey_channel_closed", json!({})))?;
    Ok(())
}

/// 用户在终端取消（Ctrl-C / 关 tab）host key 确认时调用，让 check_server_key 立即拒绝。
#[tauri::command]
pub async fn ssh_host_key_cancel(state: State<'_, AppState>, tab_id: String) -> AppResult<()> {
    locked(&state.host_key_waiters)?.remove(&tab_id);
    Ok(())
}

fn get_session(state: &State<'_, AppState>, session_id: &str) -> AppResult<client::SessionHandle> {
    locked(&state.sessions)?
        .get(session_id)
        .cloned()
        .ok_or_else(|| AppError::not_found("session_not_found", json!({})))
}
