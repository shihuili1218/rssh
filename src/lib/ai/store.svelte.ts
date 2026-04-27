/**
 * AI 排障会话前端状态。
 * - 一个目标（ssh/local tab）至多一个 AI 会话；store 保留所有会话按 target_id 索引
 * - 监听 ai:* 事件填充 chat 时间线
 * - keyboard lock 在 AI 命令执行期间生效
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { t } from "../i18n/index.svelte.ts";
import type {
  AiSessionInfo,
  AiSettings,
  AuditLog,
  ChatItem,
  CommandProposed,
  CommandResult,
  SkillRecord,
} from "./types.ts";

// ─── Position（常量先声明，下方 $state 初始化会调 loadPos） ──────

const POS_KEY = "ai_panel_position";
function loadPos(): "left" | "right" {
  const v = localStorage.getItem(POS_KEY);
  return v === "left" || v === "right" ? v : "right";
}

// ─── 全局可见状态 ─────────────────────────────────────────────────

let _open = $state(false);
let _position = $state<"left" | "right">(loadPos());
let _activeSessionId = $state<string | null>(null);
let _sessionByTarget = $state<Record<string, AiSessionInfo>>({});
let _chatBySession = $state<Record<string, ChatItem[]>>({});
let _pendingByTarget = $state<Record<string, CommandProposed | null>>({});
let _keyboardLockedByTarget = $state<Record<string, boolean>>({});
let _settings = $state<AiSettings | null>(null);

const _unlisteners: Record<string, UnlistenFn[]> = {};

export function position() { return _position; }
export function setPosition(p: "left" | "right") {
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
  let unlisten: UnlistenFn | null = null;
  let timer: number | null = null;
  const sentinelRegex = new RegExp(
    proposed.sentinel.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + ":(-?\\d+)"
  );

  const stripAnsi = (s: string) =>
    s.replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "")
     .replace(/\x1b\][^\x07]*\x07/g, "")
     .replace(/\r/g, "");

  const finish = async (output: string, exit_code: number, timed_out: boolean) => {
    if (resolved) return;
    resolved = true;
    if (unlisten) unlisten();
    if (timer != null) clearTimeout(timer);
    try {
      await invoke("ai_command_result", {
        sessionId: session_id,
        toolCallId: proposed.tool_call_id,
        exitCode: exit_code,
        output,
        timedOut: timed_out,
      });
    } catch (e) {
      console.error("[ai] ai_command_result failed:", e);
    }
  };

  unlisten = await listen<number[]>(dataEvent, (e) => {
    if (resolved) return;
    const chunk = new TextDecoder("utf-8", { fatal: false }).decode(new Uint8Array(e.payload));
    buffer += chunk;
    const m = sentinelRegex.exec(buffer);
    if (m) {
      const exit = parseInt(m[1], 10);
      const sentinelLineStart = buffer.lastIndexOf("\n", m.index);
      // sentinel 行之前的部分 = echo 行 + 实际输出
      let raw = buffer.substring(0, sentinelLineStart >= 0 ? sentinelLineStart : 0);
      raw = stripAnsi(raw);
      // 去掉第一行（PTY echo 的命令本身）
      const firstNl = raw.indexOf("\n");
      const output = firstNl >= 0 ? raw.substring(firstNl + 1) : raw;
      void finish(output.trimEnd(), exit, false);
    }
  });

  // 写命令到 PTY；末尾 \n 触发 shell 执行
  const data = Array.from(new TextEncoder().encode(proposed.full_cmd + "\n"));
  await invoke(writeCmd, { sessionId: target_session_id, data });

  timer = window.setTimeout(() => {
    void finish(stripAnsi(buffer).trim(), -1, true);
  }, Math.max(1000, proposed.timeout_s * 1000)) as unknown as number;
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
export async function loadSettings(): Promise<AiSettings> {
  _settings = await invoke<AiSettings>("ai_settings_get");
  return _settings;
}
export async function saveSettings(s: Partial<{ provider: string; model: string; endpoint: string | null; apiKey: string | null }>) {
  await invoke("ai_settings_set", s);
  await loadSettings();
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

  u.push(await listen<{ id: string; text: string }>(`ai:assistant_message_end:${sid}`, (e) => {
    const arr = _chatBySession[sid] ?? [];
    for (let i = arr.length - 1; i >= 0; i--) {
      const item = arr[i];
      if (item.kind === "assistant" && item.id === e.payload.id) {
        // 用 final text 作为权威值（防 delta 漏）；text 空就移除该气泡（纯 tool_use 轮次）
        if (!e.payload.text || e.payload.text.length === 0) {
          _chatBySession[sid] = [...arr.slice(0, i), ...arr.slice(i + 1)];
        } else {
          const replaced: ChatItem = { ...item, text: e.payload.text, streaming: false };
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
