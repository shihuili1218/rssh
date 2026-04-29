use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use russh::client;
use russh::keys::agent::AgentIdentity;
use russh::keys::{Algorithm, HashAlg, PrivateKey, PrivateKeyWithHashAlg};
use russh::ChannelMsg;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::error::{locked, AppError, AppResult};
use crate::models::{Credential, CredentialType, Profile};
use crate::terminal::recorder::Recorder;

pub const DEFAULT_CONNECT_TIMEOUT: u64 = 10;

/// Job sent to the dedicated SSH worker thread. The closure runs on the
/// worker thread inside the LocalSet — it is responsible for spawning its
/// own local future. The closure itself is `Send` (we ship it across the
/// mpsc channel), but the future it produces does NOT need to be `Send`,
/// which is the whole point.
type SshJob = Box<dyn FnOnce() + Send + 'static>;

/// Submit a job to the SSH worker thread. Lazy-spawns the worker on first
/// use: a single OS thread driving a `current_thread` tokio runtime + a
/// `LocalSet`. The worker thread (and runtime) lives for the process
/// lifetime — no drop-the-runtime-and-kill-everything regression.
///
/// **Why this layout**: the only way to dodge the HRTB-Send elaboration
/// failure on russh's internal `&Sender<Msg>` borrows (rust-lang#96865) is
/// to spawn russh futures on a runtime that doesn't require `Send` on its
/// tasks. `LocalSet::spawn_local` is exactly that. But LocalSet pins to one
/// thread — so we dedicate one. `#[tauri::command]` futures only ever
/// await `oneshot::Receiver`, which carries no russh-derived types, so the
/// HRTB-Send check on the command never sees the russh internals.
fn ssh_dispatcher() -> &'static tokio::sync::mpsc::UnboundedSender<SshJob> {
    static TX: OnceLock<tokio::sync::mpsc::UnboundedSender<SshJob>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SshJob>();
        std::thread::Builder::new()
            .name("rssh-ssh".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("ssh worker runtime");
                let local = tokio::task::LocalSet::new();
                rt.block_on(local.run_until(async move {
                    while let Some(job) = rx.recv().await {
                        job();
                    }
                }));
            })
            .expect("spawn ssh worker thread");
        tx
    })
}

/// Spawn an SSH-touching future on the dedicated SSH worker. Returns a
/// `oneshot::Receiver` for the result.
///
/// `work` must be `Send + 'static` (it crosses thread boundaries via mpsc),
/// but the future it returns does NOT need to be `Send` — it runs on the
/// LocalSet, single-threaded.
pub fn spawn_ssh<F, Fut, T>(work: F) -> tokio::sync::oneshot::Receiver<AppResult<T>>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = AppResult<T>> + 'static,
    T: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    let job: SshJob = Box::new(move || {
        let fut = work();
        tokio::task::spawn_local(async move {
            let _ = tx.send(fut.await);
        });
    });
    let _ = ssh_dispatcher().send(job);
    rx
}

/// Convenience: spawn + await + flatten. Call from async ctx.
pub async fn run_blocking_ssh<F, Fut, T>(work: F) -> AppResult<T>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = AppResult<T>> + 'static,
    T: Send + 'static,
{
    spawn_ssh(work)
        .await
        .map_err(|_| AppError::Ssh("SSH 任务取消".into()))?
}

/// Owned, type-erased progress logger that consumes `String`.
///
/// Why `String` and not `&str`: a `Fn(&str)` trait object is `for<'a> Fn(&'a str)`,
/// a higher-ranked bound. When that trait object is captured in a future and
/// awaited under `#[tauri::command]`, the compiler can't elaborate
/// `for<'a>` Send through the russh internal state (rust-lang#96865 cluster).
/// `Fn(String)` carries no HRTB — caller hands over an owned String per call.
/// Cost is one allocation per log line; we log a handful per connection.
pub type LogFn = Arc<dyn Fn(String) + Send + Sync>;

pub(crate) fn null_logger() -> LogFn {
    Arc::new(|_: String| ())
}

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
    /// Surface TOFU fingerprints / known_hosts write errors back to the user.
    log: LogFn,
}

impl client::Handler for SshHandler {
    type Error = russh::Error;

    fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        if let Ok(guard) = self.forwarded_channels.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(channel);
            }
        }
        async { Ok(()) }
    }

    fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        use russh::keys::known_hosts;
        use russh::keys::HashAlg;

        // Do all known_hosts work synchronously (it's I/O-light file reads).
        // The async block below captures only the `bool` result, which means
        // the returned future doesn't borrow `self` past the function body.
        let result = match known_hosts::check_known_hosts_path(
            &self.host,
            self.port,
            server_public_key,
            &self.known_hosts_path,
        ) {
            Ok(true) => Ok(true),
            // TOFU: print fingerprint, write to known_hosts, accept.
            // Fingerprint Display impl already formats as "SHA256:<base64>".
            Ok(false) => {
                let alg = server_public_key.algorithm().as_str().to_string();
                let fp = server_public_key.fingerprint(HashAlg::Sha256).to_string();
                (self.log)(format!(
                    "Unknown host {}:{} (first connection). {} key fingerprint {}",
                    self.host, self.port, alg, fp
                ));
                match known_hosts::learn_known_hosts_path(
                    &self.host,
                    self.port,
                    server_public_key,
                    &self.known_hosts_path,
                ) {
                    Ok(()) => (self.log)(format!(
                        "Permanently added {}:{} to known_hosts.",
                        self.host, self.port
                    )),
                    Err(e) => (self.log)(format!("known_hosts write failed: {e}")),
                }
                Ok(true)
            }
            // Known host, key changed — reject.
            Err(_) => {
                if let Ok(mut m) = self.key_mismatch.lock() {
                    *m = true;
                }
                Ok(false)
            }
        };
        async move { result }
    }

    fn disconnected(
        &mut self,
        reason: client::DisconnectReason<Self::Error>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async move {
            match reason {
                client::DisconnectReason::ReceivedDisconnect(info) => {
                    log::warn!(
                        "SSH server disconnected: {:?}: {}",
                        info.reason_code,
                        info.message
                    );
                    Ok(())
                }
                client::DisconnectReason::Error(e) => {
                    log::warn!("SSH session error: {e:?}");
                    Err(e)
                }
            }
        }
    }
}

/// Shared forwarded-channel sender, settable from outside.
pub type ForwardedChannelSender =
    Arc<StdMutex<Option<mpsc::UnboundedSender<russh::Channel<client::Msg>>>>>;

fn new_handler(
    host: &str,
    port: u16,
    known_hosts_path: PathBuf,
    log: LogFn,
) -> (SshHandler, Arc<StdMutex<bool>>, ForwardedChannelSender) {
    let mismatch = Arc::new(StdMutex::new(false));
    let fwd_channels: ForwardedChannelSender = Arc::new(StdMutex::new(None));
    let handler = SshHandler {
        host: host.to_string(),
        port,
        known_hosts_path,
        key_mismatch: mismatch.clone(),
        forwarded_channels: fwd_channels.clone(),
        log,
    };
    (handler, mismatch, fwd_channels)
}

fn map_connect_error(
    e: russh::Error,
    host: &str,
    port: u16,
    mismatch: &StdMutex<bool>,
) -> AppError {
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
/// host: String (owned) — every `&str` parameter that survives an await
/// risks tripping the HRTB-Send elaboration bug downstream.
pub async fn ssh_connect(
    config: Arc<client::Config>,
    host: String,
    port: u16,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
    log: LogFn,
) -> AppResult<client::Handle<SshHandler>> {
    let connect_timeout = Duration::from_secs(timeout_secs);
    let (handler, mismatch, _fwd) = new_handler(&host, port, known_hosts_path, log);
    match timeout(
        connect_timeout,
        client::connect(config, (host.as_str(), port), handler),
    )
    .await
    {
        Ok(result) => result.map_err(|e| map_connect_error(e, &host, port, &mismatch)),
        Err(_) => Err(AppError::Ssh(format!(
            "{}:{} 连接超时 ({}s)",
            host, port, timeout_secs
        ))),
    }
}

/// SSH connect that also returns the forwarded channel sender (for remote forwarding).
pub async fn ssh_connect_with_forward(
    config: Arc<client::Config>,
    host: String,
    port: u16,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
    log: LogFn,
) -> AppResult<(client::Handle<SshHandler>, ForwardedChannelSender)> {
    let connect_timeout = Duration::from_secs(timeout_secs);
    let (handler, mismatch, fwd) = new_handler(&host, port, known_hosts_path, log);
    let handle = match timeout(
        connect_timeout,
        client::connect(config, (host.as_str(), port), handler),
    )
    .await
    {
        Ok(result) => result.map_err(|e| map_connect_error(e, &host, port, &mismatch))?,
        Err(_) => {
            return Err(AppError::Ssh(format!(
                "{}:{} 连接超时 ({}s)",
                host, port, timeout_secs
            )))
        }
    };
    Ok((handle, fwd))
}

/// 在已有 stream 上建立 SSH 连接（用于堡垒机隧道）。同时返回 forward channel sender，
/// 让远程转发能注册到末跳 handler。普通调用方丢 `_` 即可。
pub async fn ssh_connect_stream<S>(
    config: Arc<client::Config>,
    stream: S,
    host: String,
    port: u16,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
    log: LogFn,
) -> AppResult<(client::Handle<SshHandler>, ForwardedChannelSender)>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let connect_timeout = Duration::from_secs(timeout_secs);
    let (handler, mismatch, fwd) = new_handler(&host, port, known_hosts_path, log);
    let handle = match timeout(
        connect_timeout,
        client::connect_stream(config, stream, handler),
    )
    .await
    {
        Ok(result) => result.map_err(|e| map_connect_error(e, &host, port, &mismatch))?,
        Err(_) => return Err(AppError::Ssh(format!("{}:{} SSH 握手超时", host, port))),
    };
    Ok((handle, fwd))
}

