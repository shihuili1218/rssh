//! 暴露给 LLM 的 3 个工具的 schema。
//!
//! 决议 #6：shape validator 失败时把错误回给 LLM 让它重提（最多 2 次）。
//! 工具的实际执行（运行命令、SFTP 下载、本地分析）在 session.rs 的循环里串接。

use serde::Deserialize;
use serde_json::json;

use super::llm::ToolSchema;

pub const TOOL_RUN_COMMAND: &str = "run_command";
pub const TOOL_LOAD_SKILL: &str = "load_skill";
pub const TOOL_DOWNLOAD_FILE: &str = "download_file";
pub const TOOL_ANALYZE_LOCALLY: &str = "analyze_locally";

pub fn all_tools() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: TOOL_LOAD_SKILL.into(),
            description: "加载某个 skill 的完整工作流文档。判断用户问题对应的 skill id 后第一时间调用，再按返回的工作流办事。可用 id 见 system prompt 的 skill 目录。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "skill id，如 cpu-java / mem-go / general / user-xxxx",
                    }
                },
                "required": ["id"],
            }),
        },
        ToolSchema {
            name: TOOL_RUN_COMMAND.into(),
            description: "在远端（或本地 shell）执行一条命令。\
                每条命令都会先经用户点击确认才执行。\
                输出会被本地脱敏后回传给你。\
                不要 propose 破坏性命令；不要用刷屏命令（top/htop/watch/tail -f）；\
                重复采样必须显式带次数（vmstat 1 5 而非 vmstat 1）。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "cmd": {
                        "type": "string",
                        "description": "完整命令行（含参数）。可以包含管道。",
                    },
                    "explain": {
                        "type": "string",
                        "description": "用人话告诉用户这条命令是干什么的（一句话）。",
                    },
                    "side_effect": {
                        "type": "string",
                        "description": "副作用说明（如 '会触发 Full GC，业务停顿 100-300ms'）。无副作用写 '只读'。",
                    },
                    "timeout_s": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 300,
                        "description": "超时秒数（默认 60，上限 300）。"
                    }
                },
                "required": ["cmd", "explain", "side_effect"],
            }),
        },
        ToolSchema {
            name: TOOL_DOWNLOAD_FILE.into(),
            description: "把远端文件 SFTP 下载到本地 (<app_data>/rssh/diagnose/<session>/) 用本地工具分析。\
                通常用于 heap dump、core dump、pprof profile、perf.data 等。\
                rssh 会先 ls -l 看大小，>1GB 必须用户二次确认。\
                文件路径必须是已经存在的远端绝对路径。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "remote_path": {
                        "type": "string",
                        "description": "远端绝对路径，如 /tmp/rssh-heap-1234.hprof",
                    },
                    "max_mb": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "你期望的最大文件大小（MB）。超过会让用户二次确认。",
                    }
                },
                "required": ["remote_path", "max_mb"],
            }),
        },
        ToolSchema {
            name: TOOL_ANALYZE_LOCALLY.into(),
            description: "在用户本地电脑上跑分析工具（MAT/jhat、go tool pprof、perf script 等），\
                输出被脱敏后回传。dump 二进制本身不会发给你；只有分析后的文本归因结果。\
                工具未安装时会引导用户安装并报告失败。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_path": {
                        "type": "string",
                        "description": "之前 download_file 拉下来的本地路径。",
                    },
                    "tool_hint": {
                        "type": "string",
                        "enum": ["pprof-top", "pprof-top-inuse", "mat-leak", "jhat-histo", "perf-folded"],
                        "description": "要用哪种本地工具分析。",
                    },
                },
                "required": ["local_path", "tool_hint"],
            }),
        },
    ]
}

// ─── 输入参数解析（LLM 给的 input json -> 结构化） ────────────────────

#[derive(Debug, Deserialize)]
pub struct RunCommandInput {
    pub cmd: String,
    pub explain: String,
    pub side_effect: String,
    pub timeout_s: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct DownloadFileInput {
    pub remote_path: String,
    pub max_mb: u32,
}

#[derive(Debug, Deserialize)]
pub struct AnalyzeLocallyInput {
    pub local_path: String,
    pub tool_hint: String,
}

#[derive(Debug, Deserialize)]
pub struct LoadSkillInput {
    pub id: String,
}
