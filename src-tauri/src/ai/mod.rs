//! AI 排障模块。详细约束见 docs/ai-diagnose-design.md。
//!
//! 子模块：
//! - sanitize  脱敏 + 截断 + 命令形态校验
//! - audit     内存审计 + 保存到文本
//! - llm       BYOK 客户端（Anthropic / OpenAI 兼容）
//! - tools     暴露给 LLM 的工具：run_command / load_skill / download_file /
//!             analyze_locally / match_file / patch_file
//! - session   AI 会话生命周期 + 对话循环（含远端 file_ops 脚本拼装：
//!             python3 / perl 两层降级，rssh 后端不再 cat 整文件回流）
//! - commands  Tauri 命令入口

pub mod audit;
pub mod commands;
pub mod llm;
pub mod prompts;
pub mod sanitize;
pub mod session;
pub mod skills;
pub mod tools;
