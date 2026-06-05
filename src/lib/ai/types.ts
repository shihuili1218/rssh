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

/**
 * 脱敏规则。默认规则首次运行 seed 进 DB，之后与用户规则一视同仁（无 builtin 字段）。
 * 规则变更只对新会话生效。
 */
export interface RedactRuleRecord {
  id: string;
  /** 正则源串 */
  pattern: string;
  /** 命中后替换成的占位符 */
  replacement: string;
}

export type LlmProvider = "anthropic" | "openai" | "deepseek" | "glm";

export interface AiSettings {
  provider: LlmProvider;
  model: string;
  endpoint: string | null;
  has_api_key: boolean;
  /** 危险模式总闸。off 时下面 8 个 auto_* 视同 false（持久化保留，方便切回时复原）。 */
  danger_mode: boolean;
  /** per-tool 自动批准。仅当 danger_mode=true 时生效；UI 上 danger 关时整组禁用。 */
  auto_run_command: boolean;
  auto_match_file: boolean;
  auto_download_file: boolean;
  auto_analyze_locally: boolean;
  auto_patch_cp: boolean;
  auto_patch_modify: boolean;
  auto_patch_diff: boolean;
  auto_patch_mv: boolean;
  /** 远端 shell 自动探测：off 时远端假设 POSIX；on 时 AI panel 打开时发探针。默认 off。 */
  auto_detect_remote_shell: boolean;
}

/** AI 工具卡片 kind —— 后端 emit command_proposed 时打的 tag；前端按它查 auto_* 设置。 */
export type CommandKind =
  | "run_command"
  | "match_file"
  | "download_file"
  | "analyze_locally"
  | "patch_cp"
  | "patch_modify"
  | "patch_diff"
  | "patch_mv";

export interface ModelInfo {
  id: string;
  display_name: string | null;
}

export interface AiSessionInfo {
  /** Tab 身份。actor 跟 tab 同寿命；SSH 断了重连，前端用 tab_id 仍能找到同一个 actor。 */
  tab_id: string;
  /** 当前绑定的 SSH/PTY session_id。重连时由 rebindTarget 更新。 */
  target_id: string;
  skill: string;
  model: string;
  provider: LlmProvider;
}

/** 远端 shell 三家族 —— 跟 Rust 端 ShellKind 一对一镜像（lowercase wire format）。 */
export type ShellKind = "posix" | "cmd" | "powershell";

/** 一条对话消息（前端展示用） */
export type ChatItem =
  | { kind: "user"; text: string; at: number }
  | { kind: "assistant"; id: string; text: string; at: number; streaming: boolean; cancelled?: boolean }
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
  /**
   * 工具卡片类型 —— 前端按 kind 查 settings.auto_<kind> 决定是否自动批准。
   * 历史回放（旧 audit log 重渲染）可能没有 kind，按未知处理走人审。
   */
  kind?: CommandKind;
  /**
   * patch_file 第 4 张 mv 卡片携带的 diff 文本（来自第 3 张 diff 命令的输出）——
   * 让用户审批 mv 时直接在卡片上看到 diff，不用回滚翻第 3 张的 result 区域。
   * 其他卡片不带（undefined）。
   */
  diff?: string;
}

export interface CommandResult {
  id: string;
  exit_code: number;
  timed_out: boolean;
  /** 用户在执行中点了"提前终止"。 */
  early_terminated?: boolean;
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