/// 通过堡垒机链建立到 target 的 SSH 连接。链空则直连 target。
/// 链中每一跳直接 authenticate；target 的 authenticate 由调用方负责。
/// 返回 `(target_handle, target_fwd_sender)` —— remote 转发用 fwd_sender，其余可丢弃。
///
/// All inputs are owned: chain by value, target_host as String, log as Arc<dyn>.
/// Owned-everywhere is correct here, not just convenient — these data flow
/// in one direction (DB → connect → live session), there's no other party
/// holding references. Borrowed parameters in async fns hand-cuff us with
/// HRTB-Send headaches when awaited under #[tauri::command].
pub async fn establish_via_chain(
    bastion_chain: Vec<(Profile, Credential)>,
    target_host: String,
    target_port: u16,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
    log: LogFn,
) -> AppResult<(client::Handle<SshHandler>, ForwardedChannelSender)> {
    let config = default_client_config();

    if bastion_chain.is_empty() {
        log(format!(
            "TCP connecting to {}:{} ...",
            target_host, target_port
        ));
        let (h, fwd) = ssh_connect_with_forward(
            config,
            target_host,
            target_port,
            known_hosts_path,
            timeout_secs,
            log.clone(),
        )
        .await?;
        log(format!("TCP connected. SSH handshake OK."));
        return Ok((h, fwd));
    }

    let mut hops = bastion_chain.into_iter();
    let (first_p, first_c) = hops.next().unwrap();
    let first_name = first_p.name;
    let first_host = first_p.host;
    let first_port = first_p.port;
    log(format!(
        "Connecting to bastion {} ({}:{}) ...",
        first_name, first_host, first_port
    ));
    let mut hop = ssh_connect(
        config.clone(),
        first_host,
        first_port,
        known_hosts_path.clone(),
        timeout_secs,
        log.clone(),
    )
    .await?;
    log(format!(
        "Bastion {} connected. Authenticating as {} ({}) ...",
        first_name,
        first_c.username,
        first_c.credential_type.as_str()
    ));
    authenticate(&mut hop, first_c).await?;
    log(format!("Bastion {} authenticated.", first_name));

    let mut prev_name = first_name;
    for (next_p, next_c) in hops {
        let next_name = next_p.name;
        let next_host = next_p.host;
        let next_port = next_p.port;
        log(format!(
            "Opening tunnel through {} to bastion {} ({}:{}) ...",
            prev_name, next_name, next_host, next_port
        ));
        let tunnel = open_tunnel_with_timeout(
            &hop,
            next_host.clone(),
            next_port,
            timeout_secs,
            format!("{} → {}", prev_name, next_name),
        )
        .await?;
        let (new_hop, _) = ssh_connect_stream(
            config.clone(),
            tunnel.into_stream(),
            next_host,
            next_port,
            known_hosts_path.clone(),
            timeout_secs,
            log.clone(),
        )
        .await?;
        hop = new_hop;
        log(format!(
            "Bastion {} connected. Authenticating as {} ({}) ...",
            next_name,
            next_c.username,
            next_c.credential_type.as_str()
        ));
        authenticate(&mut hop, next_c).await?;
        log(format!("Bastion {} authenticated.", next_name));
        prev_name = next_name;
    }

    log(format!(
        "Opening tunnel through {} to target {}:{} ...",
        prev_name, target_host, target_port
    ));
    let tunnel = open_tunnel_with_timeout(
        &hop,
        target_host.clone(),
        target_port,
        timeout_secs,
        format!("{} → target", prev_name),
    )
    .await?;
    log(format!("Tunnel established. SSH handshake with target ..."));
    ssh_connect_stream(
        config,
        tunnel.into_stream(),
        target_host,
        target_port,
        known_hosts_path,
        timeout_secs,
        log.clone(),
    )
    .await
}

