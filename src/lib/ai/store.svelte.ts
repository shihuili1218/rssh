/**
 * AI 排障会话前端状态。
 * - 一个目标（ssh/local tab）至多一个 AI 会话；store 保留所有会话按 target_id 索引
 * - 监听 ai:* 事件填充 chat 时间线
 * - keyboard lock 在 AI 命令执行期间生效
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { t, locale as currentLocale } from "../i18n/index.svelte.ts";
import type {
  AiSessionInfo,
  AiSettings,
  AuditLog,
  ChatItem,
  CommandProposed,
  CommandResult,
  LlmProvider,
  ModelInfo,
  SkillRecord,
} from "./types.ts";

// ─── Position ────────────────────────────────────────────────────
// 只支持 left/right。移动端用户横屏即可用——左右布局就够了，没必要再开上下分支。

export type AiPosition = "left" | "right";

const POS_KEY = "ai_panel_position";
function loadPos(): AiPosition {
  const v = localStorage.getItem(POS_KEY);
  return v === "left" || v === "right" ? v : "right";
}

// ─── 全局可见状态 ─────────────────────────────────────────────────

let _open = $state(false);
let _position = $state<AiPosition>(loadPos());
let _activeSessionId = $state<string | null>(null);
let _sessionByTarget = $state<Record<string, AiSessionInfo>>({});
let _chatBySession = $state<Record<string, ChatItem[]>>({});
let _pendingByTarget = $state<Record<string, CommandProposed | null>>({});
let _keyboardLockedByTarget = $state<Record<string, boolean>>({});
let _settings = $state<AiSettings | null>(null);

const _unlisteners: Record<string, UnlistenFn[]> = {};

export function position() { return _position; }
export function setPosition(p: AiPosition) {
  _position = p;
  localStorage.setItem(POS_KEY, p);
}

// ─── Open/close ───────────────────────────────────────────────────

export function isOpen() { return _open; }
export function openPanel() { _open = true; }
export function closePanel() { _open = false; }
export function togglePanel() { _open = !_open; }

// ─── Session ──────────────────────────────────────────────────────

export function activeSessionId() { return _activeSessionId; }
export function activeSession(): AiSessionInfo | null {
  if (!_activeSessionId) return null;
  return Object.values(_sessionByTarget).find(s => s.session_id === _activeSessionId) ?? null;
}
export function sessionForTarget(target_id: string): AiSessionInfo | undefined {
  return _sessionByTarget[target_id];
}
export function listAllSessions(): AiSessionInfo[] {
  return Object.values(_sessionByTarget);
}

export function chatItems(session_id: string): ChatItem[] {
  return _chatBySession[session_id] ?? [];
}
export function pendingCommand(target_id: string): CommandProposed | null {
  return _pendingByTarget[target_id] ?? null;
}
export function isKeyboardLocked(target_id: string): boolean {
  return _keyboardLockedByTarget[target_id] === true;
}

function pushChat(session_id: string, item: ChatItem) {
  const arr = _chatBySession[session_id] ?? [];
  _chatBySession[session_id] = [...arr, item];
}

// ─── Lifecycle ────────────────────────────────────────────────────

export async function startSession(args: {
  targetKind: "ssh" | "local";
  targetId: string;
  skill: string;
  provider: string;
  model: string;
}): Promise<AiSessionInfo> {
  const info = await invoke<AiSessionInfo>("ai_session_start", {
    targetKind: args.targetKind,
    targetId: args.targetId,
    skill: args.skill,
    provider: args.provider,
    model: args.model,
    locale: currentLocale(),
  });
  _sessionByTarget[args.targetId] = info;
  _chatBySession[info.session_id] = [];
  _activeSessionId = info.session_id;
  await attachListeners(info);
  return info;
}

export async function stopSession(session_id: string) {
  await invoke("ai_session_stop", { sessionId: session_id });
  detachListeners(session_id);
  // 清掉对应的 target 索引
  for (const [tid, info] of Object.entries(_sessionByTarget)) {
    if (info.session_id === session_id) {
      delete _sessionByTarget[tid];
      delete _pendingByTarget[tid];
      delete _keyboardLockedByTarget[tid];
    }
  }
  delete _chatBySession[session_id];
  if (_activeSessionId === session_id) _activeSessionId = null;
}

export async function sendMessage(session_id: string, text: string) {
  await invoke("ai_user_message", { sessionId: session_id, text });
}

/** 打断 actor 正在跑的 LLM 流式响应。会话上下文（history / pending command / audit）全部保留——
 *  这跟 stopSession（销毁整个会话）是两个语义。actor 不在 chat 时调用是 no-op。 */
