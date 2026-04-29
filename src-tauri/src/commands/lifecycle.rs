//! Session 生命周期管理：前端 reconcile + 窗口销毁清理。

use std::collections::HashSet;

use tauri::State;

use crate::error::{locked, AppResult};
use crate::state::AppState;

/// 前端启动 / 重连后调用：把不在 `active_ids` 列表里的所有 session 全部清掉。
///
/// `active_ids` 是前端当前持有的所有 ID（不区分 ssh / sftp / forward —
/// UUID 不会撞）。返回被清理的总数。
#[tauri::command]
pub fn reconcile_sessions(state: State<'_, AppState>, active_ids: Vec<String>) -> AppResult<usize> {
    let alive: HashSet<String> = active_ids.into_iter().collect();
    let mut closed = 0;

    // SSH sessions —— 收集所有要被关掉的 ssh id，先做 SFTP children 联动清理，
    // 再切断 TCP。
    let mut stale_ssh: Vec<String> = Vec::new();
    {
        let sessions = locked(&state.sessions)?;
        for k in sessions.keys() {
            if !alive.contains(k) {
                stale_ssh.push(k.clone());
            }
        }
    }

    // SFTP sessions：本身不在 alive 里的清掉；父 SSH 也要被清的 children 也清掉。
    {
        let mut sftp = locked(&state.sftp_sessions)?;
        let before = sftp.len();
        sftp.retain(|k, h| {
            alive.contains(k) && match h.parent_ssh_id() {
                Some(parent) => !stale_ssh.iter().any(|s| s == parent),
                None => true,
            }
        });
        closed += before - sftp.len();
    }

    // 现在再切 SSH 的 TCP（避免 children 还在的时候 disconnect 得到无意义的传输报错）
    {
        let mut sessions = locked(&state.sessions)?;
        for k in &stale_ssh {
            if let Some(h) = sessions.remove(k) {
                h.force_disconnect();
                closed += 1;
            }
        }
    }

    // Active forwards
    {
        let mut fwds = locked(&state.active_forwards)?;
        let stale: Vec<String> = fwds
            .keys()
            .filter(|k| !alive.contains(*k))
            .cloned()
            .collect();
        for k in stale {
            if let Some(h) = fwds.remove(&k) {
                h.stop();
                closed += 1;
            }
        }
    }

    // PTY（桌面平台）
    #[cfg(not(target_os = "android"))]
    {
        let mut pty = locked(&state.pty_sessions)?;
        let before = pty.len();
        pty.retain(|k, _| alive.contains(k));
        closed += before - pty.len();
    }

    Ok(closed)
}

/// 注册 session 归属窗口（session 创建时调用）。
pub fn register_window_session(state: &AppState, window_label: &str, session_id: &str) {
    if let Ok(mut ws) = state.window_sessions.lock() {
        ws.entry(window_label.to_string())
            .or_default()
            .insert(session_id.to_string());
    }
}

/// 取消 session 的窗口归属（session 单独关闭时调用）。
pub fn unregister_window_session(state: &AppState, session_id: &str) {
    if let Ok(mut ws) = state.window_sessions.lock() {
        for set in ws.values_mut() {
            set.remove(session_id);
        }
    }
}

/// 关闭指定窗口拥有的所有 session —— 窗口销毁时调用。
pub fn close_window_sessions(state: &AppState, window_label: &str) {
    let ids = match state.window_sessions.lock() {
        Ok(mut ws) => ws.remove(window_label).unwrap_or_default(),
        Err(_) => return,
    };
    if ids.is_empty() {
        return;
    }

    // 先把所有挂在这些 SSH 上的 SFTP children 清掉（基于 parent_ssh_id 反查），
    // 再切 TCP。这样传输的 channel I/O 会被底层 socket 关掉自然 error 退出。
    if let Ok(mut sftp) = state.sftp_sessions.lock() {
        sftp.retain(|sftp_id, h| {
            // ids 里的 SFTP（本身被记录在窗口下的）和 parent_ssh_id 在 ids 里的 children 都清
            !ids.contains(sftp_id)
                && match h.parent_ssh_id() {
                    Some(parent) => !ids.contains(parent),
                    None => true,
                }
        });
    }

    if let Ok(mut sessions) = state.sessions.lock() {
        for id in &ids {
            if let Some(h) = sessions.remove(id) {
                h.force_disconnect();
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    if let Ok(mut pty) = state.pty_sessions.lock() {
        for id in &ids {
            pty.remove(id);
        }
    }
    if let Ok(mut fwds) = state.active_forwards.lock() {
        for id in &ids {
            if let Some(h) = fwds.remove(id) {
                h.stop();
            }
        }
    }
}
