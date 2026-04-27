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
            description: "在用户本机分析之前 download_file 拉下来的文件（heap dump / pprof / perf.data 等）。\
                rssh 会**新开一个窗口**，里面起一个本地 shell + 一个独立 AI 会话，把任务描述自动发给那边的 AI，由它和用户在新窗口里完成分析。\
                **本会话不会收到分析结果**——这是设计：远端排障和本地分析解耦。新窗口里的分析进展用户可以直接看；如需引用分析结论，让用户把关键输出复制过来。\
                只在远端工具会和被诊断进程抢资源（4G+ heap dump 上跑 jhat 会再吃 4G、几乎压垮已经内存吃紧的服务器）时才用。".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_path": {
                        "type": "string",
                        "description": "之前 download_file 拉下来的本地绝对路径。",
                    },
                    "task": {
                        "type": "string",
                        "description": "用一两句话告诉新窗口的 AI 要干什么，例如：\"分析 /var/.../heap.hprof，找内存泄漏 top suspects 并报告 retained size 排前 10 的对象\"。",
                    },
                },
                "required": ["local_path", "task"],
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
    pub task: String,
}

#[derive(Debug, Deserialize)]
pub struct LoadSkillInput {
    pub id: String,
}
