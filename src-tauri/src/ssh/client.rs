use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use russh::client;
use russh::ChannelMsg;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::error::{AppError, AppResult};
use crate::models::{Credential, CredentialType, Profile};
use crate::terminal::recorder::Recorder;

pub const DEFAULT_CONNECT_TIMEOUT: u64 = 10;

/// 默认 SSH 客户端配置：开启 keepalive，远端死了 90 秒内能断开。
pub fn default_client_config() -> Arc<client::Config> {
    let mut cfg = client::Config::default();
    cfg.keepalive_interval = Some(Duration::from_secs(30));
    cfg.keepalive_max = 3;
    Arc::new(cfg)
}

/// Shared SSH connection handle for opening new channels (SFTP, forwarding).
pub type SshHandle = Arc<tokio::sync::Mutex<client::Handle<SshHandler>>>;

// ---------------------------------------------------------------------------
// SSH handler — known_hosts 验证（OpenSSH 标准格式）
// ---------------------------------------------------------------------------

pub struct SshHandler {
    host: String,
    port: u16,
    known_hosts_path: PathBuf,
    key_mismatch: Arc<StdMutex<bool>>,
    /// Sender for forwarded channels from remote port forwarding.
    forwarded_channels: Arc<StdMutex<Option<mpsc::UnboundedSender<russh::Channel<client::Msg>>>>>,
}

#[async_trait]
impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        if let Ok(guard) = self.forwarded_channels.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(channel);
            }
        }
        Ok(())
    }

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        use russh_keys::known_hosts;

        match known_hosts::check_known_hosts_path(
            &self.host,
            self.port,
            server_public_key,
            &self.known_hosts_path,
        ) {
            // 已知且匹配
            Ok(true) => Ok(true),
            // 完全未知 — TOFU：写入 known_hosts，接受
            Ok(false) => {
                let _ = known_hosts::learn_known_hosts_path(
                    &self.host,
                    self.port,
                    server_public_key,
                    &self.known_hosts_path,
                );
                Ok(true)
            }
            // 已知但密钥变了 — 拒绝
            Err(_) => {
                if let Ok(mut m) = self.key_mismatch.lock() {
                    *m = true;
                }
                Ok(false)
            }
        }
    }
}

/// Shared forwarded-channel sender, settable from outside.
pub type ForwardedChannelSender = Arc<StdMutex<Option<mpsc::UnboundedSender<russh::Channel<client::Msg>>>>>;

fn new_handler(host: &str, port: u16, known_hosts_path: PathBuf) -> (SshHandler, Arc<StdMutex<bool>>, ForwardedChannelSender) {
    let mismatch = Arc::new(StdMutex::new(false));
    let fwd_channels: ForwardedChannelSender = Arc::new(StdMutex::new(None));
    let handler = SshHandler {
        host: host.to_string(),
        port,
        known_hosts_path,
        key_mismatch: mismatch.clone(),
        forwarded_channels: fwd_channels.clone(),
    };
    (handler, mismatch, fwd_channels)
}