export async function cancelStream(session_id: string): Promise<void> {
  await invoke("ai_cancel_stream", { sessionId: session_id });
}

/** 当前会话的助手消息是否正在流式输出 —— UI 用它把"发送"按钮切成"停止"。 */
export function isStreaming(session_id: string): boolean {
  const arr = _chatBySession[session_id];
  if (!arr || arr.length === 0) return false;
  const last = arr[arr.length - 1];
  return last.kind === "assistant" && last.streaming === true;
}

/** 在途执行的控制句柄；按 tool_call_id 索引。`terminate` 给 UI 上的"提前终止"按钮用。 */
const _runningExecutions: Record<string, { terminate: () => Promise<void> }> = {};

export function isCommandRunning(tool_call_id: string): boolean {
  return tool_call_id in _runningExecutions;
}

/**
 * 执行 AI 提议的命令：把 `full_cmd`（含 sentinel + exit code 回显）粘到 active terminal
 * 自动回车，监听输出流找 sentinel 拿 exit code，然后把脱敏前的 output 上报后端。
 *
 * 全部在前端完成；后端的 ai 模块不直接执行任何命令。
 */
export async function executeCommand(
  session_id: string,
  proposed: CommandProposed,
  target_kind: "ssh" | "local",
  target_session_id: string,
): Promise<void> {
  const writeCmd = target_kind === "ssh" ? "ssh_write" : "pty_write";
  const dataEvent = target_kind === "ssh"
    ? `ssh:data:${target_session_id}`
    : `pty:data:${target_session_id}`;

  let buffer = "";
  let resolved = false;
  let userInterrupted = false;
  let unlisten: UnlistenFn | null = null;
  let timer: number | null = null;
  // 整个函数返回的 Promise 只在 finish() 真正跑完才 resolve——UI 上的 executing
  // 状态因此能持续覆盖整个执行周期，否则之前 await invoke(writeCmd) 一返回
  // executing 就被翻回 false，按钮立刻又能点。
  let resolveDone!: () => void;
  const done = new Promise<void>((r) => { resolveDone = r; });

  const sentinelRegex = new RegExp(
    proposed.sentinel.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + ":(-?\\d+)"
  );

  const stripAnsi = (s: string) =>
    s.replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "")
     .replace(/\x1b\][^\x07]*\x07/g, "")
     .replace(/\r/g, "");

  /** 从 PTY buffer 抽出真正给 LLM 看的 output：
   *  1. 截到 endIndex（sentinel 路径传 sentinel 行起点；terminate/timeout 传整段）
   *  2. strip ANSI / OSC / CR
   *  3. 去掉首行（PTY echo 的命令本身——shell 一定会把粘过去的命令回显一遍）
   *  4. trimEnd（不是 trim——保留前导空白，避免吃掉 `  indented output` 的对齐）
   *  三条 finish 路径共用此函数，保证上报给 LLM 的格式形态完全一致。 */
  const extractOutput = (rawBuffer: string, endIndex?: number): string => {
    const end = Math.max(0, endIndex ?? rawBuffer.length);
    const stripped = stripAnsi(rawBuffer.substring(0, end));
    const firstNl = stripped.indexOf("\n");
    const out = firstNl >= 0 ? stripped.substring(firstNl + 1) : stripped;
    return out.trimEnd();
  };

  const finish = async (output: string, exit_code: number, timed_out: boolean) => {
    if (resolved) return;
    resolved = true;
    if (unlisten) unlisten();
    if (timer != null) clearTimeout(timer);
    delete _runningExecutions[proposed.tool_call_id];
    try {
      await invoke("ai_command_result", {
        sessionId: session_id,
        toolCallId: proposed.tool_call_id,
        exitCode: exit_code,
        output,
        timedOut: timed_out,
        earlyTerminated: userInterrupted,
      });
    } catch (e) {
      console.error("[ai] ai_command_result failed:", e);
    }
    resolveDone();
  };

  unlisten = await listen<number[]>(dataEvent, (e) => {
    if (resolved) return;
    const chunk = new TextDecoder("utf-8", { fatal: false }).decode(new Uint8Array(e.payload));
    buffer += chunk;
    const m = sentinelRegex.exec(buffer);
    if (m) {
      const exit = parseInt(m[1], 10);
      // sentinel 行之前的部分 = echo 行 + 实际输出
      const sentinelLineStart = buffer.lastIndexOf("\n", m.index);
      void finish(extractOutput(buffer, sentinelLineStart), exit, false);
    }
  });

  // "提前终止"：用户的诉求是"立刻让我走"，不是"帮我等一个漂亮的退出码"。
  // Ctrl+C fire-and-forget——shell 能响应就跟着停，不能响应（cat 等 stdin、
  // 密码 prompt 等吞 SIGINT 的场景）也不扣留用户。立刻 finish，上报
  // early_terminated=true，LLM 据此知道不该自动重试。
  _runningExecutions[proposed.tool_call_id] = {
    terminate: async () => {
      if (resolved) return;
      userInterrupted = true;
      const ctrlC = Array.from(new TextEncoder().encode("\x03"));
      // fire-and-forget 但不要完全吞错——PTY 已关 / session 失联 时 invoke 会 reject，
      // 留个 warn 痕迹方便排错"我点了终止但 Ctrl+C 好像没发出去"这类反馈。
      void invoke(writeCmd, { sessionId: target_session_id, data: ctrlC })
          .catch((err) => console.warn("[ai] terminate Ctrl+C failed:", err));
      await finish(extractOutput(buffer), -1, false);
    },
  };

  // 写命令到 PTY；末尾 \n 触发 shell 执行。
  // 如果 invoke 抛错（session 已关闭等），listener / _runningExecutions 已经登记，
  // 必须走 finish() 清理一遍，否则会泄漏并让 isCommandRunning() 永远卡 true。
  const data = Array.from(new TextEncoder().encode(proposed.full_cmd + "\n"));
  try {
    await invoke(writeCmd, { sessionId: target_session_id, data });
  } catch (e) {
    await finish(`failed to write command: ${e instanceof Error ? e.message : String(e)}`, -1, false);
    throw e;
  }

  timer = window.setTimeout(() => {
    void finish(extractOutput(buffer), -1, true);
  }, Math.max(1000, proposed.timeout_s * 1000)) as unknown as number;

  return done;
}

