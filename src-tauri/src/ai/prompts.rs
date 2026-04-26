//! 内置 skill 的 prompt 内容 —— 编译时 `include_str!` 内嵌进二进制。
//! `ai::skills::BUILTIN` 引用这些常量构造内置 skill 记录。

pub const CPU_JAVA: &str = include_str!("prompts/cpu-java.md");
pub const CPU_GO: &str = include_str!("prompts/cpu-go.md");
pub const MEM_JAVA: &str = include_str!("prompts/mem-java.md");
pub const MEM_GO: &str = include_str!("prompts/mem-go.md");
pub const GENERAL: &str = include_str!("prompts/general.md");
