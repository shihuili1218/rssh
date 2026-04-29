//! AI 排障会话 actor。
//!
//! 重设计（2026-04-26）：命令不在后端执行。后端职责：
//! 1. 收 LLM 工具调用 → 生成 sentinel uuid → emit 给前端
//! 2. 前端把 `cmd; echo "<sentinel>:$?"` 粘到 active terminal 自动回车
//! 3. 前端监听 PTY 数据流找 sentinel → 提取 output + exit code → invoke ai_command_result
//! 4. 后端收 result → 脱敏 + 截断 + 入审计 + 作为 tool_result 推进 LLM 对话
//!
//! 这样命令在用户的交互终端里完整可见，没有任何后端注入或 byte 监控。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::ssh::client::SshHandle;
use crate::ssh::sftp::SftpHandle;

use super::audit::{AuditKind, AuditLog};
use super::llm::{ChatDelta, ChatMessage, ChatRequest, DeltaSink, LlmClient, ToolCall};
use super::sanitize::{self, RedactRule};
use super::skills::SkillRecord;
use super::tools::{self, AnalyzeLocallyInput, DownloadFileInput, LoadSkillInput, RunCommandInput};

#[derive(Debug)]
pub enum UserAction {
    Message(String),
    RejectCommand {
        tool_call_id: String,
        reason: String,
    },
    /// 前端把命令在终端里跑完后回报结果。output 是脱敏前的原始文本。
    CommandResult {
        tool_call_id: String,
        exit_code: i32,
        output: String,
        timed_out: bool,
    },
    Stop,
}

pub struct DiagnoseSession {
    pub session_id: String,
    pub target_id: String,
    pub skill: String,
    pub model: String,
    pub provider: String,
    pub action_tx: mpsc::UnboundedSender<UserAction>,
    pub audit: Arc<Mutex<AuditLog>>,
}

pub struct SessionConfig {
    pub session_id: String,
    pub target_id: String,
    pub skill: String,
    /// system prompt：内置 general 规则集 + user-skill 目录（id + description），
    /// 启动前由 commands 层构造。user-skill 详细内容走 `load_skill` 工具按需加载。
    pub system_prompt: String,
    /// 启动时一次性 snapshot 的 user-skill（仅自定义，不含 builtin general）；
    /// `load_skill` 工具从这里查内容，会话期间不重读 DB，避免用户中途改 skill 影响当前会话。
    pub user_skills_cache: Vec<SkillRecord>,
    pub model: String,
    pub client: Box<dyn LlmClient>,
    pub redact_rules: Vec<RedactRule>,
    pub max_output_bytes: usize,
    /// SSH target 的连接 handle（本地 PTY target 为 None）。
    /// download_file 工具复用这个 handle 起 SFTP 子系统。
    pub ssh_handle: Option<SshHandle>,
    /// dump 文件落地目录（实际文件写到 <data_dir>/diagnose/<session_id>/）。
    pub data_dir: PathBuf,
}

pub fn start(cfg: SessionConfig, app: AppHandle) -> AppResult<DiagnoseSession> {
    let system_prompt = cfg.system_prompt.clone();

    let (action_tx, action_rx) = mpsc::unbounded_channel();
    let audit = Arc::new(Mutex::new(AuditLog::default()));
    if let Ok(mut g) = audit.lock() {
        g.push(AuditKind::SessionStarted {
            skill: cfg.skill.clone(),
            target: cfg.target_id.clone(),
        });
    }

    let provider = cfg.client.provider().to_string();
    let session = DiagnoseSession {
        session_id: cfg.session_id.clone(),
        target_id: cfg.target_id.clone(),
        skill: cfg.skill.clone(),
        model: cfg.model.clone(),
        provider,
        action_tx,
        audit: audit.clone(),
    };

    let actor = Actor {
        cfg,
        system_prompt,
        history: Vec::new(),
        action_rx,
        audit,
        app,
    };
    tauri::async_runtime::spawn(actor.run());

    Ok(session)
}