/// 在已建好的 SSH 连接上开 direct-tcpip 隧道，带超时。
/// 没有这个超时，bastion 拨号 target 失败时（VPC 不通 / target 防火墙拒绝 / target 离线）
/// 客户端会一直等 server 返回 `CHANNEL_OPEN_FAILURE`，挂数十秒甚至更久。
///
/// `host` / `label` 都按值传，避免 `&str` 在 await 期间停留；`hop` 必须借用
/// 因为 channel_open_direct_tcpip 是 `&self` 方法。Handle 的内部
/// `Sender<Msg>` 借用是 russh API 决定，无可避免。
async fn open_tunnel_with_timeout(
    hop: &client::Handle<SshHandler>,
    target_host: String,
    target_port: u16,
    timeout_secs: u64,
    label: String,
) -> AppResult<russh::Channel<client::Msg>> {
    let fut =
        hop.channel_open_direct_tcpip(target_host.as_str(), target_port as u32, "127.0.0.1", 0);
    match timeout(Duration::from_secs(timeout_secs), fut).await {
        Ok(r) => r.map_err(|e| AppError::Ssh(format!("堡垒机隧道建立失败 ({label}): {e}"))),
        Err(_) => Err(AppError::Ssh(format!(
            "堡垒机隧道超时 ({label} → {target_host}:{target_port}, {timeout_secs}s)。常见原因：bastion 拨不到 target（VPC 不通 / target 防火墙 / target 离线）。",
        ))),
    }
}

// ---------------------------------------------------------------------------
// 认证 — 全部 owned Credential / String
// ---------------------------------------------------------------------------

fn check_auth_result(result: client::AuthResult) -> AppResult<()> {
    if result.success() {
        Ok(())
    } else {
        Err(AppError::Ssh("认证被拒绝".into()))
    }
}

/// Consumes Credential. For RSA keys, mirror OpenSSH's publickey auth path:
/// read RFC 8308 `server-sig-algs` and use the strongest mutual RSA signature
/// hash, falling back to the base `ssh-rsa` type only when the extension is
/// absent.
pub async fn authenticate(
    handle: &mut client::Handle<SshHandler>,
    credential: Credential,
) -> AppResult<()> {
    match credential.credential_type {
        CredentialType::Password => {
            let pw = credential.secret.unwrap_or_default();
            let result = handle
                .authenticate_password(credential.username, pw)
                .await
                .map_err(|e| AppError::Ssh(format!("密码认证失败: {e}")))?;
            check_auth_result(result)
        }
        CredentialType::Key => {
            let pem = credential
                .secret
                .as_deref()
                .ok_or_else(|| AppError::Ssh("缺少私钥数据".into()))?;
            let key: PrivateKey =
                russh::keys::decode_secret_key(pem, credential.passphrase.as_deref())
                    .map_err(|e| AppError::Ssh(format!("私钥解析失败: {e}")))?;
            authenticate_private_key(handle, credential.username, key).await
        }
        CredentialType::Agent => {
            authenticate_with_agent_or_default_keys(handle, credential.username).await
        }
        CredentialType::None => {
            let result = handle
                .authenticate_none(credential.username)
                .await
                .map_err(|e| AppError::Ssh(format!("认证失败: {e}")))?;
            check_auth_result(result)
        }
        CredentialType::Interactive => Ok(()),
    }
}

/// OpenSSH-compatible RSA signature selection.
///
/// For RSA keys, OpenSSH's `key_sig_algorithm()` uses `server-sig-algs`
/// when present; if the extension is absent it falls back to the key's base
/// signature type (`ssh-rsa`). `russh` represents that base type as `None`.
async fn pick_rsa_hash(
    handle: &client::Handle<SshHandler>,
    key: &PrivateKey,
) -> AppResult<Option<HashAlg>> {
    if !matches!(key.algorithm(), Algorithm::Rsa { .. }) {
        return Ok(None);
    }
    let supported = handle
        .best_supported_rsa_hash()
        .await
        .map_err(|e| AppError::Ssh(format!("RSA 签名算法协商失败: {e}")))?;
    Ok(supported.flatten())
}

