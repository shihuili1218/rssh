//! Anthropic Messages API 流式客户端。
//!
//! 协议：https://docs.anthropic.com/en/api/messages-streaming
//! 事件类型：message_start / content_block_start / content_block_delta / content_block_stop
//!         / message_delta / message_stop

use std::collections::BTreeMap;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;

use super::{
    ChatDelta, ChatMessage, ChatRequest, ChatResponse, DeltaSink, LlmClient, SseParser, ToolCall,
};
use crate::error::{AppError, AppResult};

const DEFAULT_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicClient {
    api_key: String,
    endpoint: String,
    http: reqwest::Client,
}

impl AnthropicClient {
    pub fn new(api_key: String, endpoint: Option<String>) -> Self {
        Self {
            api_key,
            endpoint: endpoint.unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct AnthropicReq<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<AnthropicMsg>,
    tools: Vec<serde_json::Value>,
    stream: bool,
}

#[derive(Serialize)]
struct AnthropicMsg {
    role: &'static str,
    content: Vec<AnthropicBlock>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[async_trait]
impl LlmClient for AnthropicClient {
    fn provider(&self) -> &'static str {
        "anthropic"
    }

    async fn chat(&self, req: ChatRequest, sink: DeltaSink) -> AppResult<ChatResponse> {
        let messages: Vec<AnthropicMsg> = req
            .messages
            .iter()
            .map(|m| match m {
                ChatMessage::User { content } => AnthropicMsg {
                    role: "user",
                    content: vec![AnthropicBlock::Text {
                        text: content.clone(),
                    }],
                },
                ChatMessage::Assistant {
                    content,
                    tool_calls,
                } => {
                    let mut blocks: Vec<AnthropicBlock> = Vec::new();
                    if !content.is_empty() {
                        blocks.push(AnthropicBlock::Text {
                            text: content.clone(),
                        });
                    }
                    for tc in tool_calls {
                        blocks.push(AnthropicBlock::ToolUse {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            input: tc.input.clone(),
                        });
                    }
                    AnthropicMsg {
                        role: "assistant",
                        content: blocks,
                    }
                }
                ChatMessage::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                } => AnthropicMsg {
                    role: "user",
                    content: vec![AnthropicBlock::ToolResult {
                        tool_use_id: tool_call_id.clone(),
                        content: content.clone(),
                        is_error: *is_error,
                    }],
                },
            })
            .collect();

        let tools: Vec<serde_json::Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        let body = AnthropicReq {
            model: &req.model,
            max_tokens: req.max_tokens,
            system: &req.system_prompt,
            messages,
            tools,
            stream: true,
        };

        let resp = self
            .http
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Other(format!("LLM 请求失败: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Other(format!("LLM 错误 {status}: {text}")));
        }

        // 流式解析
        let mut text_out = String::new();
        // index → (id, name, partial_json)
        let mut tool_calls: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
        let mut stop_reason = String::new();
        let mut tokens_in: Option<u32> = None;
        let mut tokens_out: Option<u32> = None;

        let mut parser = SseParser::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| AppError::Other(format!("LLM stream 读失败: {e}")))?;
            let s = String::from_utf8_lossy(&bytes).into_owned();
            for ev_data in parser.feed(&s) {
                let v: serde_json::Value = match serde_json::from_str(&ev_data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let ev_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match ev_type {
                    "message_start" => {
                        tokens_in = v["message"]["usage"]["input_tokens"]
                            .as_u64()
                            .map(|n| n as u32);
                    }
                    "content_block_start" => {
                        let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                        let cb = &v["content_block"];
                        if cb.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                            let id = cb["id"].as_str().unwrap_or("").to_string();
                            let name = cb["name"].as_str().unwrap_or("").to_string();
                            tool_calls.insert(idx, (id.clone(), name.clone(), String::new()));
                            sink(ChatDelta::ToolStart {
                                tool_call_id: id,
                                name,
                            });
                        }
                    }
                    "content_block_delta" => {
                        let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                        let delta = &v["delta"];
                        match delta.get("type").and_then(|t| t.as_str()) {
                            Some("text_delta") => {
                                let t = delta["text"].as_str().unwrap_or("");
                                if !t.is_empty() {
                                    text_out.push_str(t);
                                    sink(ChatDelta::Text(t.to_string()));
                                }
                            }
                            Some("input_json_delta") => {
                                let p = delta["partial_json"].as_str().unwrap_or("");
                                if let Some(entry) = tool_calls.get_mut(&idx) {
                                    entry.2.push_str(p);
                                    sink(ChatDelta::ToolArgs {
                                        tool_call_id: entry.0.clone(),
                                        partial: p.to_string(),
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                    "message_delta" => {
                        if let Some(reason) = v["delta"]["stop_reason"].as_str() {
                            stop_reason = reason.to_string();
                        }
                        if let Some(o) = v["usage"]["output_tokens"].as_u64() {
                            tokens_out = Some(o as u32);
                        }
                    }
                    "message_stop" => {}
                    _ => {}
                }
            }
        }

        let tcs: Vec<ToolCall> = tool_calls
            .into_values()
            .map(|(id, name, json_str)| ToolCall {
                id,
                name,
                input: serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null),
            })
            .collect();

        Ok(ChatResponse {
            text: text_out,
            tool_calls: tcs,
            stop_reason,
            tokens_in,
            tokens_out,
        })
    }
}
