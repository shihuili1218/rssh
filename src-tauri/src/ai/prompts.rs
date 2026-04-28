//! 内置 skill 的 prompt 内容 —— 编译时 `include_str!` 内嵌进二进制。
//! 现在只有 `general` 一个内置 skill；它是规则集 + 工作流参考的合并体，
//! LLM 直接基于它判断场景并挑命令。

pub const GENERAL: &str = include_str!("prompts/general.md");
