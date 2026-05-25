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
pub const TOOL_MATCH_FILE: &str = "match_file";
pub const TOOL_PATCH_FILE: &str = "patch_file";

/// match_file / patch_file 上下文字符数上限。够 LLM 判断位置又不浪费 token。
pub const MATCH_CONTEXT_DEFAULT: u32 = 80;
pub const MATCH_CONTEXT_MAX: u32 = 400;

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
                The command is inserted into the user's interactive terminal; the user inspects it and runs it themselves (or rejects). \
                The output is sanitized locally before being returned to you. \
                Do not propose commands that mutate system state (delete/format/signal/firewall/mount/shutdown/recursive chmod/etc.); \
                do not use screen-redrawing commands (top / htop / watch / tail -f); \
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
            description: "SFTP a remote file to the local machine (under the app's data dir / diagnose / <session>/) for local analysis. \
                Typically used for heap dump, core dump, pprof profile, perf.data, etc. \
                **rssh hard-caps this tool at 100 MB.** `max_mb` must be 1..=100; requests above 100 are rejected and the transfer also aborts if the remote file turns out to be larger than 100 MB. \
                Always `ls -l` the remote file first. If it's >100 MB, **don't call this tool** — ask the user to scp/rsync/sz the file themselves, then call `analyze_locally` on the local path they paste back.".into(),
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
                        "maximum": 100,
                        "description": "Maximum file size you expect (MB). Must be between 1 and 100 — rssh caps downloads at 100 MB; larger artifacts must be transferred manually by the user.",
                    }
                },
                "required": ["remote_path", "max_mb"],
            }),
        },
        ToolSchema {
            name: TOOL_MATCH_FILE.into(),
            description: "Locate every occurrence of a literal text inside a remote file (read-only; one approval card — may be auto-approved per user settings: auto_match_file under danger_mode). \
                Always call this **before** `patch_file` to: (1) confirm the find string actually exists and how many times; \
                (2) verify the surrounding context matches the locations you want to change; \
                (3) obtain `expected_count` for the follow-up `patch_file` call. \
                The find string is matched literally (no regex). Multi-line `find` is supported — embed real newlines. \
                Returns JSON: { count, matches: [{ line, context }] }. `context` is `before` chars + find + `after` chars (clamped to file boundaries).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Remote file path. Absolute (`/etc/foo`) or home-relative (`~` / `~/foo`) — rssh expands `~` and `~/` via `$HOME` before sending. `~user/...` (other users' home) is **not** supported.",
                    },
                    "find": {
                        "type": "string",
                        "description": "Literal text to search for. Newlines are honored verbatim. Empty string is rejected.",
                    },
                    "before": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": MATCH_CONTEXT_MAX,
                        "description": "Chars of context before each match (default 80, max 400).",
                    },
                    "after": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": MATCH_CONTEXT_MAX,
                        "description": "Chars of context after each match (default 80, max 400).",
                    },
                },
                "required": ["path", "find"],
            }),
        },
        ToolSchema {
            name: TOOL_PATCH_FILE.into(),
            description: "Modify a remote file by replacing every occurrence of `find` with `replace`. \
                This is the **only allowed way** to change file contents — direct shell writes (>, tee, cp, mv, sed -i, awk -i, perl -i, python, etc.) are blocked. \
                You MUST call `match_file` first to verify the find string exists and to obtain `expected_count`. \
                rssh re-reads the file just before patching and refuses the change if the count differs (race-condition / staleness guard). \
                The new file content is written atomically (tmp + mv). Returns a unified diff for your final review. \
                The flow has 4 approval cards (cp → modify → diff → mv); each card may be auto-approved independently per user settings (auto_patch_cp/_modify/_diff/_mv under danger_mode).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Remote file path. Absolute (`/etc/foo`) or home-relative (`~` / `~/foo`) — rssh expands `~` and `~/` via `$HOME` before sending. `~user/...` (other users' home) is **not** supported.",
                    },
                    "find": {
                        "type": "string",
                        "description": "Literal text to replace. Must match exactly `expected_count` times in the current file.",
                    },
                    "replace": {
                        "type": "string",
                        "description": "Replacement text. Set to empty string to delete the matched section(s).",
                    },
                    "expected_count": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Number of occurrences you expect to replace, obtained from a prior `match_file` call. If the actual count differs (e.g. file changed between calls), the patch is refused — re-run match_file and retry.",
                    },
                },
                "required": ["path", "find", "replace", "expected_count"],
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

#[derive(Debug, Deserialize)]
pub struct MatchFileInput {
    pub path: String,
    pub find: String,
    pub before: Option<u32>,
    pub after: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct PatchFileInput {
    pub path: String,
    pub find: String,
    pub replace: String,
    pub expected_count: u32,
}