struct Actor {
    cfg: SessionConfig,
    system_prompt: String,
    history: Vec<ChatMessage>,
    action_rx: mpsc::UnboundedReceiver<UserAction>,
    audit: Arc<Mutex<AuditLog>>,
    app: AppHandle,
}

impl Actor {
    async fn run(mut self) {
        loop {
            let action = match self.action_rx.recv().await {
                Some(a) => a,
                None => break,
            };
            match action {
                UserAction::Stop => break,
                UserAction::Message(text) => {
                    self.history.push(ChatMessage::User {
                        content: text.clone(),
                    });
                    self.emit("user_message", json!({ "text": text }));
                    if let Err(e) = self.dialogue_turn().await {
                        self.audit_push(AuditKind::Error {
                            message: e.to_string(),
                        });
                        self.emit("error", json!({ "message": e.to_string() }));
                    }
                }
                _ => {
                    log::warn!("unexpected action outside command dialog");
                }
            }
        }
        self.audit_push(AuditKind::SessionEnded);
        self.emit("session_ended", json!({}));
    }

    async fn dialogue_turn(&mut self) -> AppResult<()> {
        loop {
            let req = ChatRequest {
                system_prompt: self.system_prompt.clone(),
                messages: self.history.clone(),
                tools: tools::all_tools(),
                model: self.cfg.model.clone(),
                max_tokens: 4096,
            };

            let payload_text = serde_json::to_string_pretty(&self.history)
                .unwrap_or_else(|_| "<unserializable>".into());
            let redacted = sanitize::redact(&payload_text, &self.cfg.redact_rules);
            self.audit_push(AuditKind::LlmRequest {
                model: self.cfg.model.clone(),
                redacted_payload: redacted,
            });

            // 流式：先 emit start 给前端开一条空 streaming bubble；
            // delta 来了 emit assistant_delta；chat 返回后 emit end 把最终文本给前端结清。
            let msg_id = uuid::Uuid::new_v4().to_string();
            self.emit("assistant_message_start", json!({ "id": msg_id }));

            let app = self.app.clone();
            let session_id = self.cfg.session_id.clone();
            let sink_msg_id = msg_id.clone();
            let sink: DeltaSink = std::sync::Arc::new(move |delta| {
                if let ChatDelta::Text(t) = delta {
                    let _ = app.emit(
                        &format!("ai:assistant_delta:{session_id}"),
                        json!({ "id": sink_msg_id, "text": t }),
                    );
                }
            });

            let resp = self.cfg.client.chat(req, sink).await?;

            self.emit(
                "assistant_message_end",
                json!({ "id": msg_id, "text": resp.text }),
            );

            self.audit_push(AuditKind::LlmResponse {
                text: resp.text.clone(),
                tokens_in: resp.tokens_in,
                tokens_out: resp.tokens_out,
            });

            self.history.push(ChatMessage::Assistant {
                content: resp.text.clone(),
                tool_calls: resp.tool_calls.clone(),
            });

            if resp.tool_calls.is_empty() {
                return Ok(());
            }

            for tc in resp.tool_calls {
                self.handle_tool_call(tc).await?;
            }
        }
    }

    async fn handle_tool_call(&mut self, tc: ToolCall) -> AppResult<()> {
        match tc.name.as_str() {
            tools::TOOL_RUN_COMMAND => self.handle_run_command(tc).await,
            tools::TOOL_LOAD_SKILL => self.handle_load_skill(tc).await,
            tools::TOOL_DOWNLOAD_FILE => self.handle_download_file(tc).await,
            tools::TOOL_ANALYZE_LOCALLY => self.handle_analyze_locally(tc).await,
            other => {
                self.push_tool_error(&tc.id, &format!("Unknown tool: {other}"));
                Ok(())
            }
        }
    }

