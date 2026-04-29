use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::ai::session::DiagnoseSession;
use crate::db::Db;
use crate::secret::SecretStore;
use crate::ssh::client::SessionHandle;
use crate::ssh::forward::ForwardHandle;
use crate::ssh::sftp::SftpHandle;
#[cfg(not(target_os = "android"))]
use crate::terminal::pty::PtyHandle;

pub struct AppState {
    pub db: Arc<Db>,
    pub secret_store: Arc<dyn SecretStore>,
    pub sessions: Mutex<HashMap<String, SessionHandle>>,
    #[cfg(not(target_os = "android"))]
    pub pty_sessions: Mutex<HashMap<String, PtyHandle>>,
    pub sftp_sessions: Mutex<HashMap<String, Arc<SftpHandle>>>,
    pub active_forwards: Mutex<HashMap<String, ForwardHandle>>,
    pub auth_waiters: Mutex<HashMap<String, tokio::sync::oneshot::Sender<Vec<String>>>>,
    /// 私钥 passphrase 提示等待中：tab_id → oneshot sender。
    /// 与 auth_waiters 分离，因为提示来源（本地 decode）和回应负载（单条字符串）都不同。
    pub passphrase_waiters: Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>,
    /// 进程内 passphrase 缓存：cache_key（credential_id 或文件路径）→ passphrase。
    /// 进程退出即丢，绝不落盘。
    pub passphrase_cache: Mutex<HashMap<String, String>>,
    /// window_label → session IDs owned by that window (for per-window cleanup)
    pub window_sessions: Mutex<HashMap<String, HashSet<String>>>,
    /// AI 排障会话表（ai_session_id → DiagnoseSession）
    pub ai_sessions: Mutex<HashMap<String, DiagnoseSession>>,
    pub data_dir: PathBuf,
}
