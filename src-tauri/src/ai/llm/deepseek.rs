//! DeepSeek（深度求索）。完全 OpenAI 兼容，只是端点和模型不同。
//!
//! 端点：https://api.deepseek.com/v1
//! 模型（示例，实际以 list_models 拉取为准）：
//!   - deepseek-chat       —— 通用对话
//!   - deepseek-reasoner   —— 带思维链的推理模型
//!
//! 文档：https://api-docs.deepseek.com/

use async_trait::async_trait;

use super::protocol;
use super::{ChatRequest, ChatResponse, DeltaSink, LlmClient, ModelInfo};
use crate::error::AppResult;

const DEFAULT_BASE: &str = "https://api.deepseek.com/v1";

pub struct DeepSeekClient {
    api_key: String,
    chat_endpoint: String,
    http: reqwest::Client,
}

impl DeepSeekClient {
    pub fn new(api_key: String, endpoint: Option<String>) -> Self {
        Self {
            api_key,
            chat_endpoint: protocol::resolve_chat_endpoint(endpoint, DEFAULT_BASE),
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl LlmClient for DeepSeekClient {
    fn provider(&self) -> &'static str {
        "deepseek"
    }

    async fn chat(&self, req: ChatRequest, sink: DeltaSink) -> AppResult<ChatResponse> {
        protocol::chat(&self.http, &self.chat_endpoint, &self.api_key, req, sink).await
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        let url = protocol::models_endpoint_from_chat(&self.chat_endpoint);
        protocol::list_models(&self.http, &url, &self.api_key).await
    }
}
