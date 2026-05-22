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
use super::file_ops::{collect_matches, compute_unified_diff};
use super::llm::{ChatDelta, ChatMessage, ChatRequest, DeltaSink, LlmClient, ToolCall};
use super::sanitize::{self, RedactRule};
use super::skills::SkillRecord;
use super::tools::{
    self, AnalyzeLocallyInput, DownloadFileInput, LoadSkillInput, MatchFileInput, PatchFileInput,
    RunCommandInput, MATCH_CONTEXT_DEFAULT, MATCH_CONTEXT_MAX,
};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// 远端文件写能力。lazy 探测一次后缓存到 session 生命周期。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RemoteCapabilities {
    /// `python3` 在 PATH 上 — 走预制 python3 脚本写 tmp（传输 = find+replace 的 b64）
    python3: bool,
    /// `base64` 在 PATH 上 — 走 base64 全文（传输 = 整个文件的 b64）
    base64: bool,
}

impl RemoteCapabilities {
    fn none() -> Self {
        Self {
            python3: false,
            base64: false,
        }
    }
}

/// 探测命令：一行拿全部结果。输出形如 `py3=1 b64=0`。
/// 用 `command -v` 而不是 `which` —— 前者 POSIX，后者一些 shell 没有。
const PROBE_CMD: &str =
    r#"echo "py3=$(command -v python3 >/dev/null 2>&1 && echo 1 || echo 0) b64=$(command -v base64 >/dev/null 2>&1 && echo 1 || echo 0)""#;

/// 预制 python3 修改脚本（单行版本）。
///
/// 单行设计原因：rssh 前端把 full_cmd + "\n" 直接发 PTY，多行 `\n` 会触发 shell ps2 续行
/// 提示，用户在 PTY 里看到一串 `> > > >` 视觉糟糕。单行 + `;` 分隔规避。
///
/// 语义保证：python `str.count` / `str.replace` 都是字面字符串、不重叠、一次扫描，
/// 与 Rust `str::count` / `str::replace` **字节级一致**（UTF-8 文件）。这是
/// "修改结果 == 显示给用户的 diff" 这条硬指标的核心保证。
const PYTHON_PATCH_SCRIPT: &str = r#"import sys,base64; p,bf,br,e,t=sys.argv[1:6]; d=open(p,"rb").read().decode("utf-8"); f=base64.b64decode(bf).decode("utf-8"); r=base64.b64decode(br).decode("utf-8"); sys.exit(2) if d.count(f)!=int(e) else open(t,"wb").write(d.replace(f,r).encode("utf-8"))"#;

/// 解析探测命令输出。容忍前后噪音（PTY 可能带 prompt 残留 / OSC 序列），
/// 找到含 `py3=` 和 `b64=` 的那一行。
fn parse_capabilities(output: &str) -> RemoteCapabilities {
    let mut caps = RemoteCapabilities::none();
    for line in output.lines() {
        let line = line.trim();
        if !(line.contains("py3=") && line.contains("b64=")) {
            continue;
        }
        for token in line.split_whitespace() {
            if let Some(v) = token.strip_prefix("py3=") {
                caps.python3 = v == "1";
            } else if let Some(v) = token.strip_prefix("b64=") {
                caps.base64 = v == "1";
            }
        }
        return caps;
    }
    caps
}

/// 拼装 python3 写命令：远端读文件 → str.replace → 写 tmp，rssh 在 shell 层 mv 替换。
/// find/replace 走 base64 传，避免双层（shell + python source）转义。
fn build_python_write_cmd(
    path: &str,
    find: &str,
    replace: &str,
    expected: u32,
    tmp: &str,
) -> String {
    let b64f = B64.encode(find.as_bytes());
    let b64r = B64.encode(replace.as_bytes());
    format!(
        "python3 -c {} {} {} {} {} {} && mv -- {} {}",
        shell_quote(PYTHON_PATCH_SCRIPT),
        shell_quote(path),
        shell_quote(&b64f),
        shell_quote(&b64r),
        shell_quote(&expected.to_string()),
        shell_quote(tmp),
        shell_quote(tmp),
        shell_quote(path),
    )
}