    async fn handle_load_skill(&mut self, tc: ToolCall) -> AppResult<()> {
        let input: LoadSkillInput = match serde_json::from_value(tc.input.clone()) {
            Ok(i) => i,
            Err(e) => {
                self.push_tool_error(&tc.id, &format!("Failed to parse input: {e}"));
                return Ok(());
            }
        };
        // 'general' 是内置 builtin，已经直接在 system prompt 里——不可被 load
        if input.id == "general" {
            self.push_tool_error(
                &tc.id,
                "'general' is the built-in rule set and is already in the system prompt — no need to load it.",
            );
            return Ok(());
        }
        let skill = match self.cfg.user_skills_cache.iter().find(|s| s.id == input.id) {
            Some(s) => s.clone(),
            None => {
                let known: Vec<&str> = self
                    .cfg
                    .user_skills_cache
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect();
                self.push_tool_error(
                    &tc.id,
                    &format!(
                        "Unknown user-skill id: {}. Available user skills: [{}]",
                        input.id,
                        known.join(", ")
                    ),
                );
                return Ok(());
            }
        };
        self.audit_push(AuditKind::Note {
            message: format!("loaded user-skill: {} ({})", skill.id, skill.name),
        });
        self.history.push(ChatMessage::ToolResult {
            tool_call_id: tc.id,
            content: format!(
                "# {} (id: {})\n\n_{}_\n\n---\n\n{}",
                skill.name, skill.id, skill.description, skill.content
            ),
            is_error: false,
        });
        Ok(())
    }

    async fn handle_download_file(&mut self, tc: ToolCall) -> AppResult<()> {
        let input: DownloadFileInput = match serde_json::from_value(tc.input.clone()) {
            Ok(i) => i,
            Err(e) => {
                self.push_tool_error(&tc.id, &format!("Failed to parse input: {e}"));
                return Ok(());
            }
        };

        // 本地 shell target 没必要 SFTP——文件已经在用户本机
        let ssh_handle = match self.cfg.ssh_handle.as_ref() {
            Some(h) => h.clone(),
            None => {
                self.push_tool_error(
                    &tc.id,
                    "This session's target is a local shell, so SFTP isn't needed. Just tell the user the path.",
                );
                return Ok(());
            }
        };

        let dl_id = uuid::Uuid::new_v4().to_string();
        self.audit_push(AuditKind::DownloadProposed {
            id: dl_id.clone(),
            remote_path: input.remote_path.clone(),
            max_mb: input.max_mb,
        });
        self.emit(
            "download_started",
            json!({
                "id": dl_id,
                "remote_path": input.remote_path,
                "max_mb": input.max_mb,
            }),
        );

        let basename = std::path::Path::new(&input.remote_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("dump-{}", &dl_id[..8]));
        let local_dir = self
            .cfg
            .data_dir
            .join("diagnose")
            .join(&self.cfg.session_id);
        let local_path = local_dir.join(&basename);
        let max_bytes = (input.max_mb as u64).saturating_mul(1024 * 1024);

        let result: AppResult<u64> = async {
            tokio::fs::create_dir_all(&local_dir)
                .await
                .map_err(|e| AppError::Other(format!("Failed to create local directory: {e}")))?;
            let sftp = SftpHandle::from_handle(&ssh_handle).await?;
            sftp.download_to_path(&input.remote_path, &local_path, max_bytes)
                .await
        }
        .await;

        match result {
            Ok(bytes) => {
                let local_str = local_path.to_string_lossy().into_owned();
                self.audit_push(AuditKind::DownloadCompleted {
                    id: dl_id.clone(),
                    local_path: local_str.clone(),
                    bytes,
                });
                self.emit(
                    "download_completed",
                    json!({
                        "id": dl_id,
                        "local_path": local_str,
                        "bytes": bytes,
                    }),
                );
                self.history.push(ChatMessage::ToolResult {
                    tool_call_id: tc.id,
                    content: format!(
                        "Download complete: {} ({} bytes). The file is now on the user's machine; tell the user the path and let them analyze it with local tools.",
                        local_str, bytes
                    ),
                    is_error: false,
                });
            }
            Err(e) => {
                self.audit_push(AuditKind::Note {
                    message: format!("download_file failed: {e}"),
                });
                self.push_tool_error(
                    &tc.id,
                    &format!(
                        "rssh cannot SFTP directly to the target ({e}). Common cause: the user manually ssh'd through a bastion, so the connection path isn't direct. \
                         Please tell the user to use scp / rsync / sz to pull {} to their local machine themselves, then paste the key analysis output back into the chat.",
                        input.remote_path
                    ),
                );
            }
        }
        Ok(())
    }

