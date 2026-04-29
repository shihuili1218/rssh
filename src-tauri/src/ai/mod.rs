//! AI 排障模块。详细约束见 docs/ai-diagnose-design.md。
//!
//! 子模块：
//! - sanitize  脱敏 + 截断 + 命令形态校验
//! - audit     内存审计 + 保存到文本
//! - inspector OSC 7338 字节流拦截，识别 AI 注入命令的边界
//! - llm       BYOK 客户端（Anthropic / OpenAI 兼容）
//! - exec      命令注入到 PTY/SSH，等 OSC done
//! - tools     暴露给 LLM 的 3 个工具：run_command / download_file / analyze_locally
//! - session   AI 会话生命周期 + 对话循环
//! - commands  Tauri 命令入口

pub mod audit;
pub mod commands;
pub mod llm;
pub mod prompts;
pub mod sanitize;
pub mod session;
pub mod skills;
pub mod tools;