/// 拼装 base64 全文写命令：本地算好的新内容直接 base64 推到远端解码 + mv。
/// 字节精确（不依赖远端工具语义），是降级兜底。
fn build_base64_write_cmd(new_content: &str, tmp: &str, path: &str) -> String {
    let new_b64 = B64.encode(new_content.as_bytes());
    format!(
        "printf '%s' {} | base64 -d > {} && mv -- {} {}",
        shell_quote(&new_b64),
        shell_quote(tmp),
        shell_quote(tmp),
        shell_quote(path),
    )
}

/// POSIX 单引号转义。所有 rssh 后端拼装的 PTY 命令都走这个，避免路径含
/// 空格/特殊字符时被 shell 错误解析。
///
/// 转义规则：把整个串包在单引号里；输入中的每个单引号替换为 `'\''`
/// （关闭引号 → 转义字面单引号 → 重开引号）。这是 POSIX shell 通用形态，
/// bash/dash/zsh/ash/busybox 都正确解析，且能可靠覆盖所有 shell 元字符
/// （$ ` * ? & | ; < > 等单引号内都按字面）。
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 工具命令在前端 PTY 跑完后的两种结果。
#[derive(Debug)]
enum CommandOutcome {
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

struct Actor {
    cfg: SessionConfig,
    system_prompt: String,
    history: Vec<ChatMessage>,
    action_rx: mpsc::UnboundedReceiver<UserAction>,
    audit: Arc<Mutex<AuditLog>>,
    app: AppHandle,
    cancel_slot: Arc<Mutex<Option<Arc<Notify>>>>,
    /// 远端 patch_file 写能力 — lazy 探测，session 内缓存。
    /// None = 还没探测；Some = 已探测，结果有效到 session 结束。
    remote_caps: Option<RemoteCapabilities>,
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


    /// 第一次 patch_file 时探测远端写能力（python3 / base64），结果缓存到 session 结束。
    /// 后续 patch_file 直接读缓存，无网络往返。
    async fn ensure_remote_caps(&mut self) -> AppResult<RemoteCapabilities> {
        if let Some(c) = self.remote_caps {
            return Ok(c);
        }
        let probe_tc_id = uuid::Uuid::new_v4().to_string();
        let cmd_id = uuid::Uuid::new_v4().to_string();
        let sentinel = format!("__rssh_done_{}", uuid::Uuid::new_v4().simple());
        let full_cmd = format!("{}; echo \"{}:$?\"", PROBE_CMD, sentinel);

        self.audit_push(AuditKind::Note {
            message: "patch_file: probing remote capabilities (python3 / base64)".into(),
        });
        self.emit(
            "internal_command",
            json!({
                "id": cmd_id,
                "tool_call_id": probe_tc_id,
                "cmd": PROBE_CMD,
                "full_cmd": full_cmd,
                "sentinel": sentinel,
            }),
        );

        let output = match self.wait_command_outcome(&probe_tc_id).await? {
            CommandOutcome::Result { output, .. } => output,
            CommandOutcome::Rejected { reason } => {
                return Err(AppError::other(
                    "caps_probe_aborted",
                    json!({ "reason": reason }),
                ));
            }
        };
        let caps = parse_capabilities(&output);
        self.audit_push(AuditKind::Note {
            message: format!(
                "patch_file: caps probed — python3={} base64={}",
                caps.python3, caps.base64
            ),
        });
        self.remote_caps = Some(caps);
        Ok(caps)
    }

