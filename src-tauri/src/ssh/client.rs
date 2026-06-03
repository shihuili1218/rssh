use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use russh::client;
use russh::ChannelMsg;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::error::{locked, AppError, AppResult};
use crate::models::{Credential, CredentialType, Profile};
use crate::terminal::recorder::Recorder;

use super::auth::authenticate_interactive;
use super::prompt::prompt_host_key;

/// Re-export 旧 path：实现已迁移到 `ssh::auth` / `ssh::prompt`，调用点
/// （forward / sftp / commands / state）仍可写 `ssh::client::AuthCtx`、
/// `ssh::client::authenticate`，无需扩散改动。
pub use super::auth::authenticate;
pub use super::prompt::AuthCtx;

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
        .map_err(|_| AppError::ssh("ssh_task_cancelled", json!({})))?
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
    /// 终端可达性上下文：有则未知主机走 xterm 内 yes/no/指纹确认；
    /// 无（SFTP / Forward 后台连接）则未知主机直接拒绝。
    prompt_ctx: Option<AuthCtx>,
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

        // 同步部分先做完：known_hosts 查询 + 指纹计算。
        // pubkey 跨 await 边界要 Clone（learn 需要它）。
        let check = known_hosts::check_known_hosts_path(
            &self.host,
            self.port,
            server_public_key,
            &self.known_hosts_path,
        );
        let host = self.host.clone();
        let port = self.port;
        let path = self.known_hosts_path.clone();
        let alg = server_public_key.algorithm().as_str().to_string();
        let fp = server_public_key.fingerprint(HashAlg::Sha256).to_string();
        let pubkey = server_public_key.clone();
        let log = self.log.clone();
        let ctx = self.prompt_ctx.clone();
        let mismatch = self.key_mismatch.clone();

        async move {
            match check {
                Ok(true) => Ok(true),
                // 未知主机：有 ctx 走 xterm 确认，无 ctx（SFTP/Forward 后台连接）直接拒绝。
                Ok(false) => match ctx {
                    Some(c) => handle_unknown_host(c, host, port, alg, fp, pubkey, path, log).await,
                    None => {
                        log(format!(
                            "Unknown host {host}:{port} ({alg} fingerprint {fp}). \
                             No terminal context for confirmation; \
                             connect via SSH terminal first to establish trust."
                        ));
                        Ok(false)
                    }
                },
                // 已知主机但密钥变更：有 ctx 给一次"replace"机会；无 ctx 直接拒绝。
                Err(_) => match ctx {
                    Some(c) => {
                        handle_key_mismatch(c, host, port, alg, fp, pubkey, path, log, mismatch)
                            .await
                    }
                    None => {
                        if let Ok(mut m) = mismatch.lock() {
                            *m = true;
                        }
                        Ok(false)
                    }
                },
            }
        }
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

/// TOFU 第一次连：弹 banner → 用户输 yes / 指纹 / 其它 → learn 到 known_hosts。
/// 失败（取消 / 拒绝）一律返回 Ok(false) 让 russh 终止握手，不抛错——是用户主动选的。
async fn handle_unknown_host(
    ctx: AuthCtx,
    host: String,
    port: u16,
    alg: String,
    fp: String,
    pubkey: russh::keys::ssh_key::PublicKey,
    path: PathBuf,
    log: LogFn,
) -> Result<bool, russh::Error> {
    use russh::keys::known_hosts;

    let banner = format!(
        "\r\nThe authenticity of host '{host}' can't be established.\r\n\
         {alg} key fingerprint is {fp}.\r\n\
         This key is not known by any other names.\r\n\
         Are you sure you want to continue connecting (yes/no/[fingerprint])? "
    );
    let answer = match prompt_host_key(&ctx, &banner).await {
        Ok(a) => a,
        Err(_) => {
            log(format!("Host key confirmation cancelled for {host}:{port}."));
            return Ok(false);
        }
    };
    let trimmed = answer.trim();
    if !(trimmed.eq_ignore_ascii_case("yes") || trimmed == fp) {
        log(format!("Host key rejected by user for {host}:{port}."));
        return Ok(false);
    }
    match known_hosts::learn_known_hosts_path(&host, port, &pubkey, &path) {
        Ok(()) => log(format!("Permanently added {host}:{port} to known_hosts.")),
        Err(e) => log(format!("known_hosts write failed: {e}")),
    }
    Ok(true)
}