/** 提前终止：发 Ctrl+C 到目标终端。finish() 之后的上报会带 early_terminated=true。 */
export async function terminateCommand(tool_call_id: string): Promise<void> {
  const ctl = _runningExecutions[tool_call_id];
  if (ctl) await ctl.terminate();
}

export async function rejectCommand(session_id: string, tool_call_id: string, reason: string) {
  await invoke("ai_command_reject", { sessionId: session_id, toolCallId: tool_call_id, reason });
}

export async function getAudit(session_id: string): Promise<AuditLog> {
  return invoke<AuditLog>("ai_audit_get", { sessionId: session_id });
}

export async function saveAudit(session_id: string, file_path: string) {
  return invoke("ai_audit_save", { sessionId: session_id, filePath: file_path });
}

/** Desktop-only：弹原生 Save 对话框选路径并保存。返回路径或 null（用户取消）。 */
export async function saveAuditWithDialog(session_id: string): Promise<string | null> {
  return invoke<string | null>("ai_audit_save_pick", { sessionId: session_id });
}

// ─── Settings ─────────────────────────────────────────────────────

export function settings() { return _settings; }
/**
 * provider 为空 → 拉 active provider 的快照，**更新**全局 `_settings`（ChatPanel 起 session 读它）；
 * provider 非空 → 仅返回该 provider 的快照，**不动**全局缓存（避免设置页切下拉污染聊天）。
 */
export async function loadSettings(provider?: LlmProvider): Promise<AiSettings> {
  const snapshot = await invoke<AiSettings>("ai_settings_get", { provider: provider || null });
  if (!provider) _settings = snapshot;
  return snapshot;
}
export async function saveSettings(s: Partial<{ provider: string; model: string; endpoint: string | null; apiKey: string | null; dangerMode: boolean }>) {
  await invoke("ai_settings_set", s);
  await loadSettings();
}

/**
 * 拉取指定 provider 的模型列表。
 * apiKey/endpoint 为空时后端从 secret_store 取已保存值。
 * GLM 没有公开 /models，会返回硬编码白名单。
 */
export async function listModels(
  provider: LlmProvider,
  apiKey?: string,
  endpoint?: string,
): Promise<ModelInfo[]> {
  return invoke<ModelInfo[]>("ai_list_models", {
    provider,
    apiKey: apiKey || null,
    endpoint: endpoint || null,
  });
}

// ─── 事件监听 ─────────────────────────────────────────────────────

