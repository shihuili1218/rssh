/**
 * AI 排障会话前端状态。
 * - 一个 tab 至多一个 AI 会话；store 全部按 tab_id 索引
 *   （actor 跟 tab 同寿命 —— SSH 断了重连 tab_id 不变，AI 会话和历史也保留）
 * - 监听 ai:*:<tab_id> 事件填充 chat 时间线
 * - keyboard lock 在 AI 命令执行期间生效
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { t, locale as currentLocale } from "../i18n/index.svelte.ts";
import { extractOutput, findSentinel } from "./pty-output.ts";
import { PROBE_COMMAND, classifyProbeBuffer } from "./shell-probe.ts";
import type {
  AiSessionInfo,
  AiSettings,
  AuditLog,
  ChatItem,
  CommandProposed,
  CommandResult,
  LlmProvider,
  ModelInfo,
  RedactRuleRecord,
  ShellKind,
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
let _activeTabId = $state<string | null>(null);
let _sessionByTab = $state<Record<string, AiSessionInfo>>({});
let _chatByTab = $state<Record<string, ChatItem[]>>({});
let _pendingByTab = $state<Record<string, CommandProposed | null>>({});
let _keyboardLockedByTab = $state<Record<string, boolean>>({});
let _settings = $state<AiSettings | null>(null);
/**
 * tab_id → 终端类型映射。internal_command 自动执行时需要知道走 ssh_write
 * 还是 pty_write —— ChatPanel 把 targetKind 作为 prop 传给 dialog，但 store
 * 在 attachListeners 里要独立处理 internal_command 事件，所以单独缓存。
 *
 * 按 tab_id 索引（不按 target_id）——重连后 target_id 变了 kind 不变，
 * 用 tab_id 才能保证 internal_command 路由不丢。
 */
const _targetKindByTab: Record<string, "ssh" | "local"> = {};

const _unlistenersByTab: Record<string, UnlistenFn[]> = {};

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

export function activeTabId() { return _activeTabId; }
export function activeSession(): AiSessionInfo | null {
  if (!_activeTabId) return null;
  return _sessionByTab[_activeTabId] ?? null;
}
export function sessionForTab(tab_id: string): AiSessionInfo | undefined {
  return _sessionByTab[tab_id];
}
export function listAllSessions(): AiSessionInfo[] {
  return Object.values(_sessionByTab);
}

export function chatItems(tab_id: string): ChatItem[] {
  return _chatByTab[tab_id] ?? [];
}
export function pendingCommand(tab_id: string): CommandProposed | null {
  return _pendingByTab[tab_id] ?? null;
}
export function isKeyboardLocked(tab_id: string): boolean {
  return _keyboardLockedByTab[tab_id] === true;
}

function pushChat(tab_id: string, item: ChatItem) {
  const arr = _chatByTab[tab_id] ?? [];
  _chatByTab[tab_id] = [...arr, item];
}

// ─── Lifecycle ────────────────────────────────────────────────────

export async function startSession(args: {
  tabId: string;
  targetKind: "ssh" | "local";
  targetId: string;
  skill: string;
  provider: string;
  model: string;
}): Promise<AiSessionInfo> {
  const info = await invoke<AiSessionInfo>("ai_session_start", {
    tabId: args.tabId,
    target: { kind: args.targetKind, id: args.targetId },
    skill: args.skill,
    provider: args.provider,
    model: args.model,
    locale: currentLocale(),
  });
  // info.tab_id 后端权威 —— 跟 args.tabId 一定一致（后端按入参 insert），但用
  // 后端返回值就消除"未来后端 normalize tab_id"导致 cache miss 的隐患。
  _sessionByTab[info.tab_id] = info;
  _targetKindByTab[info.tab_id] = args.targetKind;
  _chatByTab[info.tab_id] = [];
  _activeTabId = info.tab_id;
  await attachListeners(info);
  return info;
}

