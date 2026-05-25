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
use tokio::sync::{mpsc, Notify};

use crate::error::{AppError, AppResult};
use crate::ssh::client::SshHandle;
use crate::ssh::sftp::SftpHandle;

use super::audit::{AuditKind, AuditLog};
use super::llm::{ChatDelta, ChatMessage, ChatRequest, DeltaSink, LlmClient, ToolCall};
use super::sanitize::{self, RedactRule};
use super::skills::SkillRecord;
use super::tools::{
    self, AnalyzeLocallyInput, DownloadFileInput, LoadSkillInput, RunCommandInput,
};

mod file_ops;

use file_ops::RemoteCapabilities;


/// 工具命令在前端 PTY 跑完后的两种结果。file_ops 子模块的 `run_file_op` 也用它。
#[derive(Debug)]
pub(in crate::ai::session) enum CommandOutcome {
    Result {
        exit_code: i32,
        output: String,
        #[allow(dead_code)]
        timed_out: bool,
        #[allow(dead_code)]
        early_terminated: bool,
    },
    Rejected {
        reason: String,
    },
}

/// SFTP 下载硬上限。LLM 不可信，rssh 在边界 enforce：超过 100MB 一律不走 SFTP。
/// 用户要分析更大的 artifact（GB-scale heap dump 等），人工 scp/rsync 拉到本地后
/// 直接调用 analyze_locally。理由：(1) 单条 SSH 上的 SFTP 不是搬 GB 文件的合适通道；
/// (2) AI 静默把巨型文件拉过来对用户是 hostile —— 让人显式动手。
const MAX_DOWNLOAD_MB: u32 = 100;

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
        /// 用户在执行中点了"提前终止"（前端发了 Ctrl+C）。
        early_terminated: bool,
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
    /// 流式响应的取消句柄。actor 在 chat() 前把 Notify 装进 slot，chat 完成/取消后清空。
    /// commands 层从 slot 取 Notify 调 notify_one() —— 没在 chat 时 slot 为 None，发了也无副作用。
    /// 这样 cancel 永远只能取消"当前正在进行的 chat"，不会污染后续轮次。
    pub cancel_slot: Arc<Mutex<Option<Arc<Notify>>>>,
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
    // system_prompt 是静态文本（rules + user-skill catalog + locale + 平台），
    // 整段不含运行期数据 —— 启动期一次性脱敏并缓存，避免每个 dialogue turn
    // 重跑一遍 regex。redact_rules 在会话生命周期内不变，所以安全。
    let system_prompt = sanitize::redact(&cfg.system_prompt, &cfg.redact_rules);

    let (action_tx, action_rx) = mpsc::unbounded_channel();
    let audit = Arc::new(Mutex::new(AuditLog::default()));
    if let Ok(mut g) = audit.lock() {
        g.push(AuditKind::SessionStarted {
            skill: cfg.skill.clone(),
            target: cfg.target_id.clone(),
        });
    }

    let cancel_slot: Arc<Mutex<Option<Arc<Notify>>>> = Arc::new(Mutex::new(None));

    let provider = cfg.client.provider().to_string();
    let session = DiagnoseSession {
        session_id: cfg.session_id.clone(),
        target_id: cfg.target_id.clone(),
        skill: cfg.skill.clone(),
        model: cfg.model.clone(),
        provider,
        action_tx,
        audit: audit.clone(),
        cancel_slot: cancel_slot.clone(),
    };

    let actor = Actor {
        cfg,
        system_prompt,
        history: Vec::new(),
        action_rx,
        audit,
        app,
        cancel_slot,
        remote_caps: None,
    };
    tauri::async_runtime::spawn(actor.run());

    Ok(session)
}