async function attachListeners(info: AiSessionInfo) {
  const sid = info.session_id;
  const tid = info.target_id;
  const u: UnlistenFn[] = [];

  u.push(await listen<{ text: string }>(`ai:user_message:${sid}`, (e) => {
    pushChat(sid, { kind: "user", text: e.payload.text, at: Date.now() });
  }));

  // 流式：start 创建空气泡，delta append，end 关 streaming 标记
  u.push(await listen<{ id: string }>(`ai:assistant_message_start:${sid}`, (e) => {
    pushChat(sid, { kind: "assistant", id: e.payload.id, text: "", at: Date.now(), streaming: true });
  }));

  u.push(await listen<{ id: string; text: string }>(`ai:assistant_delta:${sid}`, (e) => {
    const arr = _chatBySession[sid] ?? [];
    for (let i = arr.length - 1; i >= 0; i--) {
      const item = arr[i];
      if (item.kind === "assistant" && item.id === e.payload.id) {
        const replaced: ChatItem = { ...item, text: item.text + e.payload.text };
        _chatBySession[sid] = [...arr.slice(0, i), replaced, ...arr.slice(i + 1)];
        return;
      }
    }
  }));

  u.push(await listen<{ id: string; text: string; cancelled?: boolean }>(`ai:assistant_message_end:${sid}`, (e) => {
    const arr = _chatBySession[sid] ?? [];
    for (let i = arr.length - 1; i >= 0; i--) {
      const item = arr[i];
      if (item.kind === "assistant" && item.id === e.payload.id) {
        const isEmpty = !e.payload.text || e.payload.text.length === 0;
        // cancelled=true 时即使 text 空也要保留气泡——UI 模板会渲染本地化的
        // "已停止"徽章，告诉用户这一轮被自己打断了。
        // 只有"纯 tool_use 轮次"（chat 没产文本只产 tool_calls，cancelled=false）
        // 或 chat 失败（empty + cancelled=false）才移除气泡。
        if (isEmpty && !e.payload.cancelled) {
          _chatBySession[sid] = [...arr.slice(0, i), ...arr.slice(i + 1)];
        } else {
          // 防御：cancel emit 的 payload.text = 后端 captured（sink 累积）；前端 item.text =
          // 收到的 delta 累积。两者源头一致，正常情况下相等。但 tauri 事件总线异步——
          // cancel emit 抵达时若 in-flight delta 尚未处理完，payload 反而可能比 item.text
          // 短；极端退化时甚至为空（chat 刚 start 就 cancel）。用 item.text 兜底，
          // 避免"用户看着字一行行出来，按停止后只剩个徽章"。
          const finalText = e.payload.text || item.text;
          const replaced: ChatItem = {
            ...item,
            text: finalText,
            streaming: false,
            cancelled: e.payload.cancelled === true,
          };
          _chatBySession[sid] = [...arr.slice(0, i), replaced, ...arr.slice(i + 1)];
        }
        return;
      }
    }
  }));

  u.push(await listen<CommandProposed>(`ai:command_proposed:${sid}`, (e) => {
    _pendingByTarget[tid] = e.payload;
    pushChat(sid, { kind: "command", cmd: e.payload, at: Date.now() });
  }));

  u.push(await listen<{ id: string; lock_keyboard: boolean }>(`ai:command_executing:${sid}`, (e) => {
    _keyboardLockedByTarget[tid] = !!e.payload.lock_keyboard;
  }));

  u.push(await listen<CommandResult & { lock_keyboard: boolean }>(`ai:command_completed:${sid}`, (e) => {
    _keyboardLockedByTarget[tid] = !!e.payload.lock_keyboard;
    _pendingByTarget[tid] = null;
    // 给最近一条对应 id 的 command 项填上 result
    const arr = _chatBySession[sid] ?? [];
    for (let i = arr.length - 1; i >= 0; i--) {
      const item = arr[i];
      if (item.kind === "command" && item.cmd.id === e.payload.id) {
        const replaced: ChatItem = { ...item, result: e.payload };
        _chatBySession[sid] = [...arr.slice(0, i), replaced, ...arr.slice(i + 1)];
        break;
      }
    }
  }));

  u.push(await listen<{ message: string }>(`ai:error:${sid}`, (e) => {
    pushChat(sid, { kind: "error", text: e.payload.message, at: Date.now() });
  }));

  u.push(await listen<{}>(`ai:session_ended:${sid}`, () => {
    pushChat(sid, { kind: "note", text: t("ai.session.ended_note"), at: Date.now() });
  }));

  _unlisteners[sid] = u;
}

function detachListeners(sid: string) {
  const arr = _unlisteners[sid];
  if (arr) {
    arr.forEach(fn => fn());
    delete _unlisteners[sid];
  }
}

// ─── Skill 管理 ────────────────────────────────────────────────────

export async function listSkills(): Promise<SkillRecord[]> {
  return invoke<SkillRecord[]>("ai_list_skills");
}

export async function getSkill(id: string): Promise<SkillRecord | null> {
  return invoke<SkillRecord | null>("ai_get_skill", { id });
}

export async function saveSkill(s: { id: string; name: string; description: string; content: string }): Promise<void> {
  return invoke("ai_save_skill", s);
}

export async function deleteSkill(id: string): Promise<void> {
  return invoke("ai_delete_skill", { id });
}