    /// 等待前端汇报命令结果或拒绝。
    ///
    /// 命令的 emit 由调用方做（不同工具走不同事件：internal_command 不弹审批，
    /// command_proposed 弹审批）。本函数只负责等结果回报。
    async fn wait_command_outcome(&mut self, tool_call_id: &str) -> AppResult<CommandOutcome> {
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
                            "message": "Cannot send a new message while a tool is running. Wait for the result, or approve/reject the pending command.",
                        }),
                    );
                    continue;
                }
                _ => continue,
            }
        }
    }

    async fn handle_match_file(&mut self, tc: ToolCall) -> AppResult<()> {
        let input: MatchFileInput = match serde_json::from_value(tc.input.clone()) {
            Ok(i) => i,
            Err(e) => {
                self.push_tool_error(&tc.id, &format!("Failed to parse input: {e}"));
                return Ok(());
            }
        };

        if input.find.is_empty() {
            self.push_tool_error(
                &tc.id,
                "match_file: `find` must not be empty (it would match the entire file).",
            );
            return Ok(());
        }

        let before = input.before.unwrap_or(MATCH_CONTEXT_DEFAULT).min(MATCH_CONTEXT_MAX) as usize;
        let after = input.after.unwrap_or(MATCH_CONTEXT_DEFAULT).min(MATCH_CONTEXT_MAX) as usize;

        // Stage A：通过 PTY 跑 cat（internal，不弹审批）
        let cmd_id = uuid::Uuid::new_v4().to_string();
        let sentinel = format!("__rssh_done_{}", uuid::Uuid::new_v4().simple());
        let cat_cmd = format!("cat -- {}", shell_quote(&input.path));
        let full_cmd = format!("{}; echo \"{}:$?\"", cat_cmd, sentinel);

        self.audit_push(AuditKind::Note {
            message: format!("match_file: cat {} (internal)", input.path),
        });
        self.emit(
            "internal_command",
            json!({
                "id": cmd_id,
                "tool_call_id": tc.id,
                "cmd": cat_cmd,
                "full_cmd": full_cmd,
                "sentinel": sentinel,
            }),
        );

        let outcome = self.wait_command_outcome(&tc.id).await?;
        let (exit_code, output) = match outcome {
            CommandOutcome::Result {
                exit_code, output, ..
            } => (exit_code, output),
            CommandOutcome::Rejected { reason } => {
                // internal_command 不应该被前端 reject，但万一发生兜底处理
                self.push_tool_error(
                    &tc.id,
                    &format!("match_file aborted by frontend: {reason}"),
                );
                return Ok(());
            }
        };

        if exit_code != 0 {
            self.push_tool_error(
                &tc.id,
                &format!(
                    "Failed to read file {} (exit {exit_code}). Likely causes: path doesn't exist, no read permission, or remote path wrong. Output: {}",
                    input.path,
                    output.chars().take(400).collect::<String>(),
                ),
            );
            return Ok(());
        }

        // 字面查找 + 提取上下文
        let result = collect_matches(&output, &input.find, before, after);
        let payload = serde_json::to_string(&result)
            .unwrap_or_else(|_| r#"{"count":0,"matches":[]}"#.to_string());

        self.audit_push(AuditKind::Note {
            message: format!(
                "match_file: {} -> count={}",
                input.path, result.count
            ),
        });
        self.history.push(ChatMessage::ToolResult {
            tool_call_id: tc.id,
            content: payload,
            is_error: false,
        });
        Ok(())
    }

    async fn handle_patch_file(&mut self, tc: ToolCall) -> AppResult<()> {
        let input: PatchFileInput = match serde_json::from_value(tc.input.clone()) {
            Ok(i) => i,
            Err(e) => {
                self.push_tool_error(&tc.id, &format!("Failed to parse input: {e}"));
                return Ok(());
            }
        };

        if input.find.is_empty() {
            self.push_tool_error(
                &tc.id,
                "patch_file: `find` must not be empty (use match_file to discover what to change).",
            );
            return Ok(());
        }
        if input.expected_count == 0 {
            self.push_tool_error(
                &tc.id,
                "patch_file: `expected_count` must be >= 1. Use match_file first to discover the actual count, then pass it here.",
            );
            return Ok(());
        }

        // Stage A: 走 internal_command 跑 cat 拿旧内容
        let read_cmd_id = uuid::Uuid::new_v4().to_string();
        let read_sentinel = format!("__rssh_done_{}", uuid::Uuid::new_v4().simple());
        let cat_cmd = format!("cat -- {}", shell_quote(&input.path));
        let full_read = format!("{}; echo \"{}:$?\"", cat_cmd, read_sentinel);

        self.audit_push(AuditKind::Note {
            message: format!("patch_file: stage A cat {}", input.path),
        });
        self.emit(
            "internal_command",
            json!({
                "id": read_cmd_id,
                "tool_call_id": tc.id,
                "cmd": cat_cmd,
                "full_cmd": full_read,
                "sentinel": read_sentinel,
            }),
        );

        let (exit_a, old_content) = match self.wait_command_outcome(&tc.id).await? {
            CommandOutcome::Result {
                exit_code, output, ..
            } => (exit_code, output),
            CommandOutcome::Rejected { reason } => {
                self.push_tool_error(
                    &tc.id,
                    &format!("patch_file stage A aborted by frontend: {reason}"),
                );
                return Ok(());
            }
        };

        if exit_a != 0 {
            self.push_tool_error(
                &tc.id,
                &format!(
                    "Failed to read file {} (exit {exit_a}). Output: {}",
                    input.path,
                    old_content.chars().take(400).collect::<String>(),
                ),
            );
            return Ok(());
        }

        // 校验 actual_count == expected_count（用 collect_matches 保证非重叠语义）
        let match_info = collect_matches(&old_content, &input.find, 0, 0);
        if match_info.count != input.expected_count as usize {
            let first_lines: Vec<String> = match_info
                .matches
                .iter()
                .take(3)
                .map(|m| m.line.to_string())
                .collect();
            let lines_hint = if first_lines.is_empty() {
                String::new()
            } else {
                format!(" (first matches at lines {})", first_lines.join(", "))
            };
            self.push_tool_error(
                &tc.id,
                &format!(
                    "patch_file: count mismatch — file currently has {} occurrence(s) of `find`, but expected_count was {}{}. \
                     Re-run match_file to refresh the count, then call patch_file with the correct expected_count.",
                    match_info.count, input.expected_count, lines_hint,
                ),
            );
            return Ok(());
        }

        // 算新内容 + diff（本地算，diff 必须与最终落盘内容字节一致）
        let new_content = old_content.replace(&input.find, &input.replace);
        let diff = compute_unified_diff(&input.path, &old_content, &new_content);

        // 探测远端写能力，按能力选命令拼装
        let caps = self.ensure_remote_caps().await?;
        let tmp_suffix: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(8)
            .collect();
        let tmp_path = format!("{}.rssh-{}", input.path, tmp_suffix);

        let (write_cmd, strategy) = if caps.python3 {
            (
                build_python_write_cmd(
                    &input.path,
                    &input.find,
                    &input.replace,
                    input.expected_count,
                    &tmp_path,
                ),
                "python3",
            )
        } else if caps.base64 {
            (
                build_base64_write_cmd(&new_content, &tmp_path, &input.path),
                "base64",
            )
        } else {
            self.push_tool_error(
                &tc.id,
                "patch_file: remote system lacks both python3 and base64 — rssh cannot write the file via PTY. \
                 Tell the user to install python3 or coreutils (for base64), or to edit the file manually.",
            );
            return Ok(());
        };

        // Stage B: 构造写命令 + 审批 UI
        let write_cmd_id = uuid::Uuid::new_v4().to_string();
        let write_sentinel = format!("__rssh_done_{}", uuid::Uuid::new_v4().simple());
        let full_write = format!("{}; echo \"{}:$?\"", write_cmd, write_sentinel);

        self.audit_push(AuditKind::CommandProposed {
            id: write_cmd_id.clone(),
            cmd: write_cmd.clone(),
            explain: format!(
                "patch_file: replace {} occurrence(s) in {} (strategy: {strategy})",
                input.expected_count, input.path
            ),
            side_effect: format!("Writes to {} (atomic: tmp + mv)", input.path),
        });
        let started_at = std::time::Instant::now();

        self.emit(
            "command_proposed",
            json!({
                "id": write_cmd_id,
                "tool_call_id": tc.id,
                "kind": "patch_file",
                "path": input.path,
                "diff": diff,
                "changed": input.expected_count,
                "cmd": write_cmd,
                "full_cmd": full_write,
                "sentinel": write_sentinel,
                "explain": format!("Patch {} ({} occurrence(s))", input.path, input.expected_count),
                "side_effect": format!("Atomic write: tmp + mv -- {}", input.path),
                "timeout_s": 60,
            }),
        );

        match self.wait_command_outcome(&tc.id).await? {
            CommandOutcome::Rejected { reason } => {
                self.audit_push(AuditKind::CommandRejected {
                    id: write_cmd_id,
                    reason: reason.clone(),
                });
                self.push_tool_error(
                    &tc.id,
                    &format!(
                        "User rejected the patch. Reason: {reason}. Reconsider — maybe ask the user what they want changed instead."
                    ),
                );
                Ok(())
            }
            CommandOutcome::Result {
                exit_code,
                output,
                timed_out,
                early_terminated,
            } => {
                let redacted = sanitize::redact(&output, &self.cfg.redact_rules);
                let trunc = sanitize::truncate(&redacted, self.cfg.max_output_bytes);
                self.emit(
                    "command_completed",
                    json!({
                        "id": write_cmd_id,
                        "exit_code": exit_code,
                        "timed_out": timed_out,
                        "early_terminated": early_terminated,
                        "output": trunc.text,
                        "original_bytes": trunc.original_bytes,
                        "truncated_bytes": trunc.truncated_bytes,
                    }),
                );
                self.audit_push(AuditKind::CommandExecuted {
                    id: write_cmd_id,
                    exit_code,
                    output_redacted: trunc.text.clone(),
                    original_bytes: trunc.original_bytes,
                    truncated_bytes: trunc.truncated_bytes,
                    duration_ms: started_at.elapsed().as_millis() as u64,
                });
                if exit_code != 0 {
                    self.push_tool_error(
                        &tc.id,
                        &format!(
                            "patch_file write failed (exit {exit_code}). Output: {}",
                            trunc.text
                        ),
                    );
                    return Ok(());
                }
                let payload = json!({
                    "diff": diff,
                    "changed": input.expected_count,
                })
                .to_string();
                self.history.push(ChatMessage::ToolResult {
                    tool_call_id: tc.id,
                    content: payload,
                    is_error: false,
                });
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
                .map_err(|e| AppError::other("ai_local_dir_create_failed", json!({ "err": e.to_string() })))?;
            let sftp = SftpHandle::from_handle(&ssh_handle, self.cfg.target_id.clone()).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_plain_string() {
        // 无元字符的字符串，直接套单引号即可
        assert_eq!(shell_quote("hello"), "'hello'");
        assert_eq!(shell_quote("/tmp/foo.yml"), "'/tmp/foo.yml'");
    }

    #[test]
    fn shell_quote_empty_string() {
        // 空串也合法 —— shell 看到 '' 等同于空参数
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn shell_quote_with_spaces() {
        // 路径含空格是真实场景（用户主目录 / 含空格的文件名）
        assert_eq!(shell_quote("/tmp/has space/foo"), "'/tmp/has space/foo'");
    }

    #[test]
    fn shell_quote_single_quote_escape() {
        // 核心规则：单引号本身用 '\'' 拼出（关闭引号→反斜杠转义字面引号→重开引号）
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("'"), "''\\'''");
        assert_eq!(shell_quote("a'b'c"), "'a'\\''b'\\''c'");
    }

    #[test]
    fn shell_quote_double_quote_passthrough() {
        // 双引号在单引号内按字面，不转义
        assert_eq!(shell_quote(r#"say "hi""#), r#"'say "hi"'"#);
    }

    #[test]
    fn shell_quote_dollar_and_backtick_neutralized() {
        // $VAR / `cmd` 在单引号内不展开 —— 这是用单引号而非双引号的原因
        assert_eq!(shell_quote("$HOME"), "'$HOME'");
        assert_eq!(shell_quote("`whoami`"), "'`whoami`'");
        assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    #[test]
    fn shell_quote_shell_metacharacters_inert() {
        // |, &, ;, <, >, * 等在单引号内全部按字面
        assert_eq!(shell_quote("a|b&c;d"), "'a|b&c;d'");
        assert_eq!(shell_quote("> /etc/passwd"), "'> /etc/passwd'");
        assert_eq!(shell_quote("*"), "'*'");
        assert_eq!(shell_quote("~/foo"), "'~/foo'");
    }

    #[test]
    fn shell_quote_backslash_passthrough() {
        // 反斜杠在单引号内不是转义字符 —— 按字面输出
        assert_eq!(shell_quote(r"a\b"), r"'a\b'");
        assert_eq!(shell_quote(r"\n"), r"'\n'");
    }

    #[test]
    fn shell_quote_newline_preserved() {
        // 内容含换行也安全 —— shell 在单引号内保留 \n
        assert_eq!(shell_quote("line1\nline2"), "'line1\nline2'");
    }

    #[test]
    fn shell_quote_multibyte_chars() {
        // 非 ASCII（中文 / emoji 等）原样保留
        assert_eq!(shell_quote("文件名.txt"), "'文件名.txt'");
        assert_eq!(shell_quote("emoji 🦀"), "'emoji 🦀'");
    }

    // ─── parse_capabilities ─────────────────────────────────────────

    #[test]
    fn parse_caps_both_present() {
        let r = parse_capabilities("py3=1 b64=1\n");
        assert!(r.python3);
        assert!(r.base64);
    }

    #[test]
    fn parse_caps_only_base64() {
        let r = parse_capabilities("py3=0 b64=1\n");
        assert!(!r.python3);
        assert!(r.base64);
    }

    #[test]
    fn parse_caps_only_python() {
        let r = parse_capabilities("py3=1 b64=0\n");
        assert!(r.python3);
        assert!(!r.base64);
    }

    #[test]
    fn parse_caps_neither() {
        let r = parse_capabilities("py3=0 b64=0\n");
        assert!(!r.python3);
        assert!(!r.base64);
    }

    #[test]
    fn parse_caps_tolerates_pty_noise() {
        // 真实场景：PTY 输出可能含 prompt 残留 / OSC 序列 / 多行输出
        let out = "user@host:~$ echo \"py3=...\"\npy3=1 b64=1\nuser@host:~$ \n";
        let r = parse_capabilities(out);
        assert!(r.python3);
        assert!(r.base64);
    }

    #[test]
    fn parse_caps_missing_fields_defaults_false() {
        // 输出格式坏了 — 所有字段默认 false
        let r = parse_capabilities("garbage output");
        assert!(!r.python3);
        assert!(!r.base64);
    }

    #[test]
    fn parse_caps_empty_input() {
        let r = parse_capabilities("");
        assert!(!r.python3);
        assert!(!r.base64);
    }

    #[test]
    fn parse_caps_non_one_treated_as_false() {
        // 防御：非 1 的值（包括 "2" / "true" / "yes"）一律视作不可用
        let r = parse_capabilities("py3=yes b64=2");
        assert!(!r.python3);
        assert!(!r.base64);
    }

    // ─── build_python_write_cmd ─────────────────────────────────────

    #[test]
    fn python_cmd_contains_required_pieces() {
        let cmd = build_python_write_cmd("/path/file", "old", "new", 1, "/path/file.tmp");
        // 整体形态：python3 -c '<script>' '<path>' '<b64-find>' '<b64-replace>' '<expected>' '<tmp>' && mv -- '<tmp>' '<path>'
        assert!(cmd.starts_with("python3 -c '"), "should start with python3 -c");
        assert!(cmd.contains("&& mv -- "), "should have mv after && short-circuit");
        // python 脚本特征：base64 + str.count + str.replace
        assert!(cmd.contains("base64.b64decode"));
        assert!(cmd.contains("d.count(f)"));
        assert!(cmd.contains("d.replace(f,r)"));
        // path 必须 shell-quoted
        assert!(cmd.contains("'/path/file'"));
        assert!(cmd.contains("'/path/file.tmp'"));
    }

    #[test]
    fn python_cmd_encodes_find_replace_as_base64() {
        let cmd = build_python_write_cmd("/p", "abc", "xyz", 2, "/p.tmp");
        // "abc" -> "YWJj", "xyz" -> "eHl6"
        assert!(cmd.contains("'YWJj'"), "find must be base64 encoded and shell-quoted");
        assert!(cmd.contains("'eHl6'"), "replace must be base64 encoded and shell-quoted");
        // expected_count 也走 shell_quote
        assert!(cmd.contains("'2'"));
    }

    #[test]
    fn python_cmd_handles_special_chars_in_path() {
        // path 含空格 / 单引号 不会破坏命令拼装
        let cmd = build_python_write_cmd("/has space/it's", "x", "y", 1, "/has space/it's.tmp");
        // 单引号被转义成 '\''
        assert!(cmd.contains(r"'/has space/it'\''s'"));
        assert!(cmd.contains(r"'/has space/it'\''s.tmp'"));
    }

    #[test]
    fn python_cmd_does_not_expand_find_replace_to_shell() {
        // find/replace 含 shell 元字符（$VAR, `cmd`, ;）必须**不**被 shell 求值
        // 因为它们走 base64 编码，原文不进 shell command line
        let cmd = build_python_write_cmd("/p", "$HOME", "`whoami`", 1, "/p.tmp");
        let b64_dollar_home = B64.encode(b"$HOME");
        let b64_backtick = B64.encode(b"`whoami`");
        // 命令里**不能**含原始 "$HOME" 或 "`whoami`"（除了在 base64 编码后的位置）
        // 而应该是 base64 字符串
        assert!(cmd.contains(&format!("'{}'", b64_dollar_home)));
        assert!(cmd.contains(&format!("'{}'", b64_backtick)));
    }

    // ─── build_base64_write_cmd ─────────────────────────────────────

    #[test]
    fn base64_cmd_contains_required_pieces() {
        let cmd = build_base64_write_cmd("hello", "/p.tmp", "/p");
        assert!(cmd.contains("printf '%s' "), "should pipe printf to base64 -d");
        assert!(cmd.contains("| base64 -d > "));
        assert!(cmd.contains("&& mv -- "));
        assert!(cmd.contains("'/p.tmp'"));
        assert!(cmd.contains("'/p'"));
    }

    #[test]
    fn base64_cmd_encodes_full_content() {
        let cmd = build_base64_write_cmd("hello", "/p.tmp", "/p");
        // "hello" -> "aGVsbG8="
        assert!(cmd.contains("'aGVsbG8='"));
    }

    #[test]
    fn base64_cmd_handles_multibyte_content() {
        // UTF-8 多字节内容也走 base64 字节级编码
        let cmd = build_base64_write_cmd("中文", "/p.tmp", "/p");
        let expected_b64 = B64.encode("中文".as_bytes());
        assert!(cmd.contains(&format!("'{}'", expected_b64)));
    }

    #[test]
    fn base64_cmd_handles_special_chars_in_path() {
        let cmd = build_base64_write_cmd("x", "/has space/file.tmp", "/has space/file");
        assert!(cmd.contains("'/has space/file.tmp'"));
        assert!(cmd.contains("'/has space/file'"));
    }

    #[test]
    fn shell_quote_idempotent_when_no_single_quote() {
        // 不含单引号的字符串两次套引号会双层（不是 idempotent —— 这是正确的，
        // 因为 shell_quote 输出本身含单引号，再次套需要二次转义）。
        // 这个测试只是文档化"不要重复调用"的约束。
        let once = shell_quote("foo");
        let twice = shell_quote(&once);
        assert_ne!(once, twice);
        assert!(twice.contains("\\'"));
    }
}

