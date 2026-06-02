//! 认证 — password / private-key / agent / kbd-interactive 各路径。
//!
//! 顶层 `authenticate` 按 `Credential.credential_type` 分发；遇加密私钥时
//! 通过 `AuthCtx`（来自 `prompt.rs`）向终端 tab 索取 passphrase。无 ctx
//! 的场景（SFTP / forward 后台连接）则对加密私钥直接报错。
//!
//! 默认密钥 fallback 模仿 `ssh -G` 的优先级：id_rsa → id_ecdsa → ecdsa_sk
//! → ed25519 → ed25519_sk。RSA 签名算法选择走 RFC 8308 server-sig-algs，
//! 与 OpenSSH 一致。

use std::path::PathBuf;
use std::sync::Arc;

use russh::client;
use russh::keys::agent::AgentIdentity;
use russh::keys::{Algorithm, HashAlg, PrivateKey, PrivateKeyWithHashAlg};
use serde_json::json;

use crate::error::{locked, AppError, AppResult};
use crate::models::{Credential, CredentialType};

use super::client::SshHandler;
use super::prompt::{prompt_passphrase, AuthCtx};

const MAX_PASSPHRASE_RETRIES: usize = 3;

pub(crate) fn check_auth_result(result: client::AuthResult) -> AppResult<()> {
    if result.success() {
        Ok(())
    } else {
        Err(AppError::ssh("ssh_auth_rejected", json!({})))
    }
}

/// 解析私钥，遇加密时按需向终端索取 passphrase。
///
/// `cache_key` 唯一标识这把私钥在本次进程内的 passphrase 缓存项：
/// 存储凭证用 `cred:{credential_id}`，默认密钥文件用绝对路径，临时直连凭证
/// 没缓存。`prompt_label` 是直接显示给用户的提示行（含末尾冒号空格）。
pub(crate) async fn decode_key_with_prompt(
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
        let cached: Option<zeroize::Zeroizing<String>> = {
            let state = ctx.app.state();
            locked(&state.passphrase_cache)
                .ok()
                .and_then(|m| m.get(key).cloned())
        };
        if let Some(pw) = cached {
            match russh::keys::decode_secret_key(pem, Some(pw.as_str())) {
                Ok(k) => return Ok(k),
                Err(KeyIsEncrypted) => {
                    // 缓存的 passphrase 不再匹配（用户改了密码）— 清掉再走交互
                    let state = ctx.app.state();
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
                    let state = ctx.app.state();
                    if let Ok(mut m) = locked(&state.passphrase_cache) {
                        m.insert(key.to_string(), zeroize::Zeroizing::new(pw));
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
            // credential.id 是 DB UUID，必非空；保留 Some(&cache_key) 因为
            // decode_key_with_prompt 还有 ssh-agent fallback 路径需要 Option<&str>。
            let cache_key = format!("cred:{}", credential.id);
            let prompt_label = format!(
                "Enter passphrase for key '{}': ",
                if credential.name.is_empty() {
                    credential.username.as_str()
                } else {
                    credential.name.as_str()
                }
            );
            let key = decode_key_with_prompt(pem, Some(&cache_key), &prompt_label, ctx).await?;
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
        CredentialType::Interactive => {
            // Connect 路径在调本函数前已分流到 authenticate_interactive；
            // 没分流就走到这里的全是后台路径（forward / SFTP），它们传 ctx=None
            // 也没法弹 prompt —— 必须报错，不能 silent Ok 让 caller 误以为登成功。
            // 有 ctx 时仍然委派给 authenticate_interactive，让"统一通过 authenticate()
            // 入口"成立。
            let ctx = ctx.ok_or_else(|| {
                AppError::ssh("ssh_interactive_requires_terminal", json!({}))
            })?;
            authenticate_interactive(
                handle,
                credential.username,
                ctx.app.clone(),
                ctx.tab_id.clone(),
            )
            .await
        }
    }
}

// ---------------------------------------------------------------------------
// 私钥认证（RSA 签名算法选择 + publickey 交互）
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 默认密钥（~/.ssh/id_*）fallback
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 键盘交互认证
// ---------------------------------------------------------------------------

pub async fn authenticate_interactive(
    handle: &mut client::Handle<SshHandler>,
    username: String,
    app: crate::emitter::Host,
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

                // state 拿出来一次，让 &state.auth_waiters 的借用横跨 insert + guard。
                let state = app.state();
                // 必须先注册 sender 再 emit。否则前端响应快到能在 insert 之前
                // 调 ssh_auth_respond，找不到 waiter → 响应被丢，rx 永远 hang。
                locked(&state.auth_waiters)?.insert(tab_id.clone(), tx);
                // RAII：emit 失败 / rx 异常 / 提前 return 时自动清 sender。
                // 正常 await 到响应时 sender 已被 ssh_auth_respond 取走，guard 的
                // remove 是 no-op。
                let _guard = AuthWaiterGuard {
                    waiters: &state.auth_waiters,
                    tab_id: &tab_id,
                };
                if let Err(e) = app.emit(
                    &format!("ssh:auth_prompt:{tab_id}"),
                    serde_json::json!({
                        "name": name,
                        "instructions": instructions,
                        "prompts": prompt_data,
                    }),
                ) {
                    return Err(AppError::other(
                        "emit_failed",
                        json!({ "channel": "ssh:auth_prompt", "err": e.to_string() }),
                    ));
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

/// auth_waiters 的 RAII 清理器。同 `ssh::prompt::WaiterGuard` 模式，但目标
/// map 元素类型是 `Vec<String>`（kbd-interactive 多 prompt 一次性回收）。
struct AuthWaiterGuard<'a> {
    waiters: &'a std::sync::Mutex<
        std::collections::HashMap<String, tokio::sync::oneshot::Sender<Vec<String>>>,
    >,
    tab_id: &'a str,
}

impl Drop for AuthWaiterGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut m) = locked(self.waiters) {
            m.remove(self.tab_id);
        }
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

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