export async function stopSession(tab_id: string) {
  // Tear down in-flight executions for this tab FIRST. Without this,
  // the PTY data listener + 60s setTimeout linger after the session is
  // gone, the buffer keeps appending against a defunct session, and the
  // eventual ai_command_result invoke targets a session the backend has
  // already dropped (silent reject).
  //
  // Iterate via Array.from so the in-loop `.delete()` inside finish()
  // doesn't break Map iteration semantics.
  for (const exec of Array.from(_runningExecutions.values())) {
    if (exec.tabId === tab_id && !exec.resolved) {
      await exec.terminate();
    }
  }

  await invoke("ai_session_stop", { tabId: tab_id });
  detachListeners(tab_id);
  delete _sessionByTab[tab_id];
  delete _pendingByTab[tab_id];
  delete _keyboardLockedByTab[tab_id];
  delete _targetKindByTab[tab_id];
  delete _chatByTab[tab_id];
  if (_activeTabId === tab_id) _activeTabId = null;
}

export async function sendMessage(tab_id: string, text: string) {
  await invoke("ai_user_message", { tabId: tab_id, text });
}

/** 清空 actor 的对话历史（audit log 保留）。actor 不死，下条消息从头来过。 */
export async function clearContext(tab_id: string): Promise<void> {
  await invoke("ai_session_clear_context", { tabId: tab_id });
}

/** SSH 重连后调用：让 actor 内部把 target_id + ssh_handle 切到新 SSH 连接。 */
export async function rebindTarget(
  tab_id: string,
  target_kind: "ssh" | "local",
  target_id: string,
): Promise<void> {
  await invoke("ai_session_rebind_target", {
    tabId: tab_id,
    target: { kind: target_kind, id: target_id },
  });
  // 同步前端 cache：AiSessionInfo.target_id 也要换，否则下次 sendMessage 走的
  // executeCommand 还会用旧 target_session_id 给 ssh_write —— 拿不到新 PTY。
  const info = _sessionByTab[tab_id];
  if (info) {
    _sessionByTab[tab_id] = { ...info, target_id };
  }
}

/** 打断 actor 正在跑的 LLM 流式响应。会话上下文（history / pending command / audit）全部保留——
 *  这跟 stopSession（销毁整个会话）是两个语义。actor 不在 chat 时调用是 no-op。 */
export async function cancelStream(tab_id: string): Promise<void> {
  await invoke("ai_cancel_stream", { tabId: tab_id });
}

/** 连接时探测的门控：这个 SSH target 现在需要探测吗？
 *  后端判定 auto_detect on + 会话存在 + 该 profile 缓存 miss。命中缓存或开关关 →
 *  false，于是连接 / 重连都不会重复刷探针。本地 PTY 的 target_id 不在后端 sessions 里
 *  → 自然 false。 */
export async function remoteShellProbeNeeded(target_id: string): Promise<boolean> {
  return invoke<boolean>("ai_remote_shell_probe_needed", { targetId: target_id });
}

/** SSH 连接成功后探测远端 shell：粘一行 `echo P=$PSEdition=$$=E` 到 PTY，listen 1.5s
 *  解析输出，分类后写进程级缓存（ai_cache_remote_shell，key=profile_id）。AI session
 *  启动时从缓存读初始 shell —— 探测与 AI 会话生命周期解耦，无需 tab_id / actor。
 *
 *  返回 true = 探测成功并已写缓存。false = 超时 / classify 模糊（不写缓存，下次连接重探）。
 *
 *  时机：TerminalPane.connectAndWire 的 SSH 成功分支，且 remoteShellProbeNeeded 为真。
 *  init_command 已由后端在 ssh_connect 返回前写入 PTY，探针排在其后执行。
 *  视觉代价：用户看到一行 echo 滚过终端 —— 每个 profile 每进程仅一次（缓存门控）。
 *
 *  分类规则（见 shell-probe.ts，纯逻辑 + 单测）：回显行被 `(?<!echo )` lookbehind 排除，
 *  只认求值输出。powershell/posix 的求值签名一见即定；cmd 签名（== 被排除的回显签名）
 *  只可能来自真 cmd.exe 的求值输出，且只在 deadline 才采纳——给 posix/ps 求值行先到的机会，
 *  慢链路下求值行整个没到 → 不写缓存，POSIX 兜底重探（不会误缓存 cmd）。
 */