    async fn handle_analyze_locally(&mut self, tc: ToolCall) -> AppResult<()> {
        let input: AnalyzeLocallyInput = match serde_json::from_value(tc.input.clone()) {
            Ok(i) => i,
            Err(e) => {
                self.push_tool_error(&tc.id, &format!("Failed to parse input: {e}"));
                return Ok(());
            }
        };

        // 文件必须真存在——LLM 应该先 download_file
        if !std::path::Path::new(&input.local_path).exists() {
            self.push_tool_error(
                &tc.id,
                &format!(
                    "Local path does not exist: {}. Use download_file first to pull the file to the local machine.",
                    input.local_path
                ),
            );
            return Ok(());
        }

        // 把 handoff 注入到新窗口的 window.__rssh_ai_handoff；
        // 新窗口的 AppShell 在 onMount 里读它 → 建本地 shell tab → 启动独立 AI 会话 → 发首条消息。
        let handoff = json!({
            "local_path": input.local_path,
            "task": input.task,
        })
        .to_string();
        let json_literal = serde_json::to_string(&handoff)
            .map_err(|e| AppError::Other(format!("encode handoff: {e}")))?;
        // 直接把 JSON 字符串赋值为 JS string；前端走 JSON.parse(data) 还原。
        // 不要在这里 JSON.parse —— 否则 window.__rssh_ai_handoff 已经是 object，
        // 前端再 JSON.parse 会撞 "[object Object]" 解析失败。
        let init_script = format!("window.__rssh_ai_handoff = {};", json_literal);
        let label = format!("rssh-ai-{}", uuid::Uuid::new_v4().simple());

        // Tauri 2 把 .title()/.inner_size() 等窗口方法限定在 #[cfg(desktop)]，
        // 移动端不存在。analyze_locally 的本质就是开新窗口，移动端语义上不存在，
        // 直接告知 LLM 工具不可用。
        #[cfg(desktop)]
        {
            use tauri::{WebviewUrl, WebviewWindowBuilder};
            WebviewWindowBuilder::new(&self.app, &label, WebviewUrl::App("index.html".into()))
                .title("RSSH — Local Analysis")
                .inner_size(1200.0, 800.0)
                .initialization_script(&init_script)
                .build()
                .map_err(|e| AppError::Other(format!("Failed to open new window: {e}")))?;

            self.audit_push(AuditKind::Note {
                message: format!(
                    "analyze_locally: spawned new window for {} (task: {})",
                    input.local_path, input.task
                ),
            });

            self.history.push(ChatMessage::ToolResult {
                tool_call_id: tc.id,
                content: format!(
                    "Opened a new window with a separate AI session to analyze {} (task: {}). \
                     This session will NOT receive the analysis result — continue with the current remote diagnosis. \
                     Once the user has the result in the new window, they'll decide how to bring the conclusion back here.",
                    input.local_path, input.task
                ),
                is_error: false,
            });
        }

        #[cfg(mobile)]
        {
            let _ = (init_script, label);
            self.push_tool_error(
                &tc.id,
                "analyze_locally is desktop-only: this build cannot spawn additional windows. Continue diagnosis in the current session.",
            );
        }

        Ok(())
    }