/// 已知 host 但密钥变了：MITM 警告 → 用户输 'replace' 才删旧加新。
/// 字面 'replace'（不是 'yes'）是设计上加大手滑成本——这是潜在中间人攻击场景。
/// 任何路径下 mismatch flag 都要置位（取消 / 拒绝 / 删除失败），让上层把错误码翻成
/// `ssh_host_key_changed` 而不是泛泛的 connect 失败。
async fn handle_key_mismatch(
    ctx: AuthCtx,
    host: String,
    port: u16,
    alg: String,
    fp: String,
    pubkey: russh::keys::ssh_key::PublicKey,
    path: PathBuf,
    log: LogFn,
    mismatch: Arc<StdMutex<bool>>,
) -> Result<bool, russh::Error> {
    use russh::keys::known_hosts;
    use russh::keys::HashAlg;

    let set_mismatch = || {
        if let Ok(mut m) = mismatch.lock() {
            *m = true;
        }
    };

    let old_fps: Vec<String> = known_hosts::known_host_keys_path(&host, port, &path)
        .ok()
        .unwrap_or_default()
        .into_iter()
        .map(|(_, k)| k.fingerprint(HashAlg::Sha256).to_string())
        .collect();
    let old_fps_str = if old_fps.is_empty() {
        "(unknown)".to_string()
    } else {
        old_fps.join("\r\n  ")
    };
    let banner = format!(
        "\r\n@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n\
         @    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\r\n\
         @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\r\n\
         IT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!\r\n\
         Someone could be eavesdropping on you right now (man-in-the-middle attack)!\r\n\
         \r\n\
         Host: {host}:{port}\r\n\
         Old key fingerprint:\r\n  {old_fps_str}\r\n\
         New key fingerprint:\r\n  {fp} ({alg})\r\n\
         \r\n\
         If the server was legitimately reinstalled, type 'replace' to remove\r\n\
         the old key and trust the new one. Anything else aborts.\r\n\
         > "
    );
    let answer = match prompt_host_key(&ctx, &banner).await {
        Ok(a) => a,
        Err(_) => {
            set_mismatch();
            log(format!("Host key change confirmation cancelled for {host}:{port}."));
            return Ok(false);
        }
    };
    if answer.trim() != "replace" {
        set_mismatch();
        log(format!("Host key change rejected by user for {host}:{port}."));
        return Ok(false);
    }
    match crate::ssh::known_hosts::remove_host(&host, port, &path) {
        Ok(n) => log(format!("Removed {n} stale entry/entries for {host}:{port}.")),
        Err(e) => {
            log(format!("Failed to remove old known_hosts entry: {e}"));
            set_mismatch();
            return Ok(false);
        }
    }
    match known_hosts::learn_known_hosts_path(&host, port, &pubkey, &path) {
        Ok(()) => log(format!("New host key for {host}:{port} added to known_hosts.")),
        Err(e) => log(format!("known_hosts write failed: {e}")),
    }
    Ok(true)
}

/// Shared forwarded-channel sender, settable from outside.
pub type ForwardedChannelSender =
    Arc<StdMutex<Option<mpsc::UnboundedSender<russh::Channel<client::Msg>>>>>;

/// The constant context for dialing one SSH endpoint: everything that stays the
/// same across every hop of a bastion chain. Only `host`/`port` (and the relayed
/// `stream` for tunneled hops) vary per call, so those stay separate parameters.
/// `ssh_connect` / `ssh_connect_with_forward` / `ssh_connect_stream` each carried
/// these five identically before.
#[derive(Clone)]
pub struct DialCtx {
    pub config: Arc<client::Config>,
    pub known_hosts_path: PathBuf,
    pub timeout_secs: u64,
    pub log: LogFn,
    pub prompt_ctx: Option<AuthCtx>,
}

fn new_handler(
    host: &str,
    port: u16,
    known_hosts_path: PathBuf,
    log: LogFn,
    prompt_ctx: Option<AuthCtx>,
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
        prompt_ctx,
    };
    (handler, mismatch, fwd_channels)
}

fn map_connect_error(
    e: russh::Error,
    host: &str,
    port: u16,
    mismatch: &StdMutex<bool>,
) -> AppError {
    // Mismatch flag 只是一个 bool；中毒理论不会发生，但仍走 locked() 让 panic 路径
    // 转成 AppError::Lock 而不是直接 unwrap panic。
    let changed = locked(mismatch).map(|g| *g).unwrap_or(false);
    if changed {
        AppError::ssh(
            "ssh_host_key_changed",
            json!({ "host": host, "port": port }),
        )
    } else {
        AppError::ssh("ssh_connect_failed", json!({ "err": e.to_string() }))
    }
}

