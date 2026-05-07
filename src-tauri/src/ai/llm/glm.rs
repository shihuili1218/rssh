//! 智谱 BigModel（GLM 系列）。OpenAI 兼容协议，端点不同，且**不开放** `/models` 列表，
//! 所以 `list_models` 走硬编码白名单。
//!
//! 端点：https://open.bigmodel.cn/api/paas/v4
//! 模型：
//!   - glm-4.6        —— 旗舰，长上下文 + 工具调用
//!   - glm-4-plus     —— 稳定版旗舰
//!   - glm-4-air      —— 轻量
//!   - glm-4-airx     —— 轻量增强
//!   - glm-4-flash    —— 高性价比
//!   - glm-4-long     —— 长文本（1M context）
//!
//! 文档：https://bigmodel.cn/dev/api

use async_trait::async_trait;

use super::protocol;
use super::{ChatRequest, ChatResponse, DeltaSink, LlmClient, ModelInfo};
use crate::error::AppResult;

const DEFAULT_BASE: &str = "https://open.bigmodel.cn/api/paas/v4";

const KNOWN_MODELS: &[&str] = &[
    "glm-4.6",
    "glm-4-plus",
    "glm-4-air",
    "glm-4-airx",
    "glm-4-flash",
    "glm-4-long",
];

pub struct GlmClient {
    api_key: String,
    chat_endpoint: String,
    http: reqwest::Client,
}

impl GlmClient {
    pub fn new(api_key: String, endpoint: Option<String>) -> Self {
        Self {
            api_key,
            chat_endpoint: protocol::resolve_chat_endpoint(endpoint, DEFAULT_BASE),
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl LlmClient for GlmClient {
    fn provider(&self) -> &'static str {
        "glm"
    }

    async fn chat(&self, req: ChatRequest, sink: DeltaSink) -> AppResult<ChatResponse> {
        protocol::chat(&self.http, &self.chat_endpoint, &self.api_key, req, sink).await
    }

    /// 智谱不公开 `/models`，返回硬编码列表。维护：发现新模型时更新 KNOWN_MODELS。
    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(KNOWN_MODELS
            .iter()
            .map(|id| ModelInfo {
                id: (*id).to_string(),
                display_name: None,
            })
            .collect())
    }
}
