//! 终端内 oneshot 交互：xterm 弹 prompt → 用户输入 → 后端 await。
//!
//! 三类 prompt（passphrase / host_key / kbd-interactive）共享同一套
//! "注册 sender → emit → await rx" 模板；差异只在 waiters map / 事件名 /
//! 取消错误码 / payload。
//!
//! `AuthCtx` 是连接尝试的终端可达性凭证：有 ctx 才能通过该 attempt 的
//! `prompt_id` 通信。SFTP / forward 等无前端子流程用 `None`，遇加密私钥 /
//! 未知主机时直接报错。

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::json;
use tokio::sync::oneshot;

use crate::error::{locked, AppError, AppResult};
use crate::state::{OwnedWaiter, SessionOwner};

#[derive(Clone)]
pub struct AuthCtx {
    pub app: crate::emitter::Host,
    pub resource_id: String,
    pub prompt_id: String,
    pub owner: SessionOwner,
}

/// RAII：rx await 走完前如果路径上失败（比如 emit 失败），guard drop 时
/// 按 nonce 把 sender 从 map 清掉，避免该 prompt_id 永远卡在 "已存在 sender"
/// 状态，也避免旧 guard 删除同 ID 的新一代 waiter。
/// rx 正常 await 到结果时，sender 已被 commands::session::*_respond / *_cancel
/// 取走，map 里已无对应条目，guard 的 remove 是 no-op。
struct WaiterGuard<'a, T> {
    waiters: &'a Mutex<HashMap<String, OwnedWaiter<T>>>,
    prompt_id: &'a str,
    nonce: uuid::Uuid,
}

impl<T> Drop for WaiterGuard<'_, T> {
    fn drop(&mut self) {
        if let Ok(mut m) = locked(self.waiters) {
            if m.get(self.prompt_id)
                .is_some_and(|waiter| waiter.nonce == self.nonce)
            {
                m.remove(self.prompt_id);
            }
        }
    }
}

/// 通用终端 prompt：注册 oneshot sender 到指定 waiters map，emit 事件，等用户回应。
/// passphrase / host_key 等 xterm 内交互都走这条路；差异只在 waiters / 事件名 / payload。
pub(crate) async fn prompt_oneshot<T>(
    waiters: &Mutex<HashMap<String, OwnedWaiter<T>>>,
    app: &crate::emitter::Host,
    resource_id: &str,
    prompt_id: &str,
    owner: &SessionOwner,
    event_prefix: &str,
    payload: serde_json::Value,
    cancel_code: &'static str,
) -> AppResult<T> {
    let (tx, rx) = oneshot::channel::<T>();
    let nonce = uuid::Uuid::new_v4();
    let state = app.state();
    crate::commands::lifecycle::register_prompt_waiter(
        &state,
        waiters,
        resource_id,
        prompt_id,
        owner,
        event_prefix,
        nonce,
        tx,
    )?;
    // guard 在 emit 失败 / rx 错误时自动从 map 清掉 sender；正常落地是 no-op。
    let _guard = WaiterGuard {
        waiters,
        prompt_id,
        nonce,
    };
    app.emit(&format!("{event_prefix}:{prompt_id}"), payload)
        .map_err(|e| {
            AppError::other(
                "emit_failed",
                json!({ "channel": event_prefix, "err": e.to_string() }),
            )
        })?;
    rx.await.map_err(|_| AppError::ssh(cancel_code, json!({})))
}

/// 向终端弹一次 passphrase 提示，等用户输完回车。
pub(crate) async fn prompt_passphrase(ctx: &AuthCtx, prompt: &str) -> AppResult<String> {
    let state = ctx.app.state();
    prompt_oneshot(
        &state.passphrase_waiters,
        &ctx.app,
        &ctx.resource_id,
        &ctx.prompt_id,
        &ctx.owner,
        "ssh:passphrase_prompt",
        json!({ "prompt": prompt }),
        "ssh_user_cancelled_passphrase",
    )
    .await
}

/// 向终端弹一次主机密钥 TOFU 确认，等用户输入 yes / no / 指纹。
/// 调用方负责按返回字符串决定是否信任。
pub(crate) async fn prompt_host_key(ctx: &AuthCtx, banner: &str) -> AppResult<String> {
    let state = ctx.app.state();
    prompt_oneshot(
        &state.host_key_waiters,
        &ctx.app,
        &ctx.resource_id,
        &ctx.prompt_id,
        &ctx.owner,
        "ssh:host_key_prompt",
        json!({ "banner": banner }),
        "ssh_user_cancelled_hostkey",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_waiter_guard_does_not_remove_replacement() {
        let waiters = Mutex::new(HashMap::new());
        let owner = SessionOwner::Window("main".into());
        let stale_nonce = uuid::Uuid::new_v4();
        let (stale_tx, _stale_rx) = oneshot::channel::<String>();
        waiters.lock().unwrap().insert(
            "attempt".into(),
            OwnedWaiter {
                nonce: stale_nonce,
                owner: owner.clone(),
                sender: stale_tx,
            },
        );
        let guard = WaiterGuard {
            waiters: &waiters,
            prompt_id: "attempt",
            nonce: stale_nonce,
        };

        let replacement_nonce = uuid::Uuid::new_v4();
        let (replacement_tx, _replacement_rx) = oneshot::channel::<String>();
        waiters.lock().unwrap().insert(
            "attempt".into(),
            OwnedWaiter {
                nonce: replacement_nonce,
                owner,
                sender: replacement_tx,
            },
        );
        drop(guard);

        assert_eq!(
            waiters.lock().unwrap().get("attempt").unwrap().nonce,
            replacement_nonce
        );
    }
}
