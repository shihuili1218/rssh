use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
    /// window_label → session IDs owned by that window (for per-window cleanup)
    pub window_sessions: Mutex<HashMap<String, HashSet<String>>>,
    pub data_dir: PathBuf,
}
