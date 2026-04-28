//! OpenAI Chat Completions 流式客户端（兼容 vLLM 等）。
//!
//! 协议：data: {...}\n\n ... data: [DONE]\n\n
//! delta.content 累积文本；delta.tool_calls[].function.arguments 累积工具参数。

use std::collections::BTreeMap;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;

use super::{ChatDelta, ChatMessage, ChatRequest, ChatResponse, DeltaSink, LlmClient, SseParser, ToolCall};
use crate::error::{AppError, AppResult};

const DEFAULT_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

pub struct OpenAiClient {
    api_key: String,
    endpoint: String,
    http: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(api_key: String, endpoint: Option<String>) -> Self {
        Self {
            api_key,
            endpoint: endpoint.unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct OaiReq<'a> {
    model: &'a str,
    max_completion_tokens: u32,
    messages: Vec<OaiMsg>,
    tools: Vec<serde_json::Value>,
    stream: bool,
    stream_options: serde_json::Value,
}

#[derive(Serialize)]
struct OaiMsg {
    role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct OaiToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: OaiToolCallFn,
}

#[derive(Serialize)]
struct OaiToolCallFn {
    name: String,
    arguments: String,
}

#[async_trait]
impl LlmClient for OpenAiClient {
    fn provider(&self) -> &'static str {
        "openai"
    }

    async fn chat(&self, req: ChatRequest, sink: DeltaSink) -> AppResult<ChatResponse> {
        let mut messages: Vec<OaiMsg> = Vec::with_capacity(req.messages.len() + 1);
        messages.push(OaiMsg {
            role: "system",
            content: Some(req.system_prompt.clone()),
            tool_calls: None,
            tool_call_id: None,
        });
        for m in &req.messages {
            match m {
                ChatMessage::User { content } => messages.push(OaiMsg {
                    role: "user",
                    content: Some(content.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                }),
                ChatMessage::Assistant { content, tool_calls } => {
                    let oai_calls: Vec<OaiToolCall> = tool_calls
                        .iter()
                        .map(|tc| OaiToolCall {
                            id: tc.id.clone(),
                            kind: "function",
                            function: OaiToolCallFn {
                                name: tc.name.clone(),
                                arguments: serde_json::to_string(&tc.input).unwrap_or_default(),
                            },
                        })
                        .collect();
                    messages.push(OaiMsg {
                        role: "assistant",
                        content: if content.is_empty() { None } else { Some(content.clone()) },
                        tool_calls: if oai_calls.is_empty() { None } else { Some(oai_calls) },
                        tool_call_id: None,
                    });
                }
                ChatMessage::ToolResult { tool_call_id, content, is_error } => {
                    let body = if *is_error { format!("[ERROR] {content}") } else { content.clone() };
                    messages.push(OaiMsg {
                        role: "tool",
                        content: Some(body),
                        tool_calls: None,
                        tool_call_id: Some(tool_call_id.clone()),
                    });
                }
            }
        }

        let tools: Vec<serde_json::Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();

        let body = OaiReq {
            model: &req.model,
            max_completion_tokens: req.max_tokens,
            messages,
            tools,
            stream: true,
            stream_options: json!({ "include_usage": true }),
        };

        let resp = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
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

        let mut text_out = String::new();
        // index → (id, name, args_partial)
        let mut tool_calls: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
        let mut finish_reason = String::new();
        let mut tokens_in: Option<u32> = None;
        let mut tokens_out: Option<u32> = None;

        let mut parser = SseParser::new();
        let mut stream = resp.bytes_stream();
        'stream: while let Some(chunk) = stream.next().await {
            let bytes = chunk
                .map_err(|e| AppError::Other(format!("LLM stream 读失败: {e}")))?;
            let s = String::from_utf8_lossy(&bytes).into_owned();
            for ev_data in parser.feed(&s) {
                if ev_data.trim() == "[DONE]" {
                    break 'stream;
                }
                let v: serde_json::Value = match serde_json::from_str(&ev_data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(usage) = v.get("usage") {
                    tokens_in = usage["prompt_tokens"].as_u64().map(|n| n as u32);
                    tokens_out = usage["completion_tokens"].as_u64().map(|n| n as u32);
                }
                if let Some(choice) = v["choices"].get(0) {
                    if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                        finish_reason = reason.to_string();
                    }
                    let delta = &choice["delta"];
                    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                        if !content.is_empty() {
                            text_out.push_str(content);
                            sink(ChatDelta::Text(content.to_string()));
                        }
                    }
                    if let Some(tcs_arr) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                        for tc in tcs_arr {
                            let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                            let entry = tool_calls
                                .entry(idx)
                                .or_insert_with(|| (String::new(), String::new(), String::new()));
                            if let Some(id) = tc.get("id").and_then(|s| s.as_str()) {
                                if entry.0.is_empty() && !id.is_empty() {
                                    entry.0 = id.to_string();
                                }
                            }
                            if let Some(name) =
                                tc["function"].get("name").and_then(|s| s.as_str())
                            {
                                if entry.1.is_empty() && !name.is_empty() {
                                    entry.1 = name.to_string();
                                    sink(ChatDelta::ToolStart {
                                        tool_call_id: entry.0.clone(),
                                        name: entry.1.clone(),
                                    });
                                }
                            }
                            if let Some(args) =
                                tc["function"].get("arguments").and_then(|a| a.as_str())
                            {
                                if !args.is_empty() {
                                    entry.2.push_str(args);
                                    sink(ChatDelta::ToolArgs {
                                        tool_call_id: entry.0.clone(),
                                        partial: args.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        let tcs: Vec<ToolCall> = tool_calls
            .into_values()
            .map(|(id, name, args)| ToolCall {
                id,
                name,
                input: serde_json::from_str(&args).unwrap_or(serde_json::Value::Null),
            })
            .collect();

        Ok(ChatResponse {
            text: text_out,
            tool_calls: tcs,
            stop_reason: finish_reason,
            tokens_in,
            tokens_out,
        })
    }
}