export async function probeRemoteShell(target_id: string): Promise<boolean> {
  const dataEvent = `ssh:data:${target_id}`;
  const cache = (shell: ShellKind) =>
    // 后端按 target_id 查 profile_id 写缓存；target 已断则静默跳过。
    invoke("ai_cache_remote_shell", { targetId: target_id, shell });

  // Tail-bounded buffer: the probe echo + its evaluated output land at the END
  // of the stream (after any MOTD / init_command noise), so only the recent tail
  // matters. Cap it so a chatty connect can't grow an unbounded string that
  // classifyProbeBuffer re-scans every 80ms. Trim on a newline boundary so a cut
  // can never open a line mid-token and let `^P=` false-match a sliced echo line.
  const TAIL_CAP = 16 * 1024;
  let buffer = "";
  const unlisten = await listen<number[]>(dataEvent, (e) => {
    buffer += new TextDecoder("utf-8", { fatal: false }).decode(new Uint8Array(e.payload));
    if (buffer.length > TAIL_CAP) {
      const tail = buffer.slice(-TAIL_CAP);
      const nl = tail.indexOf("\n");
      buffer = nl >= 0 ? tail.slice(nl + 1) : tail;
    }
  });
  try {
    await invoke("ssh_write", {
      sessionId: target_id,
      data: Array.from(new TextEncoder().encode(PROBE_COMMAND + "\r")),
    });
    // 1.5s deadline：远端通常 100-300ms 内回响，给 5x 头量容忍慢链路。
    const deadline = Date.now() + 1500;
    while (Date.now() < deadline) {
      const { kind } = classifyProbeBuffer(buffer);
      if (kind) {
        await cache(kind); // posix/powershell 的求值行无歧义，立即定夺
        return true;
      }
      await new Promise((r) => setTimeout(r, 80));
    }
    // 超时：没等到 posix/powershell 求值行。若 buffer 里有（非回显的）cmd 求值签名 → 真 cmd.exe；
    // 慢链路下求值行整个没到（只有被 lookbehind 排除的回显）→ cmd=false → 不写缓存，POSIX 兜底。
    if (classifyProbeBuffer(buffer).cmd) {
      await cache("cmd");
      return true;
    }
    console.warn("[ai] shell probe timed out — keeping POSIX fallback");
    return false;
  } catch (e) {
    console.error("[ai] shell probe failed:", e);
    return false;
  } finally {
    unlisten();
  }
}

/** 当前会话的助手消息是否正在流式输出 —— UI 用它把"发送"按钮切成"停止"。 */
export function isStreaming(tab_id: string): boolean {
  const arr = _chatByTab[tab_id];
  if (!arr || arr.length === 0) return false;
  const last = arr[arr.length - 1];
  return last.kind === "assistant" && last.streaming === true;
}

/**
 * Bounded PTY buffer for one in-flight command.
 *
 * Why this exists: `buffer += chunk` is unbounded. Commands like `yes`,
 * `tail -f /var/log/...`, or `cat /dev/urandom | base64` can pump tens
 * of MB into the buffer before the sentinel ever appears (timeout path),
 * which freezes the renderer when `findSentinel` runs a regex over the
 * whole string on every chunk.
 *
 * Strategy: keep a fixed-size HEAD as the real output, a sliding TAIL
 * window where the sentinel must appear once the command exits. Once
 * HEAD is full, new chunks only update TAIL. `view()` is what
 * `findSentinel`/`extractOutput` see — it concatenates HEAD + TAIL,
 * dropping the middle segment for over-cap runs.
 *
 * The sentinel is `__rssh_done_<uuid_simple>:<exit_code>` — ~60 bytes.
 * TAIL = 4 KB gives a wide margin so a chunk-aligned sentinel can't be
 * lost across the head/tail seam.
 */
const HEAD_CAP = 512 * 1024;
const TAIL_WIN = 4 * 1024;

class CappedBuffer {
  private head = "";
  private tail = "";