fn map_connect_error(e: russh::Error, host: &str, port: u16, mismatch: &StdMutex<bool>) -> AppError {
    if *mismatch.lock().unwrap() {
        AppError::Ssh(format!(
            "{}:{} 的主机密钥已变更，连接已拒绝。如确认安全，请删除 ~/.ssh/known_hosts 中对应记录后重试（可用 ssh-keygen -R {} 删除）。",
            host, port, host
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
    timeout_secs: u64,
) -> AppResult<client::Handle<SshHandler>> {
    let connect_timeout = Duration::from_secs(timeout_secs);
    let (handler, mismatch, _fwd) = new_handler(host, port, known_hosts_path);
    match timeout(connect_timeout, client::connect(config, (host, port), handler)).await {
        Ok(result) => result.map_err(|e| map_connect_error(e, host, port, &mismatch)),
        Err(_) => Err(AppError::Ssh(format!("{}:{} 连接超时 ({}s)", host, port, timeout_secs))),
    }
}

/// SSH connect that also returns the forwarded channel sender (for remote forwarding).
pub async fn ssh_connect_with_forward(
    config: Arc<client::Config>,
    host: &str,
    port: u16,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
) -> AppResult<(client::Handle<SshHandler>, ForwardedChannelSender)> {
    let connect_timeout = Duration::from_secs(timeout_secs);
    let (handler, mismatch, fwd) = new_handler(host, port, known_hosts_path);
    let handle = match timeout(connect_timeout, client::connect(config, (host, port), handler)).await {
        Ok(result) => result.map_err(|e| map_connect_error(e, host, port, &mismatch))?,
        Err(_) => return Err(AppError::Ssh(format!("{}:{} 连接超时 ({}s)", host, port, timeout_secs))),
    };
    Ok((handle, fwd))
}

/// 在已有 stream 上建立 SSH 连接（用于堡垒机隧道）。
pub async fn ssh_connect_stream<S>(
    config: Arc<client::Config>,
    stream: S,
    host: &str,
    port: u16,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
) -> AppResult<client::Handle<SshHandler>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let connect_timeout = Duration::from_secs(timeout_secs);
    let (handler, mismatch, _fwd) = new_handler(host, port, known_hosts_path);
    match timeout(connect_timeout, client::connect_stream(config, stream, handler)).await {
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
            let kp = russh_keys::decode_secret_key(pem, credential.passphrase.as_deref())
                .map_err(|e| AppError::Ssh(format!("私钥解析失败: {e}")))?;
            handle
                .authenticate_publickey(&credential.username, Arc::new(kp))
                .await
                .map_err(|e| AppError::Ssh(format!("密钥认证失败: {e}")))?
        }
        CredentialType::Agent => {
            return authenticate_with_agent(handle, &credential.username).await;
        }
        CredentialType::None => handle
            .authenticate_none(&credential.username)
            .await
            .map_err(|e| AppError::Ssh(format!("认证失败: {e}")))?,
        CredentialType::Interactive => {
            // Handled separately by authenticate_interactive()
            return Ok(());
        }
    };
    if !ok {
        return Err(AppError::Ssh("认证被拒绝".into()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SSH Agent 认证
// ---------------------------------------------------------------------------

/// 用系统 SSH agent（$SSH_AUTH_SOCK / Pageant）尝试逐个 identity 认证。
pub async fn authenticate_with_agent(
    handle: &mut client::Handle<SshHandler>,
    username: &str,
) -> AppResult<()> {
    // 把内部具体到 platform 的逻辑放在独立 fn 里，外层走 Send 友好的具体 stream 类型
    #[cfg(unix)]
    {
        let agent = russh_keys::agent::client::AgentClient::connect_env()
            .await
            .map_err(|e| AppError::Ssh(format!("无法连接 SSH agent (检查 $SSH_AUTH_SOCK): {e}")))?;
        try_agent_identities(handle, username, agent).await
    }
    #[cfg(windows)]
    {
        let pipe = r"\\.\pipe\openssh-ssh-agent";
        match russh_keys::agent::client::AgentClient::connect_named_pipe(pipe).await {
            Ok(agent) => try_agent_identities(handle, username, agent).await,
            Err(_) => {
                let agent = russh_keys::agent::client::AgentClient::connect_pageant().await;
                try_agent_identities(handle, username, agent).await
            }
        }
    }
}

async fn try_agent_identities<S>(
    handle: &mut client::Handle<SshHandler>,
    username: &str,
    mut agent: russh_keys::agent::client::AgentClient<S>,
) -> AppResult<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| AppError::Ssh(format!("agent 请求 identity 列表失败: {e}")))?;

    if identities.is_empty() {
        return Err(AppError::Ssh(
            "SSH agent 中没有 identity（先用 `ssh-add` 加 key）".into(),
        ));
    }

    for pk in identities {
        let (returned, result) = handle
            .authenticate_future(username.to_string(), pk, agent)
            .await;
        agent = returned;
        match result {
            Ok(true) => return Ok(()),
            Ok(false) => continue,
            Err(e) => log::warn!("agent identity 签名失败: {e}"),
        }
    }
    Err(AppError::Ssh("SSH agent 中所有 identity 都被服务器拒绝".into()))
}

// ---------------------------------------------------------------------------
// 键盘交互认证
// ---------------------------------------------------------------------------

pub async fn authenticate_interactive(
    handle: &mut client::Handle<SshHandler>,
    username: &str,
    app: &tauri::AppHandle,
    tab_id: &str,
) -> AppResult<()> {
    use russh::client::KeyboardInteractiveAuthResponse;

    let mut reply = handle
        .authenticate_keyboard_interactive_start(username, None::<String>)
        .await
        .map_err(|e| AppError::Ssh(format!("键盘交互启动失败: {e}")))?;

    loop {
        match reply {
            KeyboardInteractiveAuthResponse::Success => return Ok(()),
            KeyboardInteractiveAuthResponse::Failure => {
                return Err(AppError::Ssh("认证被拒绝".into()));
            }
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let (tx, rx) = tokio::sync::oneshot::channel::<Vec<String>>();

                let prompt_data: Vec<serde_json::Value> = prompts
                    .iter()
                    .map(|p| serde_json::json!({ "prompt": p.prompt, "echo": p.echo }))
                    .collect();
                let _ = app.emit(
                    &format!("ssh:auth_prompt:{tab_id}"),
                    serde_json::json!({
                        "name": name,
                        "instructions": instructions,
                        "prompts": prompt_data,
                    }),
                );

                {
                    let state = app.state::<crate::state::AppState>();
                    let mut waiters = state
                        .auth_waiters
                        .lock()
                        .map_err(|_| AppError::Other("lock".into()))?;
                    waiters.insert(tab_id.to_string(), tx);
                }

                let responses = rx
                    .await
                    .map_err(|_| AppError::Ssh("用户取消了认证".into()))?;

                reply = handle
                    .authenticate_keyboard_interactive_respond(responses)
                    .await
                    .map_err(|e| AppError::Ssh(format!("认证响应失败: {e}")))?;
            }
        }
    }
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
    ssh_handle: SshHandle,
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
    pub fn ssh_handle(&self) -> &SshHandle {
        &self.ssh_handle
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
    timeout_secs: u64,
) -> AppResult<ConnectResult> {
    let log = |msg: &str| {
        if let Some(sid) = log_session_id {
            let line = format!("\x1b[90m[ssh] {msg}\x1b[0m\r\n");
            let _ = app.emit(&format!("ssh:data:{sid}"), line.into_bytes());
        }
    };

    let config = default_client_config();

    let mut handle = if let Some((bastion_profile, bastion_cred)) = bastion {
        log(&format!("Connecting to bastion {}:{} ...", bastion_profile.host, bastion_profile.port));
        let mut bastion_handle = ssh_connect(
            config.clone(),
            &bastion_profile.host,
            bastion_profile.port,
            known_hosts_path.clone(),
            timeout_secs,
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
        ssh_connect_stream(config, stream, &profile.host, profile.port, known_hosts_path, timeout_secs).await?
    } else {
        log(&format!("TCP connecting to {}:{} ...", profile.host, profile.port));
        let h = ssh_connect(config, &profile.host, profile.port, known_hosts_path, timeout_secs).await?;
        log("TCP connected. SSH handshake OK.");
        h
    };

    log(&format!("Authenticating as {} ({}) ...", credential.username, credential.credential_type.as_str()));
    if credential.credential_type == crate::models::CredentialType::Interactive {
        let tab_id = log_session_id.unwrap_or("unknown");
        authenticate_interactive(&mut handle, &credential.username, &app, tab_id).await?;
    } else {
        authenticate(&mut handle, credential).await?;
    }
    log("Authenticated.");

    // Wrap handle for channel multiplexing (SFTP, forwarding on same connection)
    let ssh_handle: SshHandle = Arc::new(tokio::sync::Mutex::new(handle));

    log("Requesting PTY + shell ...");
    let channel = {
        let h = ssh_handle.lock().await;
        h.channel_open_session()
            .await
            .map_err(|e| AppError::Ssh(format!("打开 channel 失败: {e}")))?
    };

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
        handle: SessionHandle { tx, ssh_handle },
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
