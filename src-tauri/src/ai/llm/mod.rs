//! BYOK LLM 客户端（流式）。
//!
//! 决议 #5：手写 reqwest，适配 Anthropic + OpenAI 兼容端点。
//! 流式：SSE → DeltaSink 增量回调 + 最终 ChatResponse。

pub mod anthropic;
pub mod openai;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ChatMessage {
    User { content: String },
    Assistant { content: String, tool_calls: Vec<ToolCall> },
    ToolResult { tool_call_id: String, content: String, is_error: bool },
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
}

/// 流式增量回调。Text 用于 UI 实时渲染；其余仅供调试 / 暂不消费。
#[derive(Debug, Clone)]
pub enum ChatDelta {
    Text(String),
    ToolStart { tool_call_id: String, name: String },
    ToolArgs { tool_call_id: String, partial: String },
}

pub type DeltaSink = Arc<dyn Fn(ChatDelta) + Send + Sync>;

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat(&self, req: ChatRequest, sink: DeltaSink) -> AppResult<ChatResponse>;
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
        other => Err(crate::error::AppError::Config(format!(
            "未知 LLM provider: {other}"
        ))),
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
            let sep_len = if self.buf[idx..].starts_with("\r\n\r\n") { 4 } else { 2 };
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
