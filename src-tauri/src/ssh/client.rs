use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use russh::client;
use russh::keys::agent::AgentIdentity;
use russh::keys::{Algorithm, HashAlg, PrivateKey, PrivateKeyWithHashAlg};
use russh::ChannelMsg;
use serde_json::json;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::error::{locked, AppError, AppResult};
use crate::models::{Credential, CredentialType, Profile};
use crate::state::AppState;
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

// ---------------------------------------------------------------------------
// Passphrase 交互上下文
// ---------------------------------------------------------------------------

/// 终端可达性上下文 — 提供给 `authenticate_*` 系列函数，让它们在遇到加密
/// 私钥时能向具体的终端 tab 弹"输入 passphrase"提示。
///
/// 没有 `AuthCtx` 的场景（forward / SFTP 子模块）会向加密私钥直接报错；
/// 用户需先通过 SSH 终端连一次以填充进程内 passphrase 缓存，后续 forward/SFTP
/// 可命中缓存自动通过。
#[derive(Clone)]
pub struct AuthCtx {
    pub app: tauri::AppHandle,
    pub tab_id: String,
}

const MAX_PASSPHRASE_RETRIES: usize = 3;

/// 解析私钥，遇加密时按需向终端索取 passphrase。
///
/// `cache_key` 唯一标识这把私钥在本次进程内的 passphrase 缓存项：
/// 存储凭证用 `cred:{credential_id}`，默认密钥文件用绝对路径，临时直连凭证
/// 没缓存。`prompt_label` 是直接显示给用户的提示行（含末尾冒号空格）。
async fn decode_key_with_prompt(
    pem: &str,
    cache_key: Option<&str>,
    prompt_label: &str,
    ctx: Option<&AuthCtx>,
) -> AppResult<PrivateKey> {
    use russh::keys::Error::KeyIsEncrypted;

    // 第一次：试无密码（未加密的 key 直接通过；加密的 key 才进入下面流程）
    match russh::keys::decode_secret_key(pem, None) {
        Ok(k) => return Ok(k),
        Err(KeyIsEncrypted) => {}
        Err(e) => return Err(AppError::ssh("ssh_privkey_parse_failed", json!({ "err": e.to_string() }))),
    }

    // 命中缓存 → 直接重试；不命中或失败再走交互
    if let (Some(key), Some(ctx)) = (cache_key, ctx) {
        let cached = {
            let state = ctx.app.state::<AppState>();
            locked(&state.passphrase_cache)
                .ok()
                .and_then(|m| m.get(key).cloned())
        };
        if let Some(pw) = cached {
            match russh::keys::decode_secret_key(pem, Some(&pw)) {
                Ok(k) => return Ok(k),
                Err(KeyIsEncrypted) => {
                    // 缓存的 passphrase 不再匹配（用户改了密码）— 清掉再走交互
                    let state = ctx.app.state::<AppState>();
                    if let Ok(mut m) = locked(&state.passphrase_cache) {
                        m.remove(key);
                    };
                }
                Err(e) => return Err(AppError::ssh("ssh_privkey_parse_failed", json!({ "err": e.to_string() }))),
            }
        }
    }

    // 必须有 ctx 才能交互；否则该流程拒绝加密私钥（forward / SFTP 等）
    let ctx = ctx.ok_or_else(|| AppError::ssh("ssh_privkey_encrypted_no_ctx", json!({})))?;

    // 最多 N 次重试
    for attempt in 0..MAX_PASSPHRASE_RETRIES {
        let pw = prompt_passphrase(ctx, prompt_label).await?;
        match russh::keys::decode_secret_key(pem, Some(&pw)) {
            Ok(k) => {
                if let Some(key) = cache_key {
                    let state = ctx.app.state::<AppState>();
                    if let Ok(mut m) = locked(&state.passphrase_cache) {
                        m.insert(key.to_string(), pw);
                    };
                }
                return Ok(k);
            }
            Err(KeyIsEncrypted) => {
                let remaining = MAX_PASSPHRASE_RETRIES - attempt - 1;
                let msg = if remaining > 0 {
                    format!("\x1b[31mIncorrect passphrase, {remaining} attempt(s) left.\x1b[0m\r\n")
                } else {
                    "\x1b[31mIncorrect passphrase.\x1b[0m\r\n".to_string()
                };
                let _ = ctx
                    .app
                    .emit(&format!("ssh:data:{}", ctx.tab_id), msg.into_bytes());
            }
            Err(e) => return Err(AppError::ssh("ssh_privkey_parse_failed", json!({ "err": e.to_string() }))),
        }
    }

    Err(AppError::ssh("ssh_passphrase_too_many", json!({})))
}

