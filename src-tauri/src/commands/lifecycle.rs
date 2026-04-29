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

    // SSH sessions
    {
        let mut sessions = locked(&state.sessions)?;
        let stale: Vec<String> = sessions
            .keys()
            .filter(|k| !alive.contains(*k))
            .cloned()
            .collect();
        for k in stale {
            if let Some(h) = sessions.remove(&k) {
                let _ = h.close();
                closed += 1;
            }
        }
    }

    // SFTP sessions（Drop 自动断）
    {
        let mut sftp = locked(&state.sftp_sessions)?;
        let before = sftp.len();
        sftp.retain(|k, _| alive.contains(k));
        closed += before - sftp.len();
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

    if let Ok(mut sessions) = state.sessions.lock() {
        for id in &ids {
            if let Some(h) = sessions.remove(id) {
                let _ = h.close();
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    if let Ok(mut pty) = state.pty_sessions.lock() {
        for id in &ids {
            pty.remove(id);
        }
    }
    if let Ok(mut sftp) = state.sftp_sessions.lock() {
        for id in &ids {
            sftp.remove(id);
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
