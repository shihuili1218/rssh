# Rust 代码审查报告

基于对 `src-tauri/**/*.rs` 的审查，发现以下问题：

## 1) `sftp.rs`：`download_to_path` 大小限制在流式下载中失效

**位置**：`src-tauri/src/ssh/sftp.rs`（`download_to_path`）

- 仅在下载前检查 `metadata.size`。
- 当 `metadata.size` 为 `None` 时会被当作 `0`，可绕过 `max_bytes`。
- 下载循环中未对 `transferred` 做 `max_bytes` 运行时检查，文件增长场景可能超限。

## 2) `sync/github.rs`：Header 构造使用 `unwrap()` 存在 panic 风险

**位置**：`src-tauri/src/sync/github.rs`（`headers`）

- `format!("Bearer {}", self.token).parse().unwrap()` 在 token 含非法字符时可能 panic。
- 应改为错误返回（`AppError`）而非崩溃。

## 3) `ssh/auth.rs`：passphrase 重试逻辑可能依赖了错误错误码

**位置**：`src-tauri/src/ssh/auth.rs`（`decode_key_with_prompt`）

- 当前只对 `KeyIsEncrypted` 走“密码错误重试”分支。
- 若底层库对“错误 passphrase”返回其他错误（常见为解析失败），会提前返回 `ssh_privkey_parse_failed`，重试次数形同失效。
- 还可能导致缓存中的错误 passphrase 不能正确清理。

## 4) `ssh/forward.rs`：本地/动态转发失败时静默丢连接

**位置**：`src-tauri/src/ssh/forward.rs`

- `channel_open_direct_tcpip` 失败后直接 `continue`，未反馈给客户端。
- 已 `accept` 的 TCP 客户端会收到突兀断开，排障体验差。

## 5) `ai/sanitize.rs`：hex 脱敏规则只匹配小写

**位置**：`src-tauri/src/ai/sanitize.rs`

- 规则 `\b[0-9a-f]{32,}\b` 无法覆盖大写/混合大小写十六进制串。
- 可能导致敏感 token/hash 未被脱敏并发送给 LLM。

## 6) 数据结构语义混淆：远程转发中 `remote_host` 被当作本地目标

**位置**：`src-tauri/src/ssh/forward.rs`（`start_remote`）

- `let local_host = forward.remote_host.clone();`
- 字段命名与实际语义不一致，易引发维护误用。

## 7) 设计缺陷：未知 `auth_type` 会静默降级为 `none`

**位置**：`src-tauri/src/commands/session.rs`

- `CredentialType::from_str` 对未知值返回 `None` 类型。
- 前端拼写错误不会被显式报参数错误，而是进入 SSH `none` 认证并失败，定位困难。

## 8) 设计缺陷：命令执行等待期间用户消息可能被丢弃

**位置**：`src-tauri/src/ai/session.rs`（`handle_run_command` 循环）

- 等待 `CommandResult` 时，其他 `UserAction::Message` 走 `_ => continue`，未入历史。
- 用户在命令执行中输入的信息可能被静默丢失。

---

## 建议优先级

1. **P0**：修复 `download_to_path` 的运行时大小守卫（安全与资源风险）。
2. **P0**：移除 `github.rs` 里的 header `unwrap()`（稳定性风险）。
3. **P1**：验证并修复 passphrase 重试判定逻辑（认证体验与可用性）。
4. **P1**：补齐转发失败反馈（可观测性与可维护性）。