  append(chunk: string) {
    const room = HEAD_CAP - this.head.length;
    if (room > 0) {
      if (chunk.length <= room) {
        this.head += chunk;
      } else {
        this.head += chunk.substring(0, room);
      }
    }
    // Always update tail so the sentinel detection window slides forward.
    // While head is still filling, tail mirrors head's recent suffix;
    // after head is sealed, only tail moves.
    this.tail = (this.tail + chunk).slice(-TAIL_WIN);
  }

  /** Concatenated view used by findSentinel / extractOutput. */
  view(): string {
    // While head still has room, the tail is a suffix of the head — no
    // concat needed (avoids duplicating recent bytes in the matcher).
    if (this.head.length < HEAD_CAP) return this.head;
    return this.head + this.tail;
  }
}

/** Per-tool-call execution state. Lives in `_runningExecutions` Map. */
type Execution = {
  toolCallId: string;
  tabId: string;
  targetSessionId: string;
  targetKind: "ssh" | "local";
  buffer: CappedBuffer;
  resolved: boolean;
  userInterrupted: boolean;
  unlisten: UnlistenFn | null;
  timer: number | null;
  terminate: () => Promise<void>;
};

/**
 * Indexed by tool_call_id. Keyed on the Map (not a Record) so iteration
 * is O(N) without enumerating prototype noise, and to make the "find all
 * in-flight execs for a tab" sweep in stopSession explicit.
 */
const _runningExecutions: Map<string, Execution> = new Map();

export function isCommandRunning(tool_call_id: string): boolean {
  return _runningExecutions.has(tool_call_id);
}

/**
 * Execute an AI-proposed command: paste `full_cmd` (with sentinel +
 * exit-code echo) into the active terminal, watch the PTY stream for
 * the sentinel, then report output + exit code to the backend. All
 * front-end; the backend's ai module never executes commands itself.
 */
export async function executeCommand(
  tab_id: string,
  proposed: CommandProposed,
  target_kind: "ssh" | "local",
  target_session_id: string,
): Promise<void> {
  const writeCmd = target_kind === "ssh" ? "ssh_write" : "pty_write";
  const dataEvent = target_kind === "ssh"
    ? `ssh:data:${target_session_id}`
    : `pty:data:${target_session_id}`;

  // Returned Promise resolves only when finish() actually runs, so the
  // UI's "executing" state can cover the whole execution window — not
  // just up to the `invoke(writeCmd)` round-trip.
  let resolveDone!: () => void;
  const done = new Promise<void>((r) => { resolveDone = r; });

  const exec: Execution = {
    toolCallId: proposed.tool_call_id,
    tabId: tab_id,
    targetSessionId: target_session_id,
    targetKind: target_kind,
    buffer: new CappedBuffer(),
    resolved: false,
    userInterrupted: false,
    unlisten: null,
    timer: null,
    terminate: async () => {
      if (exec.resolved) return;
      exec.userInterrupted = true;
      const ctrlC = Array.from(new TextEncoder().encode("\x03"));
      // Fire-and-forget Ctrl+C — but keep the failure visible. PTY closed
      // / session lost will reject the invoke; a warn line helps debug the
      // "I clicked terminate but Ctrl+C never went out" report path.
      void invoke(writeCmd, { sessionId: target_session_id, data: ctrlC })
          .catch((err) => console.warn("[ai] terminate Ctrl+C failed:", err));
      await finish(extractOutput(exec.buffer.view()), -1, false);
    },
  };

  const finish = async (output: string, exit_code: number, timed_out: boolean) => {
    if (exec.resolved) return;
    exec.resolved = true;
    if (exec.unlisten) exec.unlisten();
    if (exec.timer != null) clearTimeout(exec.timer);
    _runningExecutions.delete(exec.toolCallId);
    try {
      await invoke("ai_command_result", {
        tabId: tab_id,
        toolCallId: exec.toolCallId,
        exitCode: exit_code,
        output,
        timedOut: timed_out,
        earlyTerminated: exec.userInterrupted,
      });
    } catch (e) {
      console.error("[ai] ai_command_result failed:", e);
    }
    resolveDone();
  };

  exec.unlisten = await listen<number[]>(dataEvent, (e) => {
    if (exec.resolved) return;
    const chunk = new TextDecoder("utf-8", { fatal: false }).decode(new Uint8Array(e.payload));
    exec.buffer.append(chunk);
    const hit = findSentinel(exec.buffer.view(), proposed.sentinel);
    if (hit) void finish(hit.output, hit.exitCode, false);
  });

  _runningExecutions.set(exec.toolCallId, exec);

  // \r (not \n) is the cross-platform Enter byte: ConPTY/PowerShell only
  // accepts \r; Unix cooked PTY translates \r → \n via ICRNL. Matches the
  // byte xterm.js sends when the user presses Enter themselves.
  // If invoke throws (session already closed), listener + execution are
  // already registered → must funnel through finish() to clean up, else
  // isCommandRunning() stays true forever.
  const data = Array.from(new TextEncoder().encode(proposed.full_cmd + "\r"));
  try {
    await invoke(writeCmd, { sessionId: target_session_id, data });
  } catch (e) {
    await finish(`failed to write command: ${e instanceof Error ? e.message : String(e)}`, -1, false);
    throw e;
  }

  exec.timer = window.setTimeout(() => {
    void finish(extractOutput(exec.buffer.view()), -1, true);
  }, Math.max(1000, proposed.timeout_s * 1000)) as unknown as number;

  return done;
}

