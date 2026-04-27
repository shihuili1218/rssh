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
    /// catalog system prompt（含通用规则 + skill 目录），启动前由 commands 层构造
    pub system_prompt: String,
    /// 启动时一次性 load 全部 skill（含 content）作为 load_skill 工具的来源；
    /// 会话期间不重新读 DB，避免用户中途改 skill 影响当前会话
    pub skills_cache: Vec<SkillRecord>,
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
                    self.history.push(ChatMessage::User { content: text.clone() });
                    self.emit("user_message", json!({ "text": text }));
                    if let Err(e) = self.dialogue_turn().await {
                        self.audit_push(AuditKind::Error { message: e.to_string() });
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
                self.push_tool_error(&tc.id, &format!("未知工具: {other}"));
                Ok(())
            }
        }
    }

    async fn handle_load_skill(&mut self, tc: ToolCall) -> AppResult<()> {
        let input: LoadSkillInput = match serde_json::from_value(tc.input.clone()) {
            Ok(i) => i,
            Err(e) => {
                self.push_tool_error(&tc.id, &format!("input 解析失败: {e}"));
                return Ok(());
            }
        };
        let skill = match self.cfg.skills_cache.iter().find(|s| s.id == input.id) {
            Some(s) => s.clone(),
            None => {
                let known: Vec<&str> = self.cfg.skills_cache.iter().map(|s| s.id.as_str()).collect();
                self.push_tool_error(
                    &tc.id,
                    &format!(
                        "未知 skill id: {}。可用: {}",
                        input.id,
                        known.join(", ")
                    ),
                );
                return Ok(());
            }
        };
        self.audit_push(AuditKind::Note {
            message: format!("loaded skill: {} ({})", skill.id, skill.name),
        });
        self.history.push(ChatMessage::ToolResult {
            tool_call_id: tc.id,
            content: format!(
                "# {} (id: {})\n\n{}\n\n---\n\n{}",
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
                self.push_tool_error(&tc.id, &format!("input 解析失败: {e}"));
                return Ok(());
            }
        };

        // 本地 shell target 没必要 SFTP——文件已经在用户本机
        let ssh_handle = match self.cfg.ssh_handle.as_ref() {
            Some(h) => h.clone(),
            None => {
                self.push_tool_error(
                    &tc.id,
                    "当前会话目标是本地 shell，不需要 SFTP。直接把路径告诉用户即可。",
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
                .map_err(|e| AppError::Other(format!("创建本地目录失败: {e}")))?;
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
                        "下载完成：{} ({} 字节)。文件已落到用户本机；请告诉用户路径，让他用本地工具自行分析。",
                        local_str, bytes
                    ),
                    is_error: false,
                });
            }
            Err(e) => {
                self.audit_push(AuditKind::Note {
                    message: format!("download_file 失败: {e}"),
                });
                self.push_tool_error(
                    &tc.id,
                    &format!(
                        "rssh 无法通过 SFTP 直连目标拉文件（{e}）。常见原因：用户是经跳板手动 ssh 进去的，连通路径未直达。\
                         请告诉用户：用 scp / rsync / sz 等手段把 {} 自行拉到本机，分析完后把关键结果贴回对话。",
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
                self.push_tool_error(&tc.id, &format!("input 解析失败: {e}"));
                return Ok(());
            }
        };

        // 文件必须真存在——LLM 应该先 download_file
        if !std::path::Path::new(&input.local_path).exists() {
            self.push_tool_error(
                &tc.id,
                &format!(
                    "本地路径不存在：{}。先用 download_file 把文件拉到本机。",
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

        use tauri::{WebviewUrl, WebviewWindowBuilder};
        WebviewWindowBuilder::new(&self.app, &label, WebviewUrl::App("index.html".into()))
            .title("RSSH — 本地分析")
            .inner_size(1200.0, 800.0)
            .initialization_script(&init_script)
            .build()
            .map_err(|e| AppError::Other(format!("打开新窗口失败: {e}")))?;

        self.audit_push(AuditKind::Note {
            message: format!(
                "analyze_locally: spawn 新窗口分析 {} (task: {})",
                input.local_path, input.task
            ),
        });

        self.history.push(ChatMessage::ToolResult {
            tool_call_id: tc.id,
            content: format!(
                "已开新窗口启动独立 AI 会话分析 {}（任务：{}）。\
                 本会话不会收到分析结果，继续推进当前的远端排障即可；\
                 用户在新窗口看到结果后会自行决定怎么把结论带回这边。",
                input.local_path, input.task
            ),
            is_error: false,
        });
        Ok(())
    }

    async fn handle_run_command(&mut self, tc: ToolCall) -> AppResult<()> {
        let input: RunCommandInput = match serde_json::from_value(tc.input.clone()) {
            Ok(i) => i,
            Err(e) => {
                self.push_tool_error(&tc.id, &format!("input 解析失败: {e}"));
                return Ok(());
            }
        };

        if let Err(e) = sanitize::validate(&input.cmd) {
            self.push_tool_error(
                &tc.id,
                &format!("rssh 拒绝该命令：{e}。换一条符合规则的重提。"),
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
                None => return Err(AppError::Other("会话 channel 关闭".into())),
            };
            match action {
                UserAction::RejectCommand { tool_call_id, reason } if tool_call_id == tc.id => {
                    self.audit_push(AuditKind::CommandRejected {
                        id: cmd_id.clone(),
                        reason: reason.clone(),
                    });
                    self.push_tool_error(
                        &tc.id,
                        &format!("用户拒绝该命令。理由: {reason}。根据这个理由调整方案。"),
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
                UserAction::Stop => return Err(AppError::Other("会话已停止".into())),
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
