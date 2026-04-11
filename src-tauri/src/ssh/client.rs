use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use russh::client;
use russh::ChannelMsg;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::error::{AppError, AppResult};
use crate::models::{Credential, CredentialType, Profile};
use crate::terminal::recorder::Recorder;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// SSH handler — known_hosts TOFU 验证
// ---------------------------------------------------------------------------

pub struct SshHandler {
    host_port: String,
    known_hosts_path: PathBuf,
    key_mismatch: Arc<StdMutex<bool>>,
}

/// 用 key 的 name + 各变体的核心字节生成紧凑指纹
fn key_fingerprint(key: &russh_keys::key::PublicKey) -> String {
    use russh_keys::key::PublicKey;
    match key {
        PublicKey::Ed25519(k) => format!("ed25519:{:x?}", k.as_bytes()),
        _ => format!("{}", key.name()),
    }
}

fn load_known_hosts(path: &PathBuf) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_known_hosts(path: &PathBuf, hosts: &HashMap<String, String>) {
    if let Ok(json) = serde_json::to_string_pretty(hosts) {
        let _ = std::fs::write(path, json);
    }
}

#[async_trait]
impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fp = key_fingerprint(server_public_key);
        let path = self.known_hosts_path.clone();
        let host_port = self.host_port.clone();

        // 文件 I/O 放到 blocking 线程，不阻塞 async runtime
        let result = tokio::task::spawn_blocking(move || {
            let mut hosts = load_known_hosts(&path);
            if let Some(stored) = hosts.get(&host_port) {
                return if *stored == fp { Ok(true) } else { Ok(false) };
            }
            hosts.insert(host_port, fp);
            save_known_hosts(&path, &hosts);
            Ok(true)
        })
        .await
        .unwrap_or(Ok(true));

        match result {
            Ok(true) => Ok(true),
            Ok(false) => {
                *self.key_mismatch.lock().unwrap() = true;
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
}

fn new_handler(host: &str, port: u16, known_hosts_path: PathBuf) -> (SshHandler, Arc<StdMutex<bool>>) {
    let mismatch = Arc::new(StdMutex::new(false));
    let handler = SshHandler {
        host_port: format!("[{}]:{}", host, port),
        known_hosts_path,
        key_mismatch: mismatch.clone(),
    };
    (handler, mismatch)
}

fn map_connect_error(e: russh::Error, host: &str, port: u16, mismatch: &StdMutex<bool>) -> AppError {
    if *mismatch.lock().unwrap() {
        AppError::Ssh(format!(
            "{}:{} 的主机密钥已变更，连接已拒绝。如确认安全，请删除 ~/.rssh/known_hosts 中对应记录后重试。",
            host, port
        ))
    } else {
        AppError::Ssh(format!("连接失败: {e}"))
    }
}

/// 建立 SSH 连接并验证主机密钥（带超时）。
pub async fn ssh_connect(
    config: Arc<client::Config>,
    host: &str,
    port: u16,
    known_hosts_path: PathBuf,
) -> AppResult<client::Handle<SshHandler>> {
    let (handler, mismatch) = new_handler(host, port, known_hosts_path);
    match timeout(CONNECT_TIMEOUT, client::connect(config, (host, port), handler)).await {
        Ok(result) => result.map_err(|e| map_connect_error(e, host, port, &mismatch)),
        Err(_) => Err(AppError::Ssh(format!("{}:{} 连接超时 ({}s)", host, port, CONNECT_TIMEOUT.as_secs()))),
    }
}

/// 在已有 stream 上建立 SSH 连接（用于堡垒机隧道）。
pub async fn ssh_connect_stream<S>(
    config: Arc<client::Config>,
    stream: S,
    host: &str,
    port: u16,
    known_hosts_path: PathBuf,
) -> AppResult<client::Handle<SshHandler>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (handler, mismatch) = new_handler(host, port, known_hosts_path);
    match timeout(CONNECT_TIMEOUT, client::connect_stream(config, stream, handler)).await {
        Ok(result) => result.map_err(|e| map_connect_error(e, host, port, &mismatch)),
        Err(_) => Err(AppError::Ssh(format!("{}:{} SSH 握手超时", host, port))),
    }
}

