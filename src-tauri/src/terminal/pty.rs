use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tauri::Emitter;

use crate::error::{locked, AppError, AppResult};

/// 本地 PTY 会话句柄，Clone + Send + Sync。
#[derive(Clone)]
pub struct PtyHandle {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
}

impl PtyHandle {
    pub fn write(&self, data: &[u8]) -> AppResult<()> {
        locked(&self.writer)?
            .write_all(data)
            .map_err(|e| AppError::Pty(e.to_string()))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> AppResult<()> {
        locked(&self.master)?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| AppError::Pty(e.to_string()))
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
        .map_err(|e| AppError::Pty(e.to_string()))?;

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
    let _child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| AppError::Pty(e.to_string()))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::Pty(e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::Pty(e.to_string()))?;

    let id = uuid::Uuid::new_v4().to_string();
    let handle = PtyHandle {
        writer: Arc::new(Mutex::new(writer)),
        master: Arc::new(Mutex::new(pair.master)),
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