fn publickey_signature_label(key: &PrivateKey, rsa_hash: Option<HashAlg>) -> String {
    match key.algorithm() {
        Algorithm::Rsa { .. } => Algorithm::Rsa { hash: rsa_hash }.as_str().to_string(),
        a => a.as_str().to_string(),
    }
}

async fn authenticate_private_key(
    handle: &mut client::Handle<SshHandler>,
    username: String,
    key: PrivateKey,
) -> AppResult<()> {
    let alg = pick_rsa_hash(handle, &key).await?;
    let label = publickey_signature_label(&key, alg);
    let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), alg);
    let result = handle
        .authenticate_publickey(username, key_with_alg)
        .await
        .map_err(|e| AppError::Ssh(format!("密钥认证失败 ({label}): {e}")))?;
    check_auth_result(result)
}

// ---------------------------------------------------------------------------
// SSH Agent 认证
// ---------------------------------------------------------------------------

/// Match OpenSSH's common `ssh user@host` behavior: try the configured agent
/// first, then fall back to default private-key files in ~/.ssh.
pub async fn authenticate_with_agent_or_default_keys(
    handle: &mut client::Handle<SshHandler>,
    username: String,
) -> AppResult<()> {
    let agent_err = match authenticate_with_agent(handle, username.clone()).await {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };

    match authenticate_with_default_keys(handle, username).await {
        Ok(()) => Ok(()),
        Err(key_err) => Err(AppError::Ssh(format!(
            "SSH agent 认证失败: {agent_err}; 默认私钥认证也失败: {key_err}"
        ))),
    }
}

/// 用系统 SSH agent（$SSH_AUTH_SOCK / Pageant）尝试逐个 identity 认证。
pub async fn authenticate_with_agent(
    handle: &mut client::Handle<SshHandler>,
    username: String,
) -> AppResult<()> {
    use russh::keys::agent::client::AgentClient;
    #[cfg(unix)]
    {
        let agent = AgentClient::connect_env()
            .await
            .map_err(|e| AppError::Ssh(format!("无法连接 SSH agent (检查 $SSH_AUTH_SOCK): {e}")))?;
        try_agent_identities(handle, username, agent.dynamic()).await
    }
    #[cfg(windows)]
    {
        let pipe = r"\\.\pipe\openssh-ssh-agent";
        match AgentClient::connect_named_pipe(pipe).await {
            Ok(agent) => try_agent_identities(handle, username, agent.dynamic()).await,
            Err(_) => {
                let agent = AgentClient::connect_pageant().await;
                try_agent_identities(handle, username, agent.dynamic()).await
            }
        }
    }
}

async fn try_agent_identities<S>(
    handle: &mut client::Handle<SshHandler>,
    username: String,
    mut agent: russh::keys::agent::client::AgentClient<S>,
) -> AppResult<()>
where
    S: russh::keys::agent::client::AgentStream + Send + Unpin + 'static,
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

    let rsa_hash = if identities.iter().any(agent_identity_is_rsa) {
        handle
            .best_supported_rsa_hash()
            .await
            .map_err(|e| AppError::Ssh(format!("RSA 签名算法协商失败: {e}")))?
            .flatten()
    } else {
        None
    };

    for identity in identities {
        let hash_alg = if agent_identity_is_rsa(&identity) {
            rsa_hash
        } else {
            None
        };
        let result = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                handle
                    .authenticate_publickey_with(username.clone(), key, hash_alg, &mut agent)
                    .await
            }
            AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(
                        username.clone(),
                        certificate,
                        hash_alg,
                        &mut agent,
                    )
                    .await
            }
        };
        match result {
            Ok(r) if r.success() => return Ok(()),
            Ok(_) => continue,
            Err(e) => log::warn!("agent identity 签名失败: {e}"),
        }
    }
    Err(AppError::Ssh(
        "SSH agent 中所有 identity 都被服务器拒绝".into(),
    ))
}

fn agent_identity_is_rsa(identity: &AgentIdentity) -> bool {
    let algorithm = match identity {
        AgentIdentity::PublicKey { key, .. } => key.algorithm(),
        AgentIdentity::Certificate { certificate, .. } => certificate.algorithm(),
    };
    matches!(algorithm, Algorithm::Rsa { .. })
}