pub(in crate::ai::session) struct Actor {
    /// pub(in crate::ai::session) 给 `file_ops` 子模块的 `impl Actor` 访问 cfg.redact_rules / cfg.max_output_bytes
    pub(in crate::ai::session) cfg: SessionConfig,
    system_prompt: String,
    /// pub(in crate::ai::session) 给 file_ops handlers push ToolResult
    pub(in crate::ai::session) history: Vec<ChatMessage>,
    action_rx: mpsc::UnboundedReceiver<UserAction>,
    audit: Arc<Mutex<AuditLog>>,
    app: AppHandle,
    cancel_slot: Arc<Mutex<Option<Arc<Notify>>>>,
    /// 远端 file_ops 能力 — lazy 探测，session 内缓存。
    /// None = 还没探测；Some = 已探测，结果有效到 session 结束。
    /// pub(in crate::ai::session) 给 `file_ops::Actor::ensure_remote_caps` 读写。
    pub(in crate::ai::session) remote_caps: Option<RemoteCapabilities>,
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
            // 脱敏在 LLM 边界统一发生。原文留在 self.history（永不离开本机），
            // 副本送 LLM 也送 audit —— LLM 看到的就是 audit 记录的，一致。
            // system_prompt 在 start() 已经脱敏过，循环里直接复用。
            // ToolResult 在 push 时已 redact 过一次（handle_run_command），这里
            // 再过一遍是 idempotent，没成本。User/Assistant.content 此前从未脱敏。
            let rules = &self.cfg.redact_rules;
            let redacted_history: Vec<ChatMessage> = self
                .history
                .iter()
                .map(|m| sanitize::redact_message(m, rules))
                .collect();

            let req = ChatRequest {
                system_prompt: self.system_prompt.clone(),
                messages: redacted_history.clone(),
                tools: tools::all_tools(),
                model: self.cfg.model.clone(),
                max_tokens: 4096,
            };

            let payload_text = serde_json::to_string_pretty(&redacted_history)
                .unwrap_or_else(|_| "<unserializable>".into());
            self.audit_push(AuditKind::LlmRequest {
                model: self.cfg.model.clone(),
                redacted_payload: payload_text,
            });

            // 流式：先 emit start 给前端开一条空 streaming bubble；
            // delta 来了 emit assistant_delta；chat 返回后 emit end 把最终文本给前端结清。
            let msg_id = uuid::Uuid::new_v4().to_string();

            let app = self.app.clone();
            let session_id = self.cfg.session_id.clone();
            let sink_msg_id = msg_id.clone();
            // captured：sink 边 emit 边累积文本副本。取消时 chat() future 被 drop，
            // 内部的 text_out 跟着没了，但 captured 还在——拿它写一条 partial assistant
            // 进 history，否则下次发消息时 LLM 看到 [user, user] 序列会报 400（Anthropic 严格）。
            let captured: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
            let captured_for_sink = captured.clone();
            let sink: DeltaSink = std::sync::Arc::new(move |delta| {
                if let ChatDelta::Text(t) = delta {
                    if let Ok(mut g) = captured_for_sink.lock() {
                        g.push_str(&t);
                    }
                    let _ = app.emit(
                        &format!("ai:assistant_delta:{session_id}"),
                        json!({ "id": sink_msg_id, "text": t }),
                    );
                }
            });

            // 装上 cancel notifier：commands 层 ai_cancel_stream 会 notify_one 它。
            // chat 完成或取消后从 slot 摘下——slot 为 None 时 cancel 是 no-op，
            // 不会污染下一轮 chat。
            //
            // **顺序关键**：先装 slot 再 emit start。否则 UI 一看到 start 就显示 Stop
            // 按钮，用户立刻点击进 ai_cancel_stream 时 slot 还是 None，cancel 成 no-op，
            // 第一次按等于没按。装好 slot 后再吹响号 → UI 看到 Stop 按钮时 cancel
            // handle 必定就位，第一次按就生效。
            let cancel = Arc::new(Notify::new());
            if let Ok(mut g) = self.cancel_slot.lock() {
                *g = Some(cancel.clone());
            }

            self.emit("assistant_message_start", json!({ "id": msg_id }));

            let chat_future = self.cfg.client.chat(req, sink);
            let chat_result = tokio::select! {
                r = chat_future => Some(r),
                _ = cancel.notified() => None,
            };

            if let Ok(mut g) = self.cancel_slot.lock() {
                *g = None;
            }