/// 通用终端 prompt：注册 oneshot sender 到指定 waiters map，emit 事件，等用户回应。
/// passphrase / host_key 等 xterm 内交互都走这条路；差异只在 waiters / 事件名 / payload。
async fn prompt_oneshot(
    waiters: &std::sync::Mutex<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<String>>,
    >,
    app: &tauri::AppHandle,
    tab_id: &str,
    event_prefix: &str,
    payload: serde_json::Value,
    cancel_code: &'static str,
) -> AppResult<String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    {
        let mut w = locked(waiters)?;
        // 同一 tab_id 上若已有等待中的 prompt（理论不会发生），覆盖旧 sender
        w.insert(tab_id.to_string(), tx);
    }
    app.emit(&format!("{event_prefix}:{tab_id}"), payload)
        .map_err(|e| AppError::other("emit_failed", json!({ "channel": event_prefix, "err": e.to_string() })))?;
    rx.await.map_err(|_| AppError::ssh(cancel_code, json!({})))
}

/// 向终端弹一次 passphrase 提示，等用户输完回车。
async fn prompt_passphrase(ctx: &AuthCtx, prompt: &str) -> AppResult<String> {
    let state = ctx.app.state::<AppState>();
    prompt_oneshot(
        &state.passphrase_waiters,
        &ctx.app,
        &ctx.tab_id,
        "ssh:passphrase_prompt",
        serde_json::json!({ "prompt": prompt }),
        "ssh_user_cancelled_passphrase",
    )
    .await
}