// ---------------------------------------------------------------------------
// 认证
// ---------------------------------------------------------------------------

pub async fn authenticate(
    handle: &mut client::Handle<SshHandler>,
    credential: &Credential,
) -> AppResult<()> {
    let ok = match credential.credential_type {
        CredentialType::Password => {
            let pw = credential.secret.as_deref().unwrap_or("");
            handle
                .authenticate_password(&credential.username, pw)
                .await
                .map_err(|e| AppError::Ssh(format!("密码认证失败: {e}")))?
        }
        CredentialType::Key => {
            let pem = credential
                .secret
                .as_deref()
                .ok_or_else(|| AppError::Ssh("缺少私钥数据".into()))?;
            let kp = russh_keys::decode_secret_key(pem, None)
                .map_err(|e| AppError::Ssh(format!("私钥解析失败: {e}")))?;
            handle
                .authenticate_publickey(&credential.username, Arc::new(kp))
                .await
                .map_err(|e| AppError::Ssh(format!("密钥认证失败: {e}")))?
        }
        CredentialType::None => handle
            .authenticate_none(&credential.username)
            .await
            .map_err(|e| AppError::Ssh(format!("认证失败: {e}")))?,
        CredentialType::Interactive => {
            return Err(AppError::Ssh("键盘交互认证暂不支持".into()));
        }
    };
    if !ok {
        return Err(AppError::Ssh("认证被拒绝".into()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SessionCmd / SessionHandle
// ---------------------------------------------------------------------------

pub enum SessionCmd {
    Write(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    Close,
}

#[derive(Clone)]
pub struct SessionHandle {
    tx: mpsc::UnboundedSender<SessionCmd>,
}

impl SessionHandle {
    pub fn write(&self, data: &[u8]) -> AppResult<()> {
        self.tx
            .send(SessionCmd::Write(data.to_vec()))
            .map_err(|_| AppError::Ssh("会话已关闭".into()))
    }
    pub fn resize(&self, cols: u32, rows: u32) -> AppResult<()> {
        self.tx
            .send(SessionCmd::Resize { cols, rows })
            .map_err(|_| AppError::Ssh("会话已关闭".into()))
    }
    pub fn close(&self) -> AppResult<()> {
        self.tx
            .send(SessionCmd::Close)
            .map_err(|_| AppError::Ssh("会话已关闭".into()))
    }
}

// ---------------------------------------------------------------------------
// connect — 支持可选堡垒机（ProxyJump）
// ---------------------------------------------------------------------------

pub struct ConnectResult {
    pub session_id: String,
    pub handle: SessionHandle,
}

pub async fn connect(
    profile: &Profile,
    credential: &Credential,
    bastion: Option<(&Profile, &Credential)>,
    cols: u32,
    rows: u32,
    app: tauri::AppHandle,
    recording_path: Option<std::path::PathBuf>,
    log_session_id: Option<&str>,
    known_hosts_path: PathBuf,
) -> AppResult<ConnectResult> {
    let log = |msg: &str| {
        if let Some(sid) = log_session_id {
            let line = format!("\x1b[90m[ssh] {msg}\x1b[0m\r\n");
            let _ = app.emit(&format!("ssh:data:{sid}"), line.into_bytes());
        }
    };

    let config = Arc::new(client::Config::default());

    let mut handle = if let Some((bastion_profile, bastion_cred)) = bastion {
        log(&format!("Connecting to bastion {}:{} ...", bastion_profile.host, bastion_profile.port));
        let mut bastion_handle = ssh_connect(
            config.clone(),
            &bastion_profile.host,
            bastion_profile.port,
            known_hosts_path.clone(),
        ).await?;

        log(&format!("Bastion connected. Authenticating as {} ({}) ...", bastion_cred.username, bastion_cred.credential_type.as_str()));
        authenticate(&mut bastion_handle, bastion_cred).await?;
        log("Bastion authenticated.");

        log(&format!("Opening tunnel to {}:{} ...", profile.host, profile.port));
        let tunnel = bastion_handle
            .channel_open_direct_tcpip(&profile.host, profile.port as u32, "127.0.0.1", 0)
            .await
            .map_err(|e| AppError::Ssh(format!("堡垒机隧道建立失败: {e}")))?;

        log("Tunnel established. SSH handshake with target ...");
        let stream = tunnel.into_stream();
        ssh_connect_stream(config, stream, &profile.host, profile.port, known_hosts_path).await?
    } else {
        log(&format!("TCP connecting to {}:{} ...", profile.host, profile.port));
        let h = ssh_connect(config, &profile.host, profile.port, known_hosts_path).await?;
        log("TCP connected. SSH handshake OK.");
        h
    };

    log(&format!("Authenticating as {} ({}) ...", credential.username, credential.credential_type.as_str()));
    authenticate(&mut handle, credential).await?;
    log("Authenticated.");

    log("Requesting PTY + shell ...");
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| AppError::Ssh(format!("打开 channel 失败: {e}")))?;

    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .map_err(|e| AppError::Ssh(format!("PTY 请求失败: {e}")))?;

    channel
        .request_shell(false)
        .await
        .map_err(|e| AppError::Ssh(format!("Shell 请求失败: {e}")))?;

    log("Shell ready.\r\n");

    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::unbounded_channel();

    let recorder = recording_path.and_then(|p| Recorder::new(p, cols, rows).ok());

    let data_event = format!("ssh:data:{session_id}");
    let close_event = format!("ssh:close:{session_id}");
    tauri::async_runtime::spawn(async move {
        session_task(data_event, close_event, channel, rx, app, recorder).await;
    });

    Ok(ConnectResult {
        session_id,
        handle: SessionHandle { tx },
    })
}

// ---------------------------------------------------------------------------
// session_task
// ---------------------------------------------------------------------------

enum Event {
    Ssh(Option<ChannelMsg>),
    Cmd(Option<SessionCmd>),
}

async fn session_task(
    data_event: String,
    close_event: String,
    mut channel: russh::Channel<client::Msg>,
    mut rx: mpsc::UnboundedReceiver<SessionCmd>,
    app: tauri::AppHandle,
    mut recorder: Option<Recorder>,
) {
    loop {
        let event = tokio::select! {
            msg = channel.wait() => Event::Ssh(msg),
            cmd = rx.recv() => Event::Cmd(cmd),
        };

        match event {
            Event::Ssh(Some(ChannelMsg::Data { data })) => {
                if let Some(ref mut rec) = recorder {
                    let _ = rec.record(&String::from_utf8_lossy(&data));
                }
                let _ = app.emit(&data_event, data.to_vec());
            }
            Event::Ssh(Some(ChannelMsg::ExtendedData { data, .. })) => {
                if let Some(ref mut rec) = recorder {
                    let _ = rec.record(&String::from_utf8_lossy(&data));
                }
                let _ = app.emit(&data_event, data.to_vec());
            }
            Event::Ssh(Some(ChannelMsg::Eof | ChannelMsg::Close)) | Event::Ssh(None) => {
                break;
            }
            Event::Cmd(Some(SessionCmd::Write(data))) => {
                let _ = channel.data(std::io::Cursor::new(data)).await;
            }
            Event::Cmd(Some(SessionCmd::Resize { cols, rows })) => {
                let _ = channel.window_change(cols, rows, 0, 0).await;
            }
            Event::Cmd(Some(SessionCmd::Close)) | Event::Cmd(None) => {
                let _ = channel.close().await;
                break;
            }
            _ => {}
        }
    }
    if let Some(rec) = recorder {
        let _ = rec.finish();
    }
    let _ = app.emit(&close_event, ());
}