/// Try OpenSSH's default identity files in the order reported by `ssh -G`.
/// This keeps GUI behavior aligned with `ssh user@host` for hosts such as
pub async fn authenticate_with_default_keys(
    handle: &mut client::Handle<SshHandler>,
    username: String,
) -> AppResult<()> {
    let paths = default_identity_paths();
    let mut errors = Vec::new();
    let mut found = 0usize;

    for path in paths {
        let pem = match std::fs::read_to_string(&path) {
            Ok(pem) => pem,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                errors.push(format!("{}: 读取失败 ({e})", path.display()));
                continue;
            }
        };
        found += 1;

        let key: PrivateKey = match russh::keys::decode_secret_key(&pem, None) {
            Ok(key) => key,
            Err(e) => {
                errors.push(format!("{}: 私钥解析失败 ({e})", path.display()));
                continue;
            }
        };

        match authenticate_private_key(handle, username.clone(), key).await {
            Ok(()) => return Ok(()),
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }

    if found == 0 {
        return Err(AppError::Ssh(
            "未找到默认私钥（~/.ssh/id_rsa、id_ecdsa、id_ecdsa_sk、id_ed25519、id_ed25519_sk）"
                .into(),
        ));
    }

    Err(AppError::Ssh(format!(
        "所有默认私钥都不可用: {}",
        errors.join("; ")
    )))
}

fn default_identity_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let ssh_dir = home.join(".ssh");
    [
        "id_rsa",
        "id_ecdsa",
        "id_ecdsa_sk",
        "id_ed25519",
        "id_ed25519_sk",
    ]
    .into_iter()
    .map(|name| ssh_dir.join(name))
    .collect()
}

// ---------------------------------------------------------------------------
// 键盘交互认证
// ---------------------------------------------------------------------------

pub async fn authenticate_interactive(
    handle: &mut client::Handle<SshHandler>,
    username: String,
    app: tauri::AppHandle,
    tab_id: String,
) -> AppResult<()> {
    use russh::client::KeyboardInteractiveAuthResponse;

    let mut reply = handle
        .authenticate_keyboard_interactive_start(username, None::<String>)
        .await
        .map_err(|e| AppError::Ssh(format!("键盘交互启动失败: {e}")))?;

    loop {
        match reply {
            KeyboardInteractiveAuthResponse::Success => return Ok(()),
            KeyboardInteractiveAuthResponse::Failure { .. } => {
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
                    locked(&state.auth_waiters)?.insert(tab_id.clone(), tx);
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

/// All inputs by value: profile / credential / chain / log_session_id all
/// owned. The future returned by this fn carries no external borrows, so
/// `#[tauri::command]` can prove it Send for any caller-supplied state
/// lifetime without HRTB elaboration.
pub async fn connect(
    profile: Profile,
    credential: Credential,
    bastion_chain: Vec<(Profile, Credential)>,
    cols: u32,
    rows: u32,
    app: tauri::AppHandle,
    recording_path: Option<std::path::PathBuf>,
    log_session_id: Option<String>,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
) -> AppResult<ConnectResult> {
    let log: LogFn = match log_session_id.clone() {
        Some(sid) => {
            let app2 = app.clone();
            Arc::new(move |msg: String| {
                let line = format!("\x1b[90m[ssh] {msg}\x1b[0m\r\n");
                let _ = app2.emit(&format!("ssh:data:{sid}"), line.into_bytes());
            })
        }
        None => null_logger(),
    };

    let (mut handle, _fwd) = establish_via_chain(
        bastion_chain,
        profile.host.clone(),
        profile.port,
        known_hosts_path,
        timeout_secs,
        log.clone(),
    )
    .await?;

    log(format!(
        "Authenticating as {} ({}) ...",
        credential.username,
        credential.credential_type.as_str()
    ));
    if credential.credential_type == CredentialType::Interactive {
        let tab_id = log_session_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let username = credential.username.clone();
        authenticate_interactive(&mut handle, username, app.clone(), tab_id).await?;
    } else {
        authenticate(&mut handle, credential).await?;
    }
    log(format!("Authenticated."));

    // Open the shell channel BEFORE wrapping the handle in Arc<Mutex>.
    // Holding a MutexGuard across `.await` would force the resulting future
    // to hold `&Mutex<Handle>` for the inner await — fine for runtime, but
    // the compiler can't always prove that's `for<'a> Send`. Doing the
    // shell setup directly on the owned handle sidesteps the whole issue.
    log(format!("Requesting PTY + shell ..."));
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

    log(format!("Shell ready.\r\n"));

    // Now wrap for downstream multiplexing (SFTP / forwarding share the conn).
    let ssh_handle: SshHandle = Arc::new(tokio::sync::Mutex::new(handle));

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