/// 建立 SSH 连接并验证主机密钥（带超时）。
/// host: String (owned) — every `&str` parameter that survives an await
/// risks tripping the HRTB-Send elaboration bug downstream.
pub async fn ssh_connect(
    ctx: DialCtx,
    host: String,
    port: u16,
) -> AppResult<client::Handle<SshHandler>> {
    let DialCtx { config, known_hosts_path, timeout_secs, log, prompt_ctx } = ctx;
    let connect_timeout = Duration::from_secs(timeout_secs);
    let (handler, mismatch, _fwd) = new_handler(&host, port, known_hosts_path, log, prompt_ctx);
    match timeout(
        connect_timeout,
        client::connect(config, (host.as_str(), port), handler),
    )
    .await
    {
        Ok(result) => result.map_err(|e| map_connect_error(e, &host, port, &mismatch)),
        Err(_) => Err(AppError::ssh(
            "ssh_connect_timeout",
            json!({ "host": host, "port": port, "secs": timeout_secs }),
        )),
    }
}

/// SSH connect that also returns the forwarded channel sender (for remote forwarding).
pub async fn ssh_connect_with_forward(
    ctx: DialCtx,
    host: String,
    port: u16,
) -> AppResult<(client::Handle<SshHandler>, ForwardedChannelSender)> {
    let DialCtx { config, known_hosts_path, timeout_secs, log, prompt_ctx } = ctx;
    let connect_timeout = Duration::from_secs(timeout_secs);
    let (handler, mismatch, fwd) = new_handler(&host, port, known_hosts_path, log, prompt_ctx);
    let handle = match timeout(
        connect_timeout,
        client::connect(config, (host.as_str(), port), handler),
    )
    .await
    {
        Ok(result) => result.map_err(|e| map_connect_error(e, &host, port, &mismatch))?,
        Err(_) => {
            return Err(AppError::ssh(
                "ssh_connect_timeout",
                json!({ "host": host, "port": port, "secs": timeout_secs }),
            ))
        }
    };
    Ok((handle, fwd))
}