/** Early-terminate by tool_call_id: Ctrl+C to target shell + finish(). */
export async function terminateCommand(tool_call_id: string): Promise<void> {
  const exec = _runningExecutions.get(tool_call_id);
  if (exec) await exec.terminate();
}

export async function rejectCommand(tab_id: string, tool_call_id: string, reason: string) {
  await invoke("ai_command_reject", { tabId: tab_id, toolCallId: tool_call_id, reason });
}

export async function getAudit(tab_id: string): Promise<AuditLog> {
  return invoke<AuditLog>("ai_audit_get", { tabId: tab_id });
}

export async function saveAudit(tab_id: string, file_path: string) {
  return invoke("ai_audit_save", { tabId: tab_id, filePath: file_path });
}

/** Desktop-only：弹原生 Save 对话框选路径并保存。返回路径或 null（用户取消）。 */
export async function saveAuditWithDialog(tab_id: string): Promise<string | null> {
  return invoke<string | null>("ai_audit_save_pick", { tabId: tab_id });
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
export async function saveSettings(s: Partial<{
  provider: string;
  model: string;
  endpoint: string | null;
  apiKey: string | null;
  dangerMode: boolean;
  autoRunCommand: boolean;
  autoMatchFile: boolean;
  autoDownloadFile: boolean;
  autoAnalyzeLocally: boolean;
  autoPatchCp: boolean;
  autoPatchModify: boolean;
  autoPatchDiff: boolean;
  autoPatchMv: boolean;
  autoDetectRemoteShell: boolean;
}>) {
  // Backend takes a single `patch` object (AiSettingsPatch) — every field is
  // "update if present". Wrap the partial settings accordingly.
  await invoke("ai_settings_set", { patch: s });
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
  // tab_id 同时是状态字典 key、事件 topic 后缀、internal_command 路由 key —— 单一坐标。
  // info.target_id 不在闭包里捕获 —— 重连后 target_id 变了，internal_command 需要走新的，
  // 闭包里写死会一直发到旧 SSH 会话。运行期通过 _sessionByTab[tab_id].target_id 读最新值。
  const tab = info.tab_id;
  const u: UnlistenFn[] = [];

  u.push(await listen<{ text: string }>(`ai:user_message:${tab}`, (e) => {
    pushChat(tab, { kind: "user", text: e.payload.text, at: Date.now() });
  }));

  // 流式：start 创建空气泡，delta append，end 关 streaming 标记
  u.push(await listen<{ id: string }>(`ai:assistant_message_start:${tab}`, (e) => {
    pushChat(tab, { kind: "assistant", id: e.payload.id, text: "", at: Date.now(), streaming: true });
  }));

  u.push(await listen<{ id: string; text: string }>(`ai:assistant_delta:${tab}`, (e) => {
    const arr = _chatByTab[tab];
    if (!arr) return;
    // Mutate the matching item in place. Svelte 5's $state proxy picks up
    // field assignments, so we don't need React-style full-array rebuilds
    // (which were O(N) per token — an 8 000-token streamed reply over a
    // 100-message chat is 800 000 array clones / 24 MB of GC churn).
    for (let i = arr.length - 1; i >= 0; i--) {
      const item = arr[i];
      if (item.kind === "assistant" && item.id === e.payload.id) {
        item.text += e.payload.text;
        return;
      }
    }
  }));

  u.push(await listen<{ id: string; text: string; cancelled?: boolean }>(`ai:assistant_message_end:${tab}`, (e) => {
    const arr = _chatByTab[tab] ?? [];
    for (let i = arr.length - 1; i >= 0; i--) {
      const item = arr[i];
      if (item.kind === "assistant" && item.id === e.payload.id) {
        const isEmpty = !e.payload.text || e.payload.text.length === 0;
        // cancelled=true 时即使 text 空也要保留气泡——UI 模板会渲染本地化的
        // "已停止"徽章，告诉用户这一轮被自己打断了。
        // 只有"纯 tool_use 轮次"（chat 没产文本只产 tool_calls，cancelled=false）
        // 或 chat 失败（empty + cancelled=false）才移除气泡。
        if (isEmpty && !e.payload.cancelled) {
          _chatByTab[tab] = [...arr.slice(0, i), ...arr.slice(i + 1)];
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
          _chatByTab[tab] = [...arr.slice(0, i), replaced, ...arr.slice(i + 1)];
        }
        return;
      }
    }
  }));

  u.push(await listen<CommandProposed>(`ai:command_proposed:${tab}`, (e) => {
    _pendingByTab[tab] = e.payload;
    pushChat(tab, { kind: "command", cmd: e.payload, at: Date.now() });
  }));

  // internal_command：当前只用于 file_ops 工具的远端能力探测（一行只读 echo "py3=... perl=... diff=..."）。
  // 不弹审批、不入 chat 时间线，直接粘到 PTY 跑——用户在终端历史里看到探测命令滚过，
  // 透明但不打断流程。后续若加其他 read-only 内部命令也走这条路径。
  u.push(await listen<{
    id: string;
    tool_call_id: string;
    cmd: string;
    full_cmd: string;
    sentinel: string;
  }>(`ai:internal_command:${tab}`, async (e) => {
    const kind = _targetKindByTab[tab];
    // 每次都从 _sessionByTab 读最新 target_id —— 重连后这个值会被 rebindTarget 更新，
    // 闭包里不能缓存（缓存的话 internal_command 在重连后会粘到旧 SSH 会话）。
    const currentInfo = _sessionByTab[tab];
    if (!kind || !currentInfo) {
      // fail-closed：必须给后端回一个 result，否则 wait_command_outcome 永远阻塞，
      // session actor 卡在 file_ops handler 里 await 不出来，整个 AI 会话挂死。
      const msg = `internal_command without target binding for tab ${tab}`;
      console.error("[ai]", msg);
      try {
        await invoke("ai_command_result", {
          tabId: tab,
          toolCallId: e.payload.tool_call_id,
          exitCode: -1,
          output: msg,
          timedOut: false,
          earlyTerminated: false,
        });
      } catch (err) {
        console.error("[ai] failed to report internal_command target miss:", err);
      }
      return;
    }
    const proposed: CommandProposed = {
      id: e.payload.id,
      tool_call_id: e.payload.tool_call_id,
      cmd: e.payload.cmd,
      full_cmd: e.payload.full_cmd,
      sentinel: e.payload.sentinel,
      explain: "",
      side_effect: "",
      timeout_s: 60,
    };
    try {
      await executeCommand(tab, proposed, kind, currentInfo.target_id);
    } catch (err) {
      // executeCommand 在 PTY listen 失败等情况下可能在自己发 ai_command_result 之前就抛。
      // 不补一个失败 result，wait_command_outcome 会永挂在 Rust 侧。
      console.error("[ai] internal_command exec failed:", err);
      try {
        await invoke("ai_command_result", {
          tabId: tab,
          toolCallId: e.payload.tool_call_id,
          exitCode: -1,
          output: err instanceof Error ? err.message : String(err),
          timedOut: false,
          earlyTerminated: false,
        });
      } catch (reportErr) {
        console.error("[ai] failed to report internal_command exec failure:", reportErr);
      }
    }
  }));

  u.push(await listen<{ id: string; lock_keyboard: boolean }>(`ai:command_executing:${tab}`, (e) => {
    _keyboardLockedByTab[tab] = !!e.payload.lock_keyboard;
  }));

  u.push(await listen<CommandResult & { lock_keyboard: boolean }>(`ai:command_completed:${tab}`, (e) => {
    _keyboardLockedByTab[tab] = !!e.payload.lock_keyboard;
    _pendingByTab[tab] = null;
    // 给最近一条对应 id 的 command 项填上 result
    const arr = _chatByTab[tab] ?? [];
    for (let i = arr.length - 1; i >= 0; i--) {
      const item = arr[i];
      if (item.kind === "command" && item.cmd.id === e.payload.id) {
        const replaced: ChatItem = { ...item, result: e.payload };
        _chatByTab[tab] = [...arr.slice(0, i), replaced, ...arr.slice(i + 1)];
        break;
      }
    }
  }));

  // 拒绝路径单独事件 —— complete 跟 reject 是两种语义，复用 command_completed
  // 加 rejected:true 字段会让 listener 分支模糊。后端 RejectCommand 分支 emit
  // 这个，前端清 pending + 标记 ChatItem.rejected。
  u.push(await listen<{ id: string; reason: string }>(`ai:command_rejected:${tab}`, (e) => {
    _pendingByTab[tab] = null;
    const arr = _chatByTab[tab] ?? [];
    for (let i = arr.length - 1; i >= 0; i--) {
      const item = arr[i];
      if (item.kind === "command" && item.cmd.id === e.payload.id) {
        const replaced: ChatItem = { ...item, rejected: { reason: e.payload.reason } };
        _chatByTab[tab] = [...arr.slice(0, i), replaced, ...arr.slice(i + 1)];
        break;
      }
    }
  }));

  u.push(await listen<{ message: string }>(`ai:error:${tab}`, (e) => {
    pushChat(tab, { kind: "error", text: e.payload.message, at: Date.now() });
  }));

  // 用户按"清理上下文"——后端清完 history 后 emit 这个事件，前端把气泡也抹掉。
  // pending command / keyboard lock 一并清：清上下文等于把这个 actor 重置回 idle。
  u.push(await listen<{}>(`ai:context_cleared:${tab}`, () => {
    _chatByTab[tab] = [];
    _pendingByTab[tab] = null;
    _keyboardLockedByTab[tab] = false;
  }));

  u.push(await listen<{}>(`ai:session_ended:${tab}`, () => {
    pushChat(tab, { kind: "note", text: t("ai.session.ended_note"), at: Date.now() });
  }));

  _unlistenersByTab[tab] = u;
}

function detachListeners(tab_id: string) {
  const arr = _unlistenersByTab[tab_id];
  if (arr) {
    arr.forEach(fn => fn());
    delete _unlistenersByTab[tab_id];
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

// ─── 脱敏规则管理 ──────────────────────────────────────────────────
// 变更只对新会话生效（后端建会话时 snapshot）。saveRedactRule 在后端编译校验正则，
// 坏正则会抛 redact_invalid_regex。

export async function listRedactRules(): Promise<RedactRuleRecord[]> {
  return invoke<RedactRuleRecord[]>("ai_list_redact_rules");
}

export async function saveRedactRule(r: { id: string; pattern: string; replacement: string }): Promise<void> {
  return invoke("ai_save_redact_rule", r);
}

export async function deleteRedactRule(id: string): Promise<void> {
  return invoke("ai_delete_redact_rule", { id });
}
