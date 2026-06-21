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
    /// Decoded-but-not-yet-terminated event text (split on `\n\n`).
    buf: String,
    /// Bytes of an incomplete trailing UTF-8 char from the previous chunk.
    /// `reqwest` splits the stream at arbitrary byte boundaries, so a multibyte
    /// char (CJK / emoji) can straddle two chunks; we hold the partial tail here
    /// and prepend it to the next chunk instead of decoding each chunk alone
    /// (which produced replacement chars on both sides — silent corruption that
    /// still parsed as JSON and got persisted).
    pending: Vec<u8>,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            pending: Vec::new(),
        }
    }

    /// 喂入新字节，返回若干完整事件的 data 行（多行 data: 已合并）。
    /// 接收原始字节并自己处理 UTF-8 边界 —— 调用方不再各自 from_utf8_lossy。
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        self.decode_into_buf(chunk);
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

    /// Append `chunk` to `buf`, decoding UTF-8 across chunk boundaries. The
    /// maximal valid prefix is decoded; an incomplete trailing char is stashed
    /// in `pending` for the next call; genuinely invalid bytes become U+FFFD
    /// (the same lossy behavior as before, minus the splitting of real chars at
    /// chunk seams).
    fn decode_into_buf(&mut self, chunk: &[u8]) {
        let mut bytes = std::mem::take(&mut self.pending);
        bytes.extend_from_slice(chunk);

        let mut rest: &[u8] = &bytes;
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    self.buf.push_str(s);
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    // `rest[..valid]` is valid UTF-8 by definition of valid_up_to.
                    self.buf
                        .push_str(std::str::from_utf8(&rest[..valid]).unwrap_or(""));
                    match e.error_len() {
                        // Unexpected end: a valid prefix of a multibyte char sits
                        // at the tail. Hold it for the next chunk.
                        None => {
                            self.pending.extend_from_slice(&rest[valid..]);
                            break;
                        }
                        // n genuinely invalid bytes: emit one replacement char,
                        // skip past them, keep decoding the remainder.
                        Some(n) => {
                            self.buf.push('\u{FFFD}');
                            rest = &rest[valid + n..];
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod sse_tests {
    use super::SseParser;

    #[test]
    fn reassembles_multibyte_char_split_across_chunks() {
        // "中" = E4 B8 AD. Cut right after E4 so each half is invalid UTF-8 on
        // its own — the exact shape reqwest produces mid-stream. Per-chunk lossy
        // decoding (the old bug) yielded U+FFFD on both sides; the parser must
        // instead carry the partial char across feeds.
        let mut p = SseParser::new();
        let event = "data: hi 中\n\n".as_bytes();
        let cut = event.iter().position(|&b| b == 0xE4).unwrap() + 1;
        assert!(p.feed(&event[..cut]).is_empty());
        assert_eq!(p.feed(&event[cut..]), vec!["hi 中".to_string()]);
    }

    #[test]
    fn emits_replacement_for_genuinely_invalid_bytes() {
        // A lone 0xFF is never valid and never a multibyte prefix: it must become
        // U+FFFD (matching lossy) and not be hoarded as "pending" forever, which
        // would stall the stream.
        let mut p = SseParser::new();
        let mut bytes = b"data: ".to_vec();
        bytes.push(0xFF);
        bytes.extend_from_slice(b"x\n\n");
        assert_eq!(p.feed(&bytes), vec!["\u{FFFD}x".to_string()]);
    }

    #[test]
    fn splits_multiple_events_in_one_chunk() {
        // Sanity: event framing still works on the new byte path.
        let mut p = SseParser::new();
        let events = p.feed(b"data: a\n\ndata: b\n\n");
        assert_eq!(events, vec!["a".to_string(), "b".to_string()]);
    }
}
