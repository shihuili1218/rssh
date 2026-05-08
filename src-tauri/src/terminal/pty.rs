use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tauri::Emitter;

use crate::error::{locked, AppError, AppResult};

/// 子进程持有者：保证 PtyHandle 最后一份 clone 被 drop 时（tab 关闭 / session 结束），
/// 显式 kill + wait 子 shell。否则 Box<dyn Child> 在 spawn() 返回后立刻 drop，
/// 子进程退出后无人 reap，留 zombie 占 PID。
/// `Box<dyn Child + Send>` 不带 `Sync`：portable_pty 的 Child 实现普遍只是
/// Send。`Mutex<T>` 自身在 `T: Send` 时即是 Sync，无需 inner 也 Sync——
/// 加多余的 Sync bound 在某些平台上会编不过。
struct ChildReaper {
    child: Mutex<Option<Box<dyn Child + Send>>>,
}

impl Drop for ChildReaper {
    fn drop(&mut self) {
        // Drop 在 Arc 计数归零时跑一次。kill + wait 通常 < 100ms（SIGKILL → 内核 reap）。
        if let Ok(mut g) = self.child.lock() {
            if let Some(mut c) = g.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

/// 本地 PTY 会话句柄，Clone + Send + Sync。
/// `_reaper` 跟着 PtyHandle 走，最后一份 clone 消失时回收子进程。
#[derive(Clone)]
pub struct PtyHandle {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    _reaper: Arc<ChildReaper>,
}

impl PtyHandle {
    pub fn write(&self, data: &[u8]) -> AppResult<()> {
        locked(&self.writer)?
            .write_all(data)
            .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> AppResult<()> {
        locked(&self.master)?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))
    }
}

/// 启动本地 shell，返回 (session_id, handle)。
/// 读取线程通过 Tauri 事件 `pty:data:{id}` 推送数据。
/// Platform default shells.
pub fn available_shells() -> Vec<&'static str> {
    if cfg!(target_os = "windows") {
        vec!["powershell.exe", "cmd.exe", "wsl.exe", "bash.exe"]
    } else if cfg!(target_os = "macos") {
        vec!["/bin/zsh", "/bin/bash", "/bin/sh"]
    } else {
        vec!["/bin/bash", "/bin/zsh", "/bin/sh", "/usr/bin/fish"]
    }
}

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| available_shells()[0].to_string())
}

pub fn spawn(
    cols: u16,
    rows: u16,
    app: tauri::AppHandle,
    shell_override: Option<String>,
) -> AppResult<(String, PtyHandle)> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))?;

    let shell = shell_override
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_shell);

    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("RSSH_APP", "1");
    if !cfg!(target_os = "windows") {
        cmd.arg("-l");
    }
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::pty("pty_op_failed", serde_json::json!({ "err": e.to_string() })))?;

    let id = uuid::Uuid::new_v4().to_string();
    let handle = PtyHandle {
        writer: Arc::new(Mutex::new(writer)),
        master: Arc::new(Mutex::new(pair.master)),
        _reaper: Arc::new(ChildReaper {
            child: Mutex::new(Some(child)),
        }),
    };

    // 读取线程：PTY stdout → Tauri 事件
    let pty_id = id.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut reader = reader;
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = app.emit(&format!("pty:data:{pty_id}"), buf[..n].to_vec());
                }
            }
        }
        let _ = app.emit(&format!("pty:close:{pty_id}"), ());
    });

    Ok((id, handle))
}
