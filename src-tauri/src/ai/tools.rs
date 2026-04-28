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
            description: "Load the full content of a user-defined skill. \
                The system prompt lists each user-skill's id + one-line description; \
                if the user's problem matches one, call this to pull the detailed workflow / rules, then follow it. \
                Built-in rules are already in the system prompt — don't try to load 'general'.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "User-skill id (e.g. user-xxxx). See the 'User-defined skills' catalog in the system prompt.",
                    }
                },
                "required": ["id"],
            }),
        },
        ToolSchema {
            name: TOOL_RUN_COMMAND.into(),
            description: "Run a single command on the remote (or local shell). \
                Every command requires a user-confirmation click before it runs. \
                The output is sanitized locally before being returned to you. \
                Do not propose destructive commands; do not use screen-redrawing commands (top / htop / watch / tail -f); \
                repeat sampling must carry an explicit count (vmstat 1 5, not vmstat 1).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "cmd": {
                        "type": "string",
                        "description": "Full command line including arguments. Pipes are allowed.",
                    },
                    "explain": {
                        "type": "string",
                        "description": "Plain-language one-liner telling the user what this command does.",
                    },
                    "side_effect": {
                        "type": "string",
                        "description": "Side-effect description (e.g. 'triggers Full GC, 100-300ms business pause'). Write 'read-only' if there is no side effect.",
                    },
                    "timeout_s": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 300,
                        "description": "Timeout in seconds (default 60, max 300)."
                    }
                },
                "required": ["cmd", "explain", "side_effect"],
            }),
        },
        ToolSchema {
            name: TOOL_DOWNLOAD_FILE.into(),
            description: "SFTP a remote file to the local machine (<app_data>/rssh/diagnose/<session>/) for local analysis. \
                Typically used for heap dump, core dump, pprof profile, perf.data, etc. \
                rssh checks the size first via ls -l; >1GB requires user re-confirmation. \
                The path must be an existing remote absolute path.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "remote_path": {
                        "type": "string",
                        "description": "Remote absolute path, e.g. /tmp/rssh-heap-1234.hprof",
                    },
                    "max_mb": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum file size you expect (MB). Larger sizes will prompt the user to re-confirm.",
                    }
                },
                "required": ["remote_path", "max_mb"],
            }),
        },
        ToolSchema {
            name: TOOL_ANALYZE_LOCALLY.into(),
            description: "Analyze a previously downloaded file (heap dump / pprof / perf.data, etc.) on the user's machine. \
                rssh will **open a new window** containing a local shell + a separate AI session, and auto-send the task description as the first message to that AI, which then drives the analysis with the user. \
                **This session will NOT receive the analysis result** — by design: remote diagnosis and local analysis are decoupled. The user can view progress in the new window; if you need the conclusion, ask the user to copy the key output back. \
                Only use this when running the analysis on the remote would compete for resources with the diagnosed process (a 4G+ heap dump under jhat eats another 4G+ and may crush an already memory-tight server).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_path": {
                        "type": "string",
                        "description": "Local absolute path previously fetched by download_file.",
                    },
                    "task": {
                        "type": "string",
                        "description": "One or two sentences telling the new window's AI what to do, e.g.: \"Analyze /var/.../heap.hprof — find top memory-leak suspects and report the top 10 objects by retained size.\"",
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
