//! 终端内 oneshot 交互：xterm 弹 prompt → 用户输入 → 后端 await。
//!
//! 三类 prompt（passphrase / host_key / kbd-interactive）共享同一套
//! "注册 sender → emit → await rx" 模板；差异只在 waiters map / 事件名 /
//! 取消错误码 / payload。
//!
//! `AuthCtx` 是终端可达性凭证：有 ctx 才能与具体的 tab 通信。SFTP / forward
//! 等无前端的子流程用 `None`，遇加密私钥 / 未知主机时直接报错。

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::json;
use tokio::sync::oneshot;

use crate::error::{locked, AppError, AppResult};

#[derive(Clone)]
pub struct AuthCtx {
    pub app: crate::emitter::Host,
    pub tab_id: String,
}

/// RAII：rx await 走完前如果路径上失败（比如 emit 失败），guard drop 时
/// 把 sender 从 map 清掉，避免该 tab_id 永远卡在 "已存在 sender" 状态。
/// rx 正常 await 到结果时，sender 已被 commands::session::*_respond / *_cancel
/// 取走，map 里已无对应条目，guard 的 remove 是 no-op。
struct WaiterGuard<'a> {
    waiters: &'a Mutex<HashMap<String, oneshot::Sender<String>>>,
    tab_id: &'a str,
}

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut m) = locked(self.waiters) {
            m.remove(self.tab_id);
        }
    }
}

/// 通用终端 prompt：注册 oneshot sender 到指定 waiters map，emit 事件，等用户回应。
/// passphrase / host_key 等 xterm 内交互都走这条路；差异只在 waiters / 事件名 / payload。
pub(crate) async fn prompt_oneshot(
    waiters: &Mutex<HashMap<String, oneshot::Sender<String>>>,
    app: &crate::emitter::Host,
    tab_id: &str,
    event_prefix: &str,
    payload: serde_json::Value,
    cancel_code: &'static str,
) -> AppResult<String> {
    let (tx, rx) = oneshot::channel::<String>();
    {
        let mut w = locked(waiters)?;
        // dialogue 串行：上一个 prompt 不到 ok / cancel 不会发起下一个。
        // 已存在 = 上层并发设计被破坏；refuse 比 silent overwrite 更安全（旧
        // receiver 永远 hang，资源泄漏）。
        if w.contains_key(tab_id) {
            return Err(AppError::other(
                "ssh_prompt_already_pending",
                json!({ "tab_id": tab_id, "channel": event_prefix }),
            ));
        }
        w.insert(tab_id.to_string(), tx);
    }
    // guard 在 emit 失败 / rx 错误时自动从 map 清掉 sender；正常落地是 no-op。
    let _guard = WaiterGuard { waiters, tab_id };
    app.emit(&format!("{event_prefix}:{tab_id}"), payload)
        .map_err(|e| AppError::other("emit_failed", json!({ "channel": event_prefix, "err": e.to_string() })))?;
    rx.await.map_err(|_| AppError::ssh(cancel_code, json!({})))
}

/// 向终端弹一次 passphrase 提示，等用户输完回车。
pub(crate) async fn prompt_passphrase(ctx: &AuthCtx, prompt: &str) -> AppResult<String> {
    let state = ctx.app.state();
    prompt_oneshot(
        &state.passphrase_waiters,
        &ctx.app,
        &ctx.tab_id,
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
        &ctx.tab_id,
        "ssh:host_key_prompt",
        json!({ "banner": banner }),
        "ssh_user_cancelled_hostkey",
    )
    .await
}