/// 在已有 stream 上建立 SSH 连接（用于堡垒机隧道）。同时返回 forward channel sender，
/// 让远程转发能注册到末跳 handler。普通调用方丢 `_` 即可。
pub async fn ssh_connect_stream<S>(
    ctx: DialCtx,
    stream: S,
    host: String,
    port: u16,
) -> AppResult<(client::Handle<SshHandler>, ForwardedChannelSender)>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let DialCtx { config, known_hosts_path, timeout_secs, log, prompt_ctx } = ctx;
    let connect_timeout = Duration::from_secs(timeout_secs);
    let (handler, mismatch, fwd) = new_handler(&host, port, known_hosts_path, log, prompt_ctx);
    let handle = match timeout(
        connect_timeout,
        client::connect_stream(config, stream, handler),
    )
    .await
    {
        Ok(result) => result.map_err(|e| map_connect_error(e, &host, port, &mismatch))?,
        Err(_) => return Err(AppError::ssh("ssh_handshake_timeout", json!({ "host": host, "port": port }))),
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
    ctx: Option<&AuthCtx>,
) -> AppResult<(client::Handle<SshHandler>, ForwardedChannelSender)> {
    // The dial context is constant across every hop; only host/port (and the
    // tunnel stream) change. Build it once and hand a clone to each hop.
    let dial = DialCtx {
        config: default_client_config(),
        known_hosts_path,
        timeout_secs,
        log: log.clone(),
        prompt_ctx: ctx.cloned(),
    };

    if bastion_chain.is_empty() {
        log(format!(
            "TCP connecting to {}:{} ...",
            target_host, target_port
        ));
        let (h, fwd) = ssh_connect_with_forward(dial, target_host, target_port).await?;
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
    let mut hop = ssh_connect(dial.clone(), first_host, first_port).await?;
    log(format!(
        "Bastion {} connected. Authenticating as {} ({}) ...",
        first_name,
        first_c.username,
        first_c.credential_type.as_str()
    ));
    authenticate(&mut hop, first_c, ctx).await?;
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
        let (new_hop, _) =
            ssh_connect_stream(dial.clone(), tunnel.into_stream(), next_host, next_port).await?;
        hop = new_hop;
        log(format!(
            "Bastion {} connected. Authenticating as {} ({}) ...",
            next_name,
            next_c.username,
            next_c.credential_type.as_str()
        ));
        authenticate(&mut hop, next_c, ctx).await?;
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
    ssh_connect_stream(dial, tunnel.into_stream(), target_host, target_port).await
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
        Ok(r) => r.map_err(|e| AppError::ssh(
            "ssh_bastion_tunnel_failed",
            json!({ "label": &label, "err": e.to_string() }),
        )),
        Err(_) => Err(AppError::ssh(
            "ssh_bastion_tunnel_timeout",
            json!({
                "label": &label,
                "target_host": target_host,
                "target_port": target_port,
                "secs": timeout_secs,
            }),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── map_connect_error ──────────────────────────────────────────

    #[test]
    fn map_connect_error_when_mismatch_flag_set() {
        let mismatch = StdMutex::new(true);
        let err = map_connect_error(russh::Error::Version, "h", 22, &mismatch);
        assert_eq!(err.code(), "ssh_host_key_changed");
    }

    #[test]
    fn map_connect_error_when_no_mismatch() {
        let mismatch = StdMutex::new(false);
        let err = map_connect_error(russh::Error::Version, "h", 22, &mismatch);
        assert_eq!(err.code(), "ssh_connect_failed");
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
    /// 本次连接对应的 Profile.id。AI 模块用它作 remote_shell_cache 的 key ——
    /// 同一 profile 多次重连共享一份探测结果，不重发 probe。
    profile_id: String,
}

impl SessionHandle {
    pub fn write(&self, data: &[u8]) -> AppResult<()> {
        self.tx
            .send(SessionCmd::Write(data.to_vec()))
            .map_err(|_| AppError::ssh("ssh_session_closed", json!({})))
    }
    pub fn resize(&self, cols: u32, rows: u32) -> AppResult<()> {
        self.tx
            .send(SessionCmd::Resize { cols, rows })
            .map_err(|_| AppError::ssh("ssh_session_closed", json!({})))
    }
    pub fn ssh_handle(&self) -> &SshHandle {
        &self.ssh_handle
    }
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    /// 强制断开整条 SSH 连接 —— 不只是 shell channel，连 TCP 一起切。
    ///
    /// 用途：用户关 tab / 关窗口时调用。所有挂在这条 SSH 上的子资源
    /// （SFTP transfer、forward listener 等）会因为底层 socket 被切，
    /// 下一次 read/write 立刻 IO error 退出。
    ///
    /// 跑在 SSH worker 线程里 —— `Handle::disconnect` 走 russh，
    /// 必须在原 runtime 上下文。所以 dispatch 出去。
    pub fn force_disconnect(&self) {
        // 先发 Close 让 session_task 优雅退出 shell channel
        let _ = self.tx.send(SessionCmd::Close);

        let ssh_handle = self.ssh_handle.clone();
        let _ = spawn_ssh::<_, _, ()>(move || async move {
            let h = ssh_handle.lock().await;
            // ByApplication = 用户主动断；空 message + 空 lang 是合规的最小 payload
            let _ = h
                .disconnect(russh::Disconnect::ByApplication, "", "")
                .await;
            Ok(())
        });
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
    app: crate::emitter::Host,
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

    // 终端可达性上下文：只要有 tab_id 就能给用户弹 passphrase 提示。
    // 即使 verbose log 关闭、`log` 是 null_logger，passphrase 提示仍然能发。
    let ctx = log_session_id.clone().map(|tab_id| AuthCtx {
        app: app.clone(),
        tab_id,
    });

    let (mut handle, _fwd) = establish_via_chain(
        bastion_chain,
        profile.host.clone(),
        profile.port,
        known_hosts_path,
        timeout_secs,
        log.clone(),
        ctx.as_ref(),
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
        authenticate(&mut handle, credential, ctx.as_ref()).await?;
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
        .map_err(|e| AppError::ssh("ssh_open_channel_failed", json!({ "err": e.to_string() })))?;

    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .map_err(|e| AppError::ssh("ssh_pty_request_failed", json!({ "err": e.to_string() })))?;

    channel
        .request_shell(false)
        .await
        .map_err(|e| AppError::ssh("ssh_shell_request_failed", json!({ "err": e.to_string() })))?;

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
        handle: SessionHandle {
            tx,
            ssh_handle,
            profile_id: profile.id.clone(),
        },
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
    app: crate::emitter::Host,
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
                    let _ = rec.record(&data);
                }
                let _ = app.emit(&data_event, data.to_vec());
            }
            Event::Ssh(Some(ChannelMsg::ExtendedData { data, .. })) => {
                if let Some(ref mut rec) = recorder {
                    let _ = rec.record(&data);
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
