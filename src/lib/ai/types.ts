/**
 * AI 排障模块类型，镜像 Rust 侧 src-tauri/src/ai/。
 */

export interface SkillRecord {
  id: string;
  name: string;
  description: string;
  content: string;
  builtin: boolean;
}

export type LlmProvider = "anthropic" | "openai";

export interface AiSettings {
  provider: LlmProvider;
  model: string;
  endpoint: string | null;
  has_api_key: boolean;
}

export interface AiSessionInfo {
  session_id: string;
  target_id: string;
  skill: string;
  model: string;
  provider: LlmProvider;
}

/** 一条对话消息（前端展示用） */
export type ChatItem =
  | { kind: "user"; text: string; at: number }
  | { kind: "assistant"; id: string; text: string; at: number; streaming: boolean }
  | { kind: "command"; cmd: CommandProposed; at: number; result?: CommandResult; rejected?: { reason: string } }
  | { kind: "error"; text: string; at: number }
  | { kind: "note"; text: string; at: number };

export interface CommandProposed {
  id: string;
  tool_call_id: string;
  cmd: string;
  /** 实际要粘贴到终端的命令（含 sentinel + exit code 回显），由后端拼装。 */
  full_cmd: string;
  /** 用于在 PTY 输出流里识别命令完成的随机字符串。 */
  sentinel: string;
  explain: string;
  side_effect: string;
  timeout_s: number;
}

export interface CommandResult {
  id: string;
  exit_code: number;
  timed_out: boolean;
  duration_ms: number;
  output: string;
  original_bytes: number;
  truncated_bytes: number;
}

/** 审计日志（来自后端 ai_audit_get） */
export interface AuditLog {
  entries: AuditEntry[];
}
export interface AuditEntry {
  at: string; // ISO 8601
  kind: AuditKind;
}
export type AuditKind =
  | { type: "session_started"; skill: string; target: string }
  | { type: "session_ended" }
  | { type: "llm_request"; model: string; redacted_payload: string }
  | { type: "llm_response"; text: string; tokens_in: number | null; tokens_out: number | null }
  | { type: "command_proposed"; id: string; cmd: string; explain: string; side_effect: string }
  | { type: "command_rejected"; id: string; reason: string }
  | { type: "command_executed"; id: string; exit_code: number; output_redacted: string; original_bytes: number; truncated_bytes: number; duration_ms: number }
  | { type: "download_proposed"; id: string; remote_path: string; max_mb: number }
  | { type: "download_completed"; id: string; local_path: string; bytes: number }
  | { type: "note"; message: string }
  | { type: "error"; message: string };

export type AiPanelPosition = "left" | "right";
