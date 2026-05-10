# Rust 代码审查报告（第二轮全量复审）

## 复审结论

- 已完成 `src-tauri/**/*.rs` **77 个 Rust 文件**的第二轮逐文件复审。
- 本轮未发现新的 P0/P1 高风险问题。
- 之前记录的 8 项问题经复核后仍成立，继续保留在本报告中。

## 问题清单（复核后仍成立）

### 1) `sftp.rs`：`download_to_path` 大小限制在流式下载中失效（P0）

**位置**：`src-tauri/src/ssh/sftp.rs`（`download_to_path`）

- 仅在下载前检查 `metadata.size`。
- 当 `metadata.size` 为 `None` 时会被当作 `0`，可绕过 `max_bytes`。
- 下载循环中未对 `transferred` 做 `max_bytes` 运行时检查，文件增长场景可能超限。

### 2) `sync/github.rs`：Header 构造使用 `unwrap()` 存在 panic 风险（P0）

**位置**：`src-tauri/src/sync/github.rs`（`headers`）

- `format!("Bearer {}", self.token).parse().unwrap()` 在 token 含非法字符时可能 panic。
- 其他 header 也使用了 `parse().unwrap()`，同样是可避免的崩溃点。

### 3) `ssh/auth.rs`：passphrase 重试逻辑可能依赖错误类型（P1）

**位置**：`src-tauri/src/ssh/auth.rs`（`decode_key_with_prompt`）

- 当前仅对 `KeyIsEncrypted` 进入“错误密码重试”。
- 若底层库对错误 passphrase 返回其他错误，可能提前返回 `ssh_privkey_parse_failed`，重试形同失效。
- 也可能导致缓存中的错误 passphrase 未被清理。

### 4) `ssh/forward.rs`：本地/动态转发失败时静默丢连接（P1）

**位置**：`src-tauri/src/ssh/forward.rs`

- `channel_open_direct_tcpip` 失败后直接 `continue`，未向客户端反馈。
- 已 accept 的连接会被突兀断开，定位问题困难。

### 5) `ai/sanitize.rs`：hex 脱敏规则只匹配小写（P1）

**位置**：`src-tauri/src/ai/sanitize.rs`

- 规则 `\b[0-9a-f]{32,}\b` 无法覆盖大写/混合大小写十六进制串。
- 可能导致敏感 token/hash 未被脱敏后发送给 LLM。

### 6) 数据结构语义混淆：远程转发中 `remote_host` 被当作本地目标（P2）

**位置**：`src-tauri/src/ssh/forward.rs`（`start_remote`）

- `let local_host = forward.remote_host.clone();`
- 字段命名与实际语义不一致，易引发维护误用。

### 7) 设计缺陷：未知 `auth_type` 会静默降级为 `none`（P2）

**位置**：`src-tauri/src/commands/session.rs`、`src-tauri/src/models.rs`

- `CredentialType::from_str` 对未知值返回 `None` 类型。
- 前端拼写错误不会报参数错误，而是进入 `none` 认证再失败，排障成本高。

### 8) 设计缺陷：命令执行等待期间用户消息可能被丢弃（P2）

**位置**：`src-tauri/src/ai/session.rs`（`handle_run_command`）

- 等待 `CommandResult` 期间，非匹配分支统一 `_ => continue`。
- 用户在命令执行中发送的 `UserAction::Message` 可能被静默丢弃。

---

## 本轮复审覆盖范围

### 统计

- 总文件数：77
- 目录：`src-tauri/**/*.rs`
- 覆盖方式：逐文件阅读 + 风险模式复核（认证、SFTP、转发、AI 会话、命令执行、序列化/错误处理、panic 点位）

### 文件清单

```text
src-tauri/build.rs
src-tauri/src/ai/audit.rs
src-tauri/src/ai/commands.rs
src-tauri/src/ai/llm/anthropic.rs
src-tauri/src/ai/llm/deepseek.rs
src-tauri/src/ai/llm/glm.rs
src-tauri/src/ai/llm/mod.rs
src-tauri/src/ai/llm/openai.rs
src-tauri/src/ai/llm/protocol.rs
src-tauri/src/ai/mod.rs
src-tauri/src/ai/prompts.rs
src-tauri/src/ai/sanitize.rs
src-tauri/src/ai/session.rs
src-tauri/src/ai/skills.rs
src-tauri/src/ai/tools.rs
src-tauri/src/bin/rssh/commands/add.rs
src-tauri/src/bin/rssh/commands/completions.rs
src-tauri/src/bin/rssh/commands/config.rs
src-tauri/src/bin/rssh/commands/edit.rs
src-tauri/src/bin/rssh/commands/ls.rs
src-tauri/src/bin/rssh/commands/mod.rs
src-tauri/src/bin/rssh/commands/open.rs
src-tauri/src/bin/rssh/commands/rm.rs
src-tauri/src/bin/rssh/ctx.rs
src-tauri/src/bin/rssh/helpers/cred.rs
src-tauri/src/bin/rssh/helpers/mod.rs
src-tauri/src/bin/rssh/helpers/ssh_builder.rs
src-tauri/src/bin/rssh/helpers/tui.rs
src-tauri/src/bin/rssh/main.rs
src-tauri/src/commands/cli.rs
src-tauri/src/commands/forward.rs
src-tauri/src/commands/group.rs
src-tauri/src/commands/lifecycle.rs
src-tauri/src/commands/mod.rs
src-tauri/src/commands/profile.rs
src-tauri/src/commands/pty.rs
src-tauri/src/commands/session.rs
src-tauri/src/commands/settings.rs
src-tauri/src/commands/sftp.rs
src-tauri/src/commands/sync.rs
src-tauri/src/commands/update.rs
src-tauri/src/commands/window.rs
src-tauri/src/crypto.rs
src-tauri/src/db/ai_skill.rs
src-tauri/src/db/credential.rs
src-tauri/src/db/forward.rs
src-tauri/src/db/group.rs
src-tauri/src/db/highlight.rs
src-tauri/src/db/mod.rs
src-tauri/src/db/profile.rs
src-tauri/src/db/schema.rs
src-tauri/src/db/secret.rs
src-tauri/src/db/settings.rs
src-tauri/src/db/snippet.rs
src-tauri/src/error.rs
src-tauri/src/lib.rs
src-tauri/src/main.rs
src-tauri/src/models.rs
src-tauri/src/secret/db_store.rs
src-tauri/src/secret/keyring_store.rs
src-tauri/src/secret/mod.rs
src-tauri/src/ssh/auth.rs
src-tauri/src/ssh/bastion.rs
src-tauri/src/ssh/client.rs
src-tauri/src/ssh/config.rs
src-tauri/src/ssh/forward.rs
src-tauri/src/ssh/known_hosts.rs
src-tauri/src/ssh/mod.rs
src-tauri/src/ssh/prompt.rs
src-tauri/src/ssh/sftp.rs
src-tauri/src/state.rs
src-tauri/src/sync/config.rs
src-tauri/src/sync/github.rs
src-tauri/src/sync/mod.rs
src-tauri/src/terminal/mod.rs
src-tauri/src/terminal/pty.rs
src-tauri/src/terminal/recorder.rs
```
