//! BYOK LLM 客户端（流式）。
//!
//! 决议 #5：手写 reqwest + 自解析 SSE，零额外 SDK 依赖。
//!
//! 协议 → 厂商映射：
//! - **Anthropic Messages API**（自家协议） → `anthropic.rs`
//! - **OpenAI Chat Completions**（事实上的标准） → `protocol.rs` 实现，
//!   被以下 vendor 文件复用：`openai.rs` / `deepseek.rs` / `glm.rs`
//!
//! 新增厂商的步骤：写一个 ~40 行的 vendor 文件指定 endpoint + 默认模型 +
//! list_models 实现，然后在 `build_client` 加一行分发即可。

pub mod anthropic;
pub mod deepseek;
pub mod glm;
pub mod openai;
mod protocol;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ChatMessage {
    User {
        content: String,
    },
    Assistant {
        content: String,
        tool_calls: Vec<ToolCall>,
        /// 部分模型（如 DeepSeek `deepseek-reasoner`）会输出"思考链"。
        /// 这些厂商要求多轮对话时把 reasoning 原样塞回去，否则 400。
        /// 其他厂商：传 None，序列化时不会出现该字段，零影响。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
        is_error: bool,
        /// Internal flag: content was already redacted at insertion site.
        /// `redact_message` skips a second pass — important for structured
        /// JSON payloads (file_ops) where re-redacting hex hashes inside
        /// `package-lock.json` / git oid would corrupt the LLM's view of
        /// the file and trigger downstream `count_mismatch` loops.
        ///
        /// Skipped during serialization so neither LLM provider sees it.
        #[serde(skip)]
        pre_redacted: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSchema>,
    pub model: String,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: String,
    pub tokens_in: Option<u32>,
    pub tokens_out: Option<u32>,
    /// 思考链（DeepSeek reasoner 之类）。原样传回 history 给下一轮。
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
}

/// 流式增量回调。Text 用于 UI 实时渲染；其余仅供调试 / 暂不消费。
#[derive(Debug, Clone)]
pub enum ChatDelta {
    Text(String),
    ToolStart {
        tool_call_id: String,
        name: String,
    },
    ToolArgs {
        tool_call_id: String,
        partial: String,
    },
}

pub type DeltaSink = Arc<dyn Fn(ChatDelta) + Send + Sync>;

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat(&self, req: ChatRequest, sink: DeltaSink) -> AppResult<ChatResponse>;
    async fn list_models(&self) -> AppResult<Vec<ModelInfo>>;
    fn provider(&self) -> &'static str;
}

pub fn build_client(
    provider: &str,
    api_key: String,
    endpoint: Option<String>,
) -> AppResult<Box<dyn LlmClient>> {
    match provider {
        "anthropic" => Ok(Box::new(anthropic::AnthropicClient::new(api_key, endpoint))),
        "openai" | "openai-compatible" => {
            Ok(Box::new(openai::OpenAiClient::new(api_key, endpoint)))
        }
        "deepseek" => Ok(Box::new(deepseek::DeepSeekClient::new(api_key, endpoint))),
        "glm" => Ok(Box::new(glm::GlmClient::new(api_key, endpoint))),
        other => Err(crate::error::AppError::config(
            "llm_unknown_provider",
            serde_json::json!({ "provider": other }),
        )),
    }
}

// ─── SSE 解析公共工具 ────────────────────────────────────────────

/// 增量 SSE 解析器：feed 接收任意 byte chunk，返回完整事件的 data 字符串列表。
pub(crate) struct SseParser {
    buf: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self { buf: String::new() }
    }

    /// 喂入新字节，返回若干完整事件的 data 行（多行 data: 已合并）。
    pub fn feed(&mut self, chunk: &str) -> Vec<String> {
        self.buf.push_str(chunk);
        let mut events = Vec::new();
        loop {
            let sep_idx = self.buf.find("\n\n").or_else(|| self.buf.find("\r\n\r\n"));
            let Some(idx) = sep_idx else { break };
            let sep_len = if self.buf[idx..].starts_with("\r\n\r\n") {
                4
            } else {
                2
            };
            let event_text = self.buf[..idx].to_string();
            self.buf = self.buf[idx + sep_len..].to_string();

            let mut data_lines: Vec<&str> = Vec::new();
            for line in event_text.lines() {
                let line = line.trim_end_matches('\r');
                if let Some(d) = line.strip_prefix("data:") {
                    data_lines.push(d.strip_prefix(' ').unwrap_or(d));
                }
            }
            if !data_lines.is_empty() {
                events.push(data_lines.join("\n"));
            }
        }
        events
    }
}
