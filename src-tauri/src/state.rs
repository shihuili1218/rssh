use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::ai::session::DiagnoseSession;
use crate::db::Db;
use crate::secret::SecretStore;
use crate::ssh::client::SessionHandle;
use crate::ssh::forward::ForwardHandle;
use crate::ssh::sftp::SftpHandle;
#[cfg(not(target_os = "android"))]
use crate::terminal::pty::PtyHandle;
#[cfg(not(target_os = "android"))]
use crate::terminal::serial::SerialHandle;
use crate::terminal::telnet::TelnetHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Ssh,
    #[cfg(not(target_os = "android"))]
    Pty,
    #[cfg(not(target_os = "android"))]
    Serial,
    Telnet,
    Sftp,
    Forward,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SessionOwner {
    Window(String),
    Headless(uuid::Uuid),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Pending,
    Ready,
    Closed,
}

#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub nonce: uuid::Uuid,
    pub kind: SessionKind,
    pub owner: SessionOwner,
    pub phase: SessionPhase,
    pub parent: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AiSessionRecord {
    pub nonce: uuid::Uuid,
    pub owner: SessionOwner,
    pub phase: SessionPhase,
}

pub struct OwnedWaiter<T> {
    pub nonce: uuid::Uuid,
    pub owner: SessionOwner,
    pub sender: tokio::sync::oneshot::Sender<T>,
}

pub struct AppState {
    pub db: Arc<Db>,
    pub secret_store: Arc<dyn SecretStore>,
    /// Single source of truth for connection identity, ownership and lifecycle.
    /// Typed handle maps below contain Ready handles only.
    pub lifecycle_sessions: Mutex<HashMap<String, SessionRecord>>,
    pub sessions: Mutex<HashMap<String, SessionHandle>>,
    #[cfg(not(target_os = "android"))]
    pub pty_sessions: Mutex<HashMap<String, PtyHandle>>,
    #[cfg(not(target_os = "android"))]
    pub serial_sessions: Mutex<HashMap<String, SerialHandle>>,
    /// Telnet is plain TCP — available on every platform, no android gate.
    pub telnet_sessions: Mutex<HashMap<String, TelnetHandle>>,
    pub sftp_sessions: Mutex<HashMap<String, Arc<SftpHandle>>>,
    /// 进行中的 SFTP 传输 cancel flag：transfer_id → AtomicBool。
    /// 用户在传输页点"取消"会把对应位置 1，streaming 循环每个 chunk 查一次，
    /// 命中即提前 Err 退出。传输结束（成功 / 失败 / 取消）都从 map 里移除。
    pub transfer_cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub active_forwards: Mutex<HashMap<String, ForwardHandle>>,
    /// Keyboard-interactive prompt：prompt_id → owner/nonce/sender。
    pub auth_waiters: Mutex<HashMap<String, OwnedWaiter<Vec<String>>>>,
    /// 私钥 passphrase 提示等待中：prompt_id → owner/nonce/sender。
    /// 与 auth_waiters 分离，因为提示来源（本地 decode）和回应负载（单条字符串）都不同。
    pub passphrase_waiters: Mutex<HashMap<String, OwnedWaiter<String>>>,
    /// 主机密钥 TOFU 终端确认等待中：prompt_id → owner/nonce/sender。
    /// 用户在 xterm 中输入 yes / no / 指纹，由 ssh_host_key_respond 把字符串送回 check_server_key。
    pub host_key_waiters: Mutex<HashMap<String, OwnedWaiter<String>>>,
    /// 进程内 passphrase 缓存：cache_key（credential_id 或文件路径）→ passphrase。
    /// 进程退出即丢，绝不落盘。值用 `Zeroizing<String>` 包裹，drop 时擦写底层字节，
    /// 减少内存 dump / swap 中残留明文的窗口。
    pub passphrase_cache: Mutex<HashMap<String, zeroize::Zeroizing<String>>>,
    /// Windows bound to move together by directional "open in new window":
    /// dragging one drags the rest. Desktop-only (mobile is single-window).
    #[cfg(desktop)]
    pub window_groups: Mutex<crate::commands::window::WindowGroups>,
    /// AI 排障会话表（ai_session_id → DiagnoseSession）
    pub ai_sessions: Mutex<HashMap<String, DiagnoseSession>>,
    /// AI actors intentionally keep `tab_id` reuse semantics, so ownership is
    /// tracked separately from the transport registry's permanent UUID tombstones.
    pub ai_session_owners: Mutex<HashMap<String, AiSessionRecord>>,
    /// 远端 shell 探测结果缓存：profile_id → ShellKind。
    /// 进程级（不落盘）—— 同一 profile 重连/多次开 AI panel 复用结果。
    /// 命中即用，不重发探针；用户切了远端 DefaultShell 注册表后需要重启 app
    /// 才能让 rssh 重新探测（可接受 —— 这是罕见的运维变更）。
    pub ai_remote_shell_cache: Mutex<HashMap<String, crate::ai::shell::ShellKind>>,
    pub data_dir: PathBuf,
}