/// 向终端弹一次主机密钥 TOFU 确认，等用户输入 yes / no / 指纹。
/// 调用方负责按返回字符串决定是否信任。
async fn prompt_host_key(ctx: &AuthCtx, banner: &str) -> AppResult<String> {
    let state = ctx.app.state::<AppState>();
    prompt_oneshot(
        &state.host_key_waiters,
        &ctx.app,
        &ctx.tab_id,
        "ssh:host_key_prompt",
        serde_json::json!({ "banner": banner }),
        "ssh_user_cancelled_hostkey",
    )
    .await
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
                // 未知主机：有 ctx 走 xterm 确认，无 ctx 直接拒绝。
                Ok(false) => match ctx {
                    Some(ctx) => {
                        let banner = format!(
                            "\r\nThe authenticity of host '{host}' can't be established.\r\n\
                             {alg} key fingerprint is {fp}.\r\n\
                             This key is not known by any other names.\r\n\
                             Are you sure you want to continue connecting (yes/no/[fingerprint])? "
                        );
                        let answer = match prompt_host_key(&ctx, &banner).await {
                            Ok(a) => a,
                            Err(_) => {
                                log(format!(
                                    "Host key confirmation cancelled for {host}:{port}."
                                ));
                                return Ok(false);
                            }
                        };
                        let trimmed = answer.trim();
                        if trimmed.eq_ignore_ascii_case("yes") || trimmed == fp {
                            match known_hosts::learn_known_hosts_path(&host, port, &pubkey, &path)
                            {
                                Ok(()) => log(format!(
                                    "Permanently added {host}:{port} to known_hosts."
                                )),
                                Err(e) => log(format!("known_hosts write failed: {e}")),
                            }
                            Ok(true)
                        } else {
                            log(format!("Host key rejected by user for {host}:{port}."));
                            Ok(false)
                        }
                    }
                    None => {
                        log(format!(
                            "Unknown host {host}:{port} ({alg} fingerprint {fp}). \
                             No terminal context for confirmation; \
                             connect via SSH terminal first to establish trust."
                        ));
                        Ok(false)
                    }
                },
                // 已知主机但密钥变更：有 ctx 给一次"replace"机会（移动端没法跑 ssh-keygen -R）；
                // 无 ctx 直接拒绝。要求字面输入 'replace' 而非 'yes'，加大手滑成本。
                Err(_) => match ctx {
                    Some(ctx) => {
                        let old_fps: Vec<String> =
                            russh::keys::known_hosts::known_host_keys_path(&host, port, &path)
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
                                if let Ok(mut m) = mismatch.lock() {
                                    *m = true;
                                }
                                log(format!(
                                    "Host key change confirmation cancelled for {host}:{port}."
                                ));
                                return Ok(false);
                            }
                        };
                        if answer.trim() == "replace" {
                            match crate::ssh::known_hosts::remove_host(&host, port, &path) {
                                Ok(n) => log(format!(
                                    "Removed {n} stale entry/entries for {host}:{port}."
                                )),
                                Err(e) => {
                                    log(format!("Failed to remove old known_hosts entry: {e}"));
                                    if let Ok(mut m) = mismatch.lock() {
                                        *m = true;
                                    }
                                    return Ok(false);
                                }
                            }
                            match known_hosts::learn_known_hosts_path(&host, port, &pubkey, &path) {
                                Ok(()) => log(format!(
                                    "New host key for {host}:{port} added to known_hosts."
                                )),
                                Err(e) => log(format!("known_hosts write failed: {e}")),
                            }
                            Ok(true)
                        } else {
                            if let Ok(mut m) = mismatch.lock() {
                                *m = true;
                            }
                            log(format!("Host key change rejected by user for {host}:{port}."));
                            Ok(false)
                        }
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

/// Shared forwarded-channel sender, settable from outside.
pub type ForwardedChannelSender =
    Arc<StdMutex<Option<mpsc::UnboundedSender<russh::Channel<client::Msg>>>>>;

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
    if *mismatch.lock().unwrap() {
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
    config: Arc<client::Config>,
    host: String,
    port: u16,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
    log: LogFn,
    prompt_ctx: Option<AuthCtx>,
) -> AppResult<client::Handle<SshHandler>> {
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
    config: Arc<client::Config>,
    host: String,
    port: u16,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
    log: LogFn,
    prompt_ctx: Option<AuthCtx>,
) -> AppResult<(client::Handle<SshHandler>, ForwardedChannelSender)> {
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
    config: Arc<client::Config>,
    stream: S,
    host: String,
    port: u16,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
    log: LogFn,
    prompt_ctx: Option<AuthCtx>,
) -> AppResult<(client::Handle<SshHandler>, ForwardedChannelSender)>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
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
            ctx.cloned(),
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
        ctx.cloned(),
    )
    .await?;
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
        let (new_hop, _) = ssh_connect_stream(
            config.clone(),
            tunnel.into_stream(),
            next_host,
            next_port,
            known_hosts_path.clone(),
            timeout_secs,
            log.clone(),
            ctx.cloned(),
        )
        .await?;
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
    ssh_connect_stream(
        config,
        tunnel.into_stream(),
        target_host,
        target_port,
        known_hosts_path,
        timeout_secs,
        log.clone(),
        ctx.cloned(),
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

// ---------------------------------------------------------------------------
// 认证 — 全部 owned Credential / String
// ---------------------------------------------------------------------------

fn check_auth_result(result: client::AuthResult) -> AppResult<()> {
    if result.success() {
        Ok(())
    } else {
        Err(AppError::ssh("ssh_auth_rejected", json!({})))
    }
}

/// Consumes Credential. For RSA keys, mirror OpenSSH's publickey auth path:
/// read RFC 8308 `server-sig-algs` and use the strongest mutual RSA signature
/// hash, falling back to the base `ssh-rsa` type only when the extension is
/// absent.
///
/// `ctx` 提供终端反馈通道：加密私钥会在终端内提示输入 passphrase；
/// 为 `None` 时（forward / SFTP 等子模块）加密私钥直接报错。
pub async fn authenticate(
    handle: &mut client::Handle<SshHandler>,
    credential: Credential,
    ctx: Option<&AuthCtx>,
) -> AppResult<()> {
    match credential.credential_type {
        CredentialType::Password => {
            let pw = credential.secret.unwrap_or_default();
            let result = handle
                .authenticate_password(credential.username, pw)
                .await
                .map_err(|e| AppError::ssh("ssh_password_auth_failed", json!({ "err": e.to_string() })))?;
            check_auth_result(result)
        }
        CredentialType::Key => {
            let pem = credential
                .secret
                .as_deref()
                .ok_or_else(|| AppError::ssh("ssh_privkey_missing", json!({})))?;
            let cache_key = if credential.id.is_empty() {
                None
            } else {
                Some(format!("cred:{}", credential.id))
            };
            let prompt_label = format!(
                "Enter passphrase for key '{}': ",
                if credential.name.is_empty() {
                    credential.username.as_str()
                } else {
                    credential.name.as_str()
                }
            );
            let key = decode_key_with_prompt(pem, cache_key.as_deref(), &prompt_label, ctx).await?;
            authenticate_private_key(handle, credential.username, key).await
        }
        CredentialType::Agent => {
            authenticate_with_agent_or_default_keys(handle, credential.username, ctx).await
        }
        CredentialType::None => {
            let result = handle
                .authenticate_none(credential.username)
                .await
                .map_err(|e| AppError::ssh("ssh_auth_failed", json!({ "err": e.to_string() })))?;
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
        .map_err(|e| AppError::ssh("ssh_rsa_sigalg_failed", json!({ "err": e.to_string() })))?;
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
        .map_err(|e| AppError::ssh("ssh_pubkey_auth_failed", json!({ "label": &label, "err": e.to_string() })))?;
    check_auth_result(result)
}

// ---------------------------------------------------------------------------
// SSH Agent 认证
// ---------------------------------------------------------------------------

/// Match OpenSSH's common `ssh user@host` behavior: try the configured agent
/// first, then fall back to default private-key files in ~/.ssh.
///
/// 默认密钥若加密会通过 `ctx` 在终端内索取 passphrase；ctx 为 None 时
/// 加密的默认密钥被跳过（保留旧行为，避免 forward 场景死锁）。
pub async fn authenticate_with_agent_or_default_keys(
    handle: &mut client::Handle<SshHandler>,
    username: String,
    ctx: Option<&AuthCtx>,
) -> AppResult<()> {
    let agent_err = match authenticate_with_agent(handle, username.clone()).await {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    match authenticate_with_default_keys(handle, username, ctx).await {
        Ok(()) => Ok(()),
        // default keys 完全没文件可试 → fallback 没条件走，agent_err 才是真正失败原因
        // （Agent 凭证类型用户明确依赖 agent，丢掉 agent_err 会得到误导性的"默认密钥不存在"）。
        Err(key_err) if key_err.code() == "ssh_default_keys_not_found" => Err(agent_err),
        Err(key_err) => Err(key_err),
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
            .map_err(|e| AppError::ssh("ssh_agent_unix_connect_failed", json!({ "err": e.to_string() })))?;
        try_agent_identities(handle, username, agent.dynamic()).await
    }
    #[cfg(windows)]
    {
        // 优先 OpenSSH for Windows 命名管道；不通时再退到 Pageant。
        // 两个 connect 都返回 Result —— 前面少了一次解包，导致 .dynamic() 在
        // Result 上找不到，windows 这边编译就挂。
        let pipe = r"\\.\pipe\openssh-ssh-agent";
        if let Ok(agent) = AgentClient::connect_named_pipe(pipe).await {
            return try_agent_identities(handle, username, agent.dynamic()).await;
        }
        let agent = AgentClient::connect_pageant()
            .await
            .map_err(|e| AppError::ssh("ssh_agent_pageant_failed", json!({ "err": e.to_string() })))?;
        try_agent_identities(handle, username, agent.dynamic()).await
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
        .map_err(|e| AppError::ssh("ssh_agent_list_failed", json!({ "err": e.to_string() })))?;

    if identities.is_empty() {
        return Err(AppError::ssh("ssh_agent_no_identity", json!({})));
    }

    let rsa_hash = if identities.iter().any(agent_identity_is_rsa) {
        handle
            .best_supported_rsa_hash()
            .await
            .map_err(|e| AppError::ssh("ssh_rsa_sigalg_failed", json!({ "err": e.to_string() })))?
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
            Err(e) => log::warn!("agent identity sign failed: {e}"),
        }
    }
    Err(AppError::ssh("ssh_agent_all_rejected", json!({})))
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
/// tmate that accept only publickey auth.
///
/// 加密的默认私钥：有 ctx 则终端内索取 passphrase，无 ctx 则跳过该文件
/// 并把错误记入 errors（避免 forward 类无界面流程卡住）。
pub async fn authenticate_with_default_keys(
    handle: &mut client::Handle<SshHandler>,
    username: String,
    ctx: Option<&AuthCtx>,
) -> AppResult<()> {
    let paths = default_identity_paths();
    // 只记最后一条 (path, code) — 多个 key 都失败时，第一条与最后一条 code 一般差不多，
    // 给前端一条标量信息足够；要全量明细去看 stderr/日志。
    let mut last_path: Option<String> = None;
    let mut last_code: Option<&'static str> = None;
    let mut found = 0usize;

    for path in paths {
        let pem = match std::fs::read_to_string(&path) {
            Ok(pem) => pem,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                last_path = Some(path.display().to_string());
                last_code = Some("io_error");
                continue;
            }
        };
        found += 1;

        let cache_key = path.to_string_lossy().into_owned();
        let prompt_label = format!("Enter passphrase for {}: ", path.display());
        let key = match decode_key_with_prompt(&pem, Some(&cache_key), &prompt_label, ctx).await {
            Ok(k) => k,
            Err(e) => {
                last_path = Some(path.display().to_string());
                last_code = Some(e.code());
                continue;
            }
        };

        match authenticate_private_key(handle, username.clone(), key).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_path = Some(path.display().to_string());
                last_code = Some(e.code());
            }
        }
    }

    if found == 0 {
        return Err(AppError::ssh("ssh_default_keys_not_found", json!({})));
    }

    Err(AppError::ssh(
        "ssh_default_keys_unavailable",
        json!({
            "path": last_path.unwrap_or_default(),
            "code": last_code.unwrap_or("unknown"),
        }),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── check_auth_result ──────────────────────────────────────────

    #[test]
    fn check_auth_success() {
        assert!(check_auth_result(client::AuthResult::Success).is_ok());
    }

    #[test]
    fn check_auth_failure_maps_to_ssh_auth_rejected() {
        let result = client::AuthResult::Failure {
            remaining_methods: russh::MethodSet::empty(),
            partial_success: false,
        };
        let err = check_auth_result(result).unwrap_err();
        assert_eq!(err.code(), "ssh_auth_rejected");
    }

    // ── default_identity_paths ─────────────────────────────────────

    #[test]
    fn default_identity_paths_match_openssh_order_when_home_present() {
        // CI 环境 HOME 不存在的话函数返回空 Vec — 那就跳过断言。
        if dirs::home_dir().is_none() {
            assert!(default_identity_paths().is_empty());
            return;
        }
        let paths = default_identity_paths();
        assert_eq!(paths.len(), 5);
        let names: Vec<_> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["id_rsa", "id_ecdsa", "id_ecdsa_sk", "id_ed25519", "id_ed25519_sk"]
        );
        // 全部位于 .ssh/ 子目录
        for p in &paths {
            assert!(p.parent().unwrap().ends_with(".ssh"));
        }
    }

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

    // ── publickey_signature_label ──────────────────────────────────

    #[test]
    fn publickey_label_for_ed25519_ignores_rsa_hash() {
        let kp = russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&[7u8; 32]);
        let key: PrivateKey = kp.into();
        // 即便传 SHA-512，ed25519 也走 algorithm.as_str() 那条路
        let label = publickey_signature_label(&key, Some(HashAlg::Sha512));
        assert_eq!(label, "ssh-ed25519");
        // 没传 hash 一样
        let label2 = publickey_signature_label(&key, None);
        assert_eq!(label2, "ssh-ed25519");
    }
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
        .map_err(|e| AppError::ssh("ssh_kbi_start_failed", json!({ "err": e.to_string() })))?;

    loop {
        match reply {
            KeyboardInteractiveAuthResponse::Success => return Ok(()),
            KeyboardInteractiveAuthResponse::Failure { .. } => {
                return Err(AppError::ssh("ssh_auth_rejected", json!({})));
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
                    .map_err(|_| AppError::ssh("ssh_user_cancelled_auth", json!({})))?;

                reply = handle
                    .authenticate_keyboard_interactive_respond(responses)
                    .await
                    .map_err(|e| AppError::ssh("ssh_kbi_response_failed", json!({ "err": e.to_string() })))?;
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