    async fn handle_run_command(&mut self, tc: ToolCall) -> AppResult<()> {
        let input: RunCommandInput = match serde_json::from_value(tc.input.clone()) {
            Ok(i) => i,
            Err(e) => {
                self.push_tool_error(&tc.id, &format!("Failed to parse input: {e}"));
                return Ok(());
            }
        };

        if let Err(e) = sanitize::validate(&input.cmd) {
            self.push_tool_error(
                &tc.id,
                &format!("rssh refused the command: {e}. Try a compliant rewrite."),
            );
            return Ok(());
        }

        let cmd_id = uuid::Uuid::new_v4().to_string();
        let sentinel = format!("__rssh_done_{}", uuid::Uuid::new_v4().simple());
        let timeout_s = input.timeout_s.unwrap_or(60).clamp(1, 300);
        // 前端实际粘贴这个完整命令（含 sentinel + exit code 回显）
        let full_cmd = format!("{}; echo \"{}:$?\"", input.cmd, sentinel);

        self.audit_push(AuditKind::CommandProposed {
            id: cmd_id.clone(),
            cmd: input.cmd.clone(),
            explain: input.explain.clone(),
            side_effect: input.side_effect.clone(),
        });

        self.emit(
            "command_proposed",
            json!({
                "id": cmd_id,
                "tool_call_id": tc.id,
                "cmd": input.cmd,
                "full_cmd": full_cmd,
                "sentinel": sentinel,
                "explain": input.explain,
                "side_effect": input.side_effect,
                "timeout_s": timeout_s,
            }),
        );

        loop {
            let action = match self.action_rx.recv().await {
                Some(a) => a,
                None => return Err(AppError::Other("Session channel closed".into())),
            };
            match action {
                UserAction::RejectCommand {
                    tool_call_id,
                    reason,
                } if tool_call_id == tc.id => {
                    self.audit_push(AuditKind::CommandRejected {
                        id: cmd_id.clone(),
                        reason: reason.clone(),
                    });
                    self.push_tool_error(
                        &tc.id,
                        &format!("User rejected the command. Reason: {reason}. Adjust your plan based on this reason."),
                    );
                    return Ok(());
                }
                UserAction::CommandResult {
                    tool_call_id,
                    exit_code,
                    output,
                    timed_out,
                } if tool_call_id == tc.id => {
                    let redacted = sanitize::redact(&output, &self.cfg.redact_rules);
                    let trunc = sanitize::truncate(&redacted, self.cfg.max_output_bytes);

                    self.emit(
                        "command_completed",
                        json!({
                            "id": cmd_id,
                            "exit_code": exit_code,
                            "timed_out": timed_out,
                            "output": trunc.text,
                            "original_bytes": trunc.original_bytes,
                            "truncated_bytes": trunc.truncated_bytes,
                        }),
                    );

                    self.audit_push(AuditKind::CommandExecuted {
                        id: cmd_id,
                        exit_code,
                        output_redacted: trunc.text.clone(),
                        original_bytes: trunc.original_bytes,
                        truncated_bytes: trunc.truncated_bytes,
                        duration_ms: 0,
                    });

                    let tool_payload = format!(
                        "exit={exit_code} timed_out={timed_out}\n--- output ---\n{}",
                        trunc.text
                    );
                    self.history.push(ChatMessage::ToolResult {
                        tool_call_id: tc.id,
                        content: tool_payload,
                        is_error: timed_out || exit_code != 0,
                    });
                    return Ok(());
                }
                UserAction::Stop => return Err(AppError::Other("Session stopped".into())),
                _ => continue,
            }
        }
    }

    fn push_tool_error(&mut self, tool_call_id: &str, msg: &str) {
        self.history.push(ChatMessage::ToolResult {
            tool_call_id: tool_call_id.to_string(),
            content: msg.to_string(),
            is_error: true,
        });
        self.audit_push(AuditKind::Note {
            message: format!("[tool_error {tool_call_id}] {msg}"),
        });
    }

    fn audit_push(&self, kind: AuditKind) {
        if let Ok(mut g) = self.audit.lock() {
            g.push(kind);
        }
    }

    fn emit(&self, kind: &str, payload: serde_json::Value) {
        let event = format!("ai:{kind}:{}", self.cfg.session_id);
        let _ = self.app.emit(&event, payload);
    }
}
