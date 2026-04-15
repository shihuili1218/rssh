//! Session 生命周期管理：前端 reconcile + 窗口销毁清理。

use std::collections::HashSet;

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// 前端启动 / 重连后调用：把不在 `active_ids` 列表里的所有 session 全部清掉。
///
/// `active_ids` 是前端当前持有的所有 ID（不区分 ssh / sftp / forward —
/// UUID 不会撞）。返回被清理的总数。
#[tauri::command]
pub fn reconcile_sessions(
    state: State<'_, AppState>,
    active_ids: Vec<String>,
) -> AppResult<usize> {
    let alive: HashSet<String> = active_ids.into_iter().collect();
    let mut closed = 0;

    // SSH sessions
    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| AppError::Other("sessions lock poisoned".into()))?;
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
        let mut sftp = state
            .sftp_sessions
            .lock()
            .map_err(|_| AppError::Other("sftp lock poisoned".into()))?;
        let before = sftp.len();
        sftp.retain(|k, _| alive.contains(k));
        closed += before - sftp.len();
    }

    // Active forwards
    {
        let mut fwds = state
            .active_forwards
            .lock()
            .map_err(|_| AppError::Other("forward lock poisoned".into()))?;
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
        let mut pty = state
            .pty_sessions
            .lock()
            .map_err(|_| AppError::Other("pty lock poisoned".into()))?;
        let before = pty.len();
        pty.retain(|k, _| alive.contains(k));
        closed += before - pty.len();
    }

    Ok(closed)
}

/// 不带 reconcile 的全清理 —— 窗口销毁时调用。
pub fn close_all(state: &AppState) {
    if let Ok(mut sessions) = state.sessions.lock() {
        for (_, h) in sessions.drain() {
            let _ = h.close();
        }
    }
    if let Ok(mut sftp) = state.sftp_sessions.lock() {
        sftp.clear();
    }
    if let Ok(mut fwds) = state.active_forwards.lock() {
        for (_, h) in fwds.drain() {
            h.stop();
        }
    }
    #[cfg(not(target_os = "android"))]
    if let Ok(mut pty) = state.pty_sessions.lock() {
        pty.clear();
    }
}