            let resp = match chat_result {
                Some(Ok(r)) => r,
                Some(Err(e)) => {
                    // chat 失败也必须把 assistant_message_end 发出去——start/end 是前端
                    // 那条 streaming 气泡的开/关闸门。漏掉 end，前端 isStreaming() 永远
                    // 卡 true，"停止"按钮收不回，textarea 一直 disabled。
                    // text 空 → store 监听器会移除整条气泡（见 store.svelte.ts），
                    // 让 UI 干净；error 通过 dialogue_turn 上抛后 run() 会再 emit
                    // "ai:error" 把错误信息独立展示在 banner。
                    self.emit("assistant_message_end", json!({ "id": msg_id, "text": "" }));
                    // history 也要补一条 assistant 占位，否则下次用户发消息时序列变
                    // [..., user, user]，Anthropic 严格 provider 会 400。
                    // 内容用通用 marker（不放 e.to_string()）—— LLM 不需要看到真实
                    // error 字符串（可能含 endpoint/header/key 等内部细节），banner
                    // 给用户看的是真实 error。
                    self.history.push(ChatMessage::Assistant {
                        content: "[response failed]".to_string(),
                        tool_calls: vec![],
                        reasoning_content: None,
                    });
                    return Err(e);
                }
                None => {
                    // 用户取消：chat future 已 drop，TCP 流随之断开。
                    //
                    // 数据流分叉是刻意的——两个消费者诉求不一样：
                    // - UI（emit）：拿 partial 原文 + cancelled=true flag，前端用 i18n
                    //   渲染本地化的"已停止"徽章，避免把英文 marker 硬塞进用户视野。
                    // - LLM（history）：写带英文 marker 的字符串，提示模型"前面这条
                    //   被打断"，下轮别假定其有效。LLM 看的是后端 system prompt 风格
                    //   （英文），marker 跟着英文走更自然。
                    let partial = captured.lock().map(|g| g.clone()).unwrap_or_default();
                    self.emit(
                        "assistant_message_end",
                        json!({ "id": msg_id, "text": partial, "cancelled": true }),
                    );
                    self.audit_push(AuditKind::Note {
                        message: format!(
                            "user cancelled streaming response (partial {} bytes)",
                            partial.len()
                        ),
                    });
                    // history 的 assistant content 不能空——空字符串某些 provider 会拒。
                    let history_content = if partial.is_empty() {
                        "[response stopped by user]".to_string()
                    } else {
                        format!("{partial}\n\n[response stopped by user]")
                    };
                    self.history.push(ChatMessage::Assistant {
                        content: history_content,
                        tool_calls: vec![],
                        reasoning_content: None,
                    });
                    return Ok(());
                }
            };

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
                reasoning_content: resp.reasoning_content.clone(),
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
            tools::TOOL_MATCH_FILE => self.handle_match_file(tc).await,
            tools::TOOL_PATCH_FILE => self.handle_patch_file(tc).await,
            other => {
                self.push_tool_error(&tc.id, &format!("Unknown tool: {other}"));
                Ok(())
            }
        }
    }

    /// 等待前端汇报命令结果或拒绝。
    ///
    /// 命令的 emit 由调用方做（不同工具走不同事件：`internal_command` 不弹审批，
    /// `command_proposed` 弹审批）。本函数只负责等结果回报。
    ///
    /// `pub(in crate::ai::session)` 给 `file_ops` 子模块的 `ensure_remote_caps` / `run_file_op` 共用。
    /// `handle_run_command` 用自己的 loop（要做不同的 audit/emit）—— 没复用这里。
    pub(in crate::ai::session) async fn wait_command_outcome(
        &mut self,
        tool_call_id: &str,
    ) -> AppResult<CommandOutcome> {
        loop {
            let action = match self.action_rx.recv().await {
                Some(a) => a,
                None => return Err(AppError::other("session_channel_closed", json!({}))),
            };
            match action {
                UserAction::CommandResult {
                    tool_call_id: rid,
                    exit_code,
                    output,
                    timed_out,
                    early_terminated,
                } if rid == tool_call_id => {
                    return Ok(CommandOutcome::Result {
                        exit_code,
                        output,
                        timed_out,
                        early_terminated,
                    });
                }
                UserAction::RejectCommand {
                    tool_call_id: rid,
                    reason,
                } if rid == tool_call_id => {
                    return Ok(CommandOutcome::Rejected { reason });
                }
                UserAction::Stop => {
                    return Err(AppError::other("session_stopped_user", json!({})));
                }
                UserAction::Message(text) => {
                    // 工具调用中拒绝新消息（同 handle_run_command 现有行为）
                    let redacted = sanitize::redact(&text, &self.cfg.redact_rules);
                    self.audit_push(AuditKind::Note {
                        message: format!(
                            "user message dropped during tool call {tool_call_id}: {redacted}"
                        ),
                    });
                    self.emit(
                        "error",
                        json!({
                            // 统一英文跟 handle_run_command 的"pending approval"错误一致，前端 ai:error 直显，不绕 i18n
                            "message": "Cannot send a new message while a tool call is running. Wait for it to finish, or approve/reject the command card.",
                        }),
                    );
                    continue;
                }
                _ => continue,
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

        // 100MB 硬上限：拒绝 max_mb 申请就超的请求，免得 SFTP 起头后才 abort
        if input.max_mb > MAX_DOWNLOAD_MB {
            self.push_tool_error(
                &tc.id,
                &format!(
                    "rssh caps download_file at {MAX_DOWNLOAD_MB} MB (you requested max_mb={}). \
                     Don't retry with a smaller max_mb if the actual file is larger — `ls -l` it first. \
                     For artifacts >{MAX_DOWNLOAD_MB} MB, tell the user to transfer {} via scp / rsync / sz \
                     to their local machine themselves, then call `analyze_locally` on that local path.",
                    input.max_mb, input.remote_path
                ),
            );
            return Ok(());
        }

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

        // 审批卡片：跟 run_command / patch_file 同一个 command_proposed 事件，前端按
        // kind="download_file" 决定是否走 auto_download_file 自动批准。SFTP 不走 PTY，
        // 所以 full_cmd/sentinel 填空——前端识别 kind 后直接 ack 不发命令到终端。
        //
        // side_effect 展示实际写入目录前缀（绝对路径），避免用 `~/.../` 这种省略号文案
        // 让用户误以为是真路径片段。
        let dest_dir = self
            .cfg
            .data_dir
            .join("diagnose")
            .join(&self.cfg.session_id);
        self.emit(
            "command_proposed",
            json!({
                "id": dl_id,
                "tool_call_id": tc.id,
                "cmd": format!("download_file: {} (max {} MB)", input.remote_path, input.max_mb),
                "full_cmd": "",
                "sentinel": "",
                "explain": "SFTP download remote artifact to local rssh data dir for offline analysis.",
                "side_effect": format!("Write under {}/", dest_dir.display()),
                "timeout_s": 600,
                "kind": "download_file",
            }),
        );
        // 跟 run_command 一致：审批 + 实际执行的端到端耗时计入 duration_ms，
        // 前端 CommandResult.duration_ms 期待这个字段，缺了会渲染 "undefinedms"。
        let started_at = std::time::Instant::now();

        match self.wait_command_outcome(&tc.id).await? {
            CommandOutcome::Rejected { reason } => {
                self.audit_push(AuditKind::CommandRejected {
                    id: dl_id.clone(),
                    reason: reason.clone(),
                });
                // 跟 run_command / file_ops 一致用 command_rejected —— 前端 listener
                // 清 pending + 把 ChatItem.rejected 填上。之前用 fake command_completed
                // + exit_code=1 + 中文 "已拒绝" output 是 hack（rejection 被 UI 当成
                // failed execution，填 result 不是 rejected）。
                self.emit(
                    "command_rejected",
                    json!({
                        "id": dl_id,
                        "reason": reason.clone(),
                    }),
                );
                self.push_tool_error(
                    &tc.id,
                    &format!("User rejected download_file. Reason: {reason}."),
                );
                return Ok(());
            }
            CommandOutcome::Result { .. } => { /* approved (前端 ack)，继续实际 SFTP */ }
        }

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
                .map_err(|e| AppError::other("ai_local_dir_create_failed", json!({ "err": e.to_string() })))?;
            let sftp = SftpHandle::from_handle(&ssh_handle, self.cfg.target_id.clone()).await?;
            // SFTP 不展开 `~` —— 协议层直接把字面 `~/foo` 当文件名 stat，必然 ENOENT。
            // LLM 习惯先 `ls ~/foo` 验证（shell 展开了），再原样塞进 remote_path。
            // 在入口把 `~` / `~/` 替换成 sftp.home_dir() canonicalize 出来的绝对路径。
            // 其它形态（`~user/...`）SFTP 没法解，留给用户自己写绝对路径。
            let resolved = if input.remote_path == "~" || input.remote_path.starts_with("~/") {
                let home = sftp.home_dir().await?;
                if input.remote_path == "~" {
                    home
                } else {
                    format!("{home}/{}", &input.remote_path[2..])
                }
            } else {
                input.remote_path.clone()
            };
            sftp.download_to_path(&resolved, &local_path, max_bytes)
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
                let card_output = format!("已下载 {} 字节 → {}", bytes, local_str);
                self.emit(
                    "command_completed",
                    json!({
                        "id": dl_id,
                        "exit_code": 0,
                        "timed_out": false,
                        "early_terminated": false,
                        "output": card_output,
                        "original_bytes": card_output.len(),
                        "truncated_bytes": 0,
                        "duration_ms": started_at.elapsed().as_millis() as u64,
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
                // 卡片 output 是给用户看的文本，不能塞 AppError 的 `__rssh_err__|JSON` 协议串
                // （那是给前端 errMsg() 翻译用的，渲染到 <pre> 里就是赤裸的协议体）。
                // 用 code() 给个语义分类 + 远端路径让用户辨识，足以指导下一步动作。
                let card_msg = match e.code() {
                    "sftp_file_too_large" => format!(
                        "远端文件超出 {MAX_DOWNLOAD_MB} MB 上限：{}",
                        input.remote_path
                    ),
                    "sftp_io_failed" => format!(
                        "无法访问远端文件（不存在或不可读）：{}",
                        input.remote_path
                    ),
                    _ => format!("下载失败：{}", input.remote_path),
                };
                self.emit(
                    "command_completed",
                    json!({
                        "id": dl_id,
                        "exit_code": 1,
                        "timed_out": false,
                        "early_terminated": false,
                        "output": card_msg,
                        "original_bytes": 0,
                        "truncated_bytes": 0,
                        "duration_ms": started_at.elapsed().as_millis() as u64,
                    }),
                );
                let msg = if e.code() == "sftp_file_too_large" {
                    format!(
                        "Remote file {} exceeds rssh's {} MB download cap (size was discovered mid-transfer or grew past the requested max_mb). \
                         Don't retry — ask the user to transfer it via scp / rsync / sz to their local machine, then call `analyze_locally` on the local path they paste back.",
                        input.remote_path, MAX_DOWNLOAD_MB
                    )
                } else {
                    format!(
                        "SFTP transfer failed ({e}). Common cause: the user manually ssh'd through a bastion, so rssh's connection terminates at the bastion and can't see the target's filesystem. \
                         Tell the user to pull {} via scp / rsync / sz to their local machine themselves, then paste the key analysis output back into the chat.",
                        input.remote_path
                    )
                };
                self.push_tool_error(&tc.id, &msg);
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

        // 审批卡片：开新窗口副作用比较大（独立窗口、独立 AI 会话、消耗一次 API 调用），
        // 走 command_proposed，前端按 kind="analyze_locally" 决策是否 auto-approve。
        let card_id = uuid::Uuid::new_v4().to_string();
        self.emit(
            "command_proposed",
            json!({
                "id": card_id,
                "tool_call_id": tc.id,
                "cmd": format!("analyze_locally: {} ({})", input.local_path, input.task),
                "full_cmd": "",
                "sentinel": "",
                "explain": "Spawn a new window with an independent AI session to analyze the local artifact.",
                "side_effect": "New window opens; local AI session starts; current session unaffected.",
                "timeout_s": 30,
                "kind": "analyze_locally",
            }),
        );
        let started_at = std::time::Instant::now();
        match self.wait_command_outcome(&tc.id).await? {
            CommandOutcome::Rejected { reason } => {
                self.audit_push(AuditKind::CommandRejected {
                    id: card_id.clone(),
                    reason: reason.clone(),
                });
                // 跟 download_file / run_command / file_ops 统一用 command_rejected
                self.emit(
                    "command_rejected",
                    json!({
                        "id": card_id,
                        "reason": reason.clone(),
                    }),
                );
                self.push_tool_error(
                    &tc.id,
                    &format!("User rejected analyze_locally. Reason: {reason}."),
                );
                return Ok(());
            }
            CommandOutcome::Result { .. } => { /* approved，继续实际开窗口 */ }
        }

        // 把 handoff 注入到新窗口的 window.__rssh_ai_handoff；
        // 新窗口的 AppShell 在 onMount 里读它 → 建本地 shell tab → 启动独立 AI 会话 → 发首条消息。
        let handoff = json!({
            "local_path": input.local_path,
            "task": input.task,
        })
        .to_string();
        // tool_use 必须有对应 tool_result，否则下一轮 LLM 请求 400（Anthropic 严格）。
        // 失败一律走 push_tool_error，绝不 `?` 让错误冒到 dialogue_turn 的 for 循环——
        // 那会让本轮其它已 push 的 tool_result 与未 push 的 tool_use 配对错乱。
        let json_literal = match serde_json::to_string(&handoff) {
            Ok(s) => s,
            Err(e) => {
                self.push_tool_error(
                    &tc.id,
                    &format!("Failed to encode handoff payload: {e}"),
                );
                return Ok(());
            }
        };
        // 直接把 JSON 字符串赋值为 JS string；前端走 JSON.parse(data) 还原。
        // 不要在这里 JSON.parse —— 否则 window.__rssh_ai_handoff 已经是 object，
        // 前端再 JSON.parse 会撞 "[object Object]" 解析失败。
        let init_script = format!("window.__rssh_ai_handoff = {};", json_literal);
        let label = format!("rssh-ai-{}", uuid::Uuid::new_v4().simple());

        // Tauri 2 把 .title()/.inner_size() 等窗口方法限定在 #[cfg(desktop)]，
        // 移动端不存在。analyze_locally 的本质就是开新窗口，移动端语义上不存在，
        // 直接告知 LLM 工具不可用。
        // 简单的卡片关闭辅助：开窗成功 / 失败 / 移动端都得 emit command_completed，
        // 否则 UI 上审批卡片一直停在 "executing"（前端已 ack 但没拿到结果事件）。
        let emit_done = |this: &Self, exit: i32, output: String| {
            this.emit(
                "command_completed",
                json!({
                    "id": card_id,
                    "exit_code": exit,
                    "timed_out": false,
                    "early_terminated": false,
                    "output": output,
                    "original_bytes": 0,
                    "truncated_bytes": 0,
                    "duration_ms": started_at.elapsed().as_millis() as u64,
                }),
            );
        };

        #[cfg(desktop)]
        {
            use tauri::{WebviewUrl, WebviewWindowBuilder};
            // 同上：窗口创建失败也走 push_tool_error，保持 tool_use/tool_result 配对。
            if let Err(e) = WebviewWindowBuilder::new(&self.app, &label, WebviewUrl::App("index.html".into()))
                .title("RSSH — Local Analysis")
                .inner_size(1200.0, 800.0)
                .initialization_script(&init_script)
                .build()
            {
                emit_done(self, 1, format!("打开分析窗口失败：{e}"));
                self.push_tool_error(
                    &tc.id,
                    &format!(
                        "Failed to open analysis window: {e}. Continue diagnosis in the current session."
                    ),
                );
                return Ok(());
            }

            self.audit_push(AuditKind::Note {
                message: format!(
                    "analyze_locally: spawned new window for {} (task: {})",
                    input.local_path, input.task
                ),
            });

            emit_done(self, 0, format!("已打开分析窗口：{}", input.local_path));
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
            emit_done(self, 1, "该功能仅支持桌面端".into());
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
        // 审计语义：从 AI 提议命令到收到执行结果的端到端耗时（含用户犹豫 + shell 跑命令）。
        // 不拆分 approve/run 两段——审计要看完整决策时序，单一数即可。
        let started_at = std::time::Instant::now();

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
                "kind": "run_command",
            }),
        );

        loop {
            let action = match self.action_rx.recv().await {
                Some(a) => a,
                None => return Err(AppError::other("session_channel_closed", json!({}))),
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
                    // 前端只在 command_completed/command_rejected 上清 pending；
                    // 缺这个 emit 会让用户拒绝后命令卡片一直 pending 卡死。
                    self.emit(
                        "command_rejected",
                        json!({
                            "id": cmd_id.clone(),
                            "reason": reason.clone(),
                        }),
                    );
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
                    early_terminated,
                } if tool_call_id == tc.id => {
                    let redacted = sanitize::redact(&output, &self.cfg.redact_rules);
                    let trunc = sanitize::truncate(&redacted, self.cfg.max_output_bytes);

                    self.emit(
                        "command_completed",
                        json!({
                            "id": cmd_id,
                            "exit_code": exit_code,
                            "timed_out": timed_out,
                            "early_terminated": early_terminated,
                            "output": trunc.text,
                            "original_bytes": trunc.original_bytes,
                            "truncated_bytes": trunc.truncated_bytes,
                            "duration_ms": started_at.elapsed().as_millis() as u64,
                        }),
                    );

                    self.audit_push(AuditKind::CommandExecuted {
                        id: cmd_id,
                        exit_code,
                        output_redacted: trunc.text.clone(),
                        original_bytes: trunc.original_bytes,
                        truncated_bytes: trunc.truncated_bytes,
                        duration_ms: started_at.elapsed().as_millis() as u64,
                    });

                    let tool_payload = format!(
                        "exit={exit_code} timed_out={timed_out} early_terminated={early_terminated}\n--- output ---\n{}",
                        trunc.text
                    );
                    self.history.push(ChatMessage::ToolResult {
                        tool_call_id: tc.id,
                        content: tool_payload,
                        is_error: timed_out || early_terminated || exit_code != 0,
                    });
                    return Ok(());
                }
                UserAction::Stop => return Err(AppError::other("session_stopped_user", json!({}))),
                // 命令审批期间不能接受新消息：tool_use 必须有对应 tool_result 才能再开下一轮 user。
                // 之前 _ => continue 把 Message 默默吞掉——用户敲完字消息消失，没有任何反馈。
                // 现在显式 audit + emit ai:error，让用户知道"先决定命令再发消息"。
                UserAction::Message(text) => {
                    // 不要把用户原文裸塞进 audit——可能含 secret/PII（用户复制粘贴
                    // 时随手带的）。audit log 可能离开本机（用户分享给开发者排错），
                    // 走跟 history/command_output 同一套 redact 规则，至少把已知模式
                    // 的敏感串脱掉。
                    let redacted = sanitize::redact(&text, &self.cfg.redact_rules);
                    self.audit_push(AuditKind::Note {
                        message: format!(
                            "user message dropped during command approval (pending tool_call {}): {redacted}",
                            tc.id
                        ),
                    });
                    self.emit(
                        "error",
                        json!({
                            "message": "Cannot send a new message while a command is pending approval. Approve or reject the command first.",
                        }),
                    );
                    continue;
                }
                // 落到这里的只剩 stale RejectCommand/CommandResult（id 不匹配），静默丢即可。
                _ => continue,
            }
        }
    }

    pub(in crate::ai::session) fn push_tool_error(&mut self, tool_call_id: &str, msg: &str) {
        self.history.push(ChatMessage::ToolResult {
            tool_call_id: tool_call_id.to_string(),
            content: msg.to_string(),
            is_error: true,
        });
        self.audit_push(AuditKind::Note {
            message: format!("[tool_error {tool_call_id}] {msg}"),
        });
    }

    pub(in crate::ai::session) fn audit_push(&self, kind: AuditKind) {
        if let Ok(mut g) = self.audit.lock() {
            g.push(kind);
        }
    }

    pub(in crate::ai::session) fn emit(&self, kind: &str, payload: serde_json::Value) {
        let event = format!("ai:{kind}:{}", self.cfg.session_id);
        let _ = self.app.emit(&event, payload);
    }
}

