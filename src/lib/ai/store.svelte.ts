/**
 * AI 排障会话前端状态。
 * - 一个 tab 至多一个 AI 会话；store 全部按 tab_id 索引
 *   （切 tab / SSH 重连不丢；显式关闭 AI 面板则结束当前会话）
 * - 监听 ai:*:<tab_id> 事件填充 chat 时间线
 * - keyboard lock 在 AI 命令执行期间生效
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type Event as TauriEvent, type UnlistenFn } from "@tauri-apps/api/event";
import { saveTextFile, fileStamp } from "../save-file.ts";
import { t, errMsg, locale as currentLocale } from "../i18n/index.svelte.ts";
import { extractOutput, findSentinel } from "./pty-output.ts";
import { truncateCommand } from "./format.ts";
import { PROBE_COMMAND, classifyProbeBuffer } from "./shell-probe.ts";
import {
  applyTerminalMutations,
  restoreTimeline,
  type AiTerminalMutation,
} from "./timeline.ts";
import { sessionCommandKey, type SessionInstanceRef } from "./session-identity.ts";
import { commandApprovals, isAutoApprovalAllowed } from "./command-approval.ts";
export type { SessionInstanceRef } from "./session-identity.ts";
import type {
  AiSessionInfo,
  AiSettings,
  AiTargetKind,
  AuditLog,
  ChatItem,
  CommandProposed,
  CategoryGroup,
  CommandResult,
  ConversationMeta,
  LlmProvider,
  ModelInfo,
  RedactRuleRecord,
  ShellKind,
  SkillRecord,
  TokenUsage,
} from "./types.ts";
import { isRawDeviceKind } from "./types.ts";

// ─── Position ────────────────────────────────────────────────────
// 只支持 left/right。移动端用户横屏即可用——左右布局就够了，没必要再开上下分支。

export type AiPosition = "left" | "right";

const POS_KEY = "ai_panel_position";
const LEGACY_PANEL_WIDTH_KEY = "ai-panel-width";
const MIN_PANEL_WIDTH = 280;
function loadPos(): AiPosition {
  const v = localStorage.getItem(POS_KEY);
  return v === "left" || v === "right" ? v : "right";
}

function loadLegacyPanelWidth(): number | null {
  const raw = localStorage.getItem(LEGACY_PANEL_WIDTH_KEY);
  if (!raw) return null;
  const width = Number.parseInt(raw, 10);
  return Number.isFinite(width) && width >= MIN_PANEL_WIDTH ? width : null;
}

// ─── Per-tab visibility ───────────────────────────────────────────

let _openByTab = $state<Record<string, true>>({});
let _panelWidthByTab = $state<Record<string, number | null>>({});
let _position = $state<AiPosition>(loadPos());
let _initialPanelWidth = loadLegacyPanelWidth();
let _sessionByTab = $state<Record<string, AiSessionInfo>>({});
let _chatByTab = $state<Record<string, ChatItem[]>>({});
let _pendingByTab = $state<Record<string, CommandProposed | null>>({});
let _keyboardLockedByTab = $state<Record<string, boolean>>({});
/**
 * tab_id → cumulative token spend for the actor's lifetime. Deliberately NOT
 * reset on context clear — clearing the conversation doesn't refund tokens
 * already billed; the counter tracks money, not context size.
 */
let _tokensByTab = $state<Record<string, TokenUsage>>({});
let _settings = $state<AiSettings | null>(null);
/**
 * tab_id → 终端类型映射。internal_command 自动执行时需要知道走 ssh_write
 * 还是 pty_write —— ChatPanel 把 targetKind 作为 prop 传给 dialog，但 store
 * 在 attachListeners 里要独立处理 internal_command 事件，所以单独缓存。
 *
 * 按 tab_id 索引（不按 target_id）——重连后 target_id 变了 kind 不变，
 * 用 tab_id 才能保证 internal_command 路由不丢。
 */
const _targetKindByTab: Record<string, AiTargetKind> = {};

type ContextEpochState = Readonly<{
  instanceId: string;
  epoch: number;
}>;

type PendingContextClear = {
  readonly instanceId: string;
  readonly targetEpoch: number;
  readonly bufferedEvents: Array<() => void>;
};

/**
 * One monotonic conversation epoch per live actor. Both sides start at zero
 * and increment only after a ClearContext action has been processed. Tauri
 * commands and events use independent channels, so the epoch is the fence that
 * stops an already-queued pre-clear event from rebuilding cleared UI.
 */
const _contextEpochByTab: Record<string, ContextEpochState> = {};
const _pendingContextClearByTab: Record<string, PendingContextClear> = {};

const _unlistenersByTab: Record<string, UnlistenFn[]> = {};
const _tabGeneration: Record<string, number> = {};
const _disposedTabs = new Set<string>();
const _sessionTeardownByTab: Partial<Record<string, Promise<void>>> = {};
const _sessionLaunchesByTab: Record<string, Set<Promise<AiSessionInfo>>> = {};
type PendingConversationMutation = Readonly<{
  kind: "send";
  sequence: number;
  clientId: string;
  operation: Promise<void>;
}> | Readonly<{
  kind: "clear";
  sequence: number;
  operation: Promise<void>;
}>;
const _pendingConversationMutationsByTab: Record<
  string,
  Set<PendingConversationMutation>
> = {};
/** Backend actor actions are ordered, but independent Tauri commands are not a
 * frontend ordering primitive. Serialize Message/ClearContext per tab so the
 * order in which callers mutate the UI is also the order Rust receives them.
 * In particular, a clear acknowledgement and its UI reset must finish before a
 * later send can produce assistant/command events. */
const _conversationMutationTailByTab: Partial<Record<string, Promise<void>>> = {};
let _nextConversationMutationId = 0;

class SessionClosedError extends Error {
  constructor(tab_id: string) {
    super(`AI session closed for tab: ${tab_id}`);
    this.name = "SessionClosedError";
  }
}

class CommandSetupError extends Error {
  constructor(readonly cause: unknown) {
    super(errMsg(cause));
    this.name = "CommandSetupError";
  }
}

function isAiSessionNotFound(error: unknown): boolean {
  const message = typeof error === "string"
    ? error
    : error instanceof Error ? error.message : "";
  return message.includes("ai_session_not_found");
}

function tabGeneration(tab_id: string): number {
  return _tabGeneration[tab_id] ?? 0;
}

function isTabLive(tab_id: string, generation: number): boolean {
  return !_disposedTabs.has(tab_id) && tabGeneration(tab_id) === generation;
}

function assertTabLive(tab_id: string, generation = tabGeneration(tab_id)): void {
  if (!isTabLive(tab_id, generation)) throw new SessionClosedError(tab_id);
}

/** Opaque ownership token for one panel-open conversation lifetime. Async UI
 * actions capture it before their first await; closePanel bumps generation, so
 * no old continuation can attach to or mutate a later conversation in the same tab. */
export interface SessionLease {
  readonly tabId: string;
  readonly generation: number;
}

export function captureSessionLease(tab_id: string): SessionLease {
  const generation = tabGeneration(tab_id);
  assertTabLive(tab_id, generation);
  return { tabId: tab_id, generation };
}

function generationForLease(tab_id: string, lease: SessionLease): number {
  if (lease.tabId !== tab_id) throw new SessionClosedError(tab_id);
  return lease.generation;
}

function assertLeaseLive(tab_id: string, lease: SessionLease): void {
  assertTabLive(tab_id, generationForLease(tab_id, lease));
}

export function position() { return _position; }
export function setPosition(p: AiPosition) {
  _position = p;
  localStorage.setItem(POS_KEY, p);
}

// ─── Open/close ───────────────────────────────────────────────────

export function isOpen(tab_id: string) { return _openByTab[tab_id] === true; }
export function openPanel(tab_id: string) { _openByTab[tab_id] = true; }
function hidePanel(tab_id: string) { delete _openByTab[tab_id]; }
export function closePanel(tab_id: string): Promise<void> {
  hidePanel(tab_id);
  clearPrefill(tab_id);
  // 面板关闭即结束这轮 conversation。先换 generation，所有迟到 listener/start
  // continuation 都失效；宽度是 tab 偏好，不属于 conversation，继续保留。
  _tabGeneration[tab_id] = tabGeneration(tab_id) + 1;
  if (_sessionByTab[tab_id] || _sessionLaunchesByTab[tab_id]?.size) {
    return stopSession(tab_id);
  }
  const teardown = _sessionTeardownByTab[tab_id];
  if (teardown) return teardown;
  clearSessionState(tab_id);
  return Promise.resolve();
}
export async function togglePanel(tab_id: string): Promise<void> {
  if (isOpen(tab_id)) await closePanel(tab_id);
  else openPanel(tab_id);
}
function hasPanelWidthState(tab_id: string): boolean {
  return Object.prototype.hasOwnProperty.call(_panelWidthByTab, tab_id);
}
export function panelWidth(tab_id: string): number | null {
  return hasPanelWidthState(tab_id) ? _panelWidthByTab[tab_id] : null;
}
export function setPanelWidth(tab_id: string, width: number | null) {
  if (!_disposedTabs.has(tab_id)) _panelWidthByTab[tab_id] = width;
}
/** Commit only at the end of a drag. The active tab keeps its own value; the
 * committed value seeds tabs created later and preserves the pre-per-tab
 * localStorage behavior across app restarts. */
export function commitPanelWidth(tab_id: string): boolean {
  if (_disposedTabs.has(tab_id) || !hasPanelWidthState(tab_id)) {
    if (_disposedTabs.has(tab_id)) delete _panelWidthByTab[tab_id];
    return false;
  }
  const width = _panelWidthByTab[tab_id];
  if (width === null) {
    _initialPanelWidth = null;
    try {
      localStorage.removeItem(LEGACY_PANEL_WIDTH_KEY);
    } catch (error) {
      console.warn("[ai] clear panel width:", error);
    }
    return true;
  }
  if (!Number.isFinite(width) || width < MIN_PANEL_WIDTH) return false;
  _initialPanelWidth = width;
  try {
    localStorage.setItem(LEGACY_PANEL_WIDTH_KEY, String(width));
  } catch (error) {
    console.warn("[ai] persist panel width:", error);
  }
  return true;
}
export function discardPanelState(tab_id: string) {
  hidePanel(tab_id);
  delete _panelWidthByTab[tab_id];
  clearPrefill(tab_id);
}

/** addTab 的生命周期入口；同一个 id 即使被测试/调用方复用，旧异步任务也会因
 * generation 不同而失效。正常产品路径使用 UUID，不依赖复用。 */
export function activateTab(tab_id: string) {
  _tabGeneration[tab_id] = tabGeneration(tab_id) + 1;
  _disposedTabs.delete(tab_id);
  if (!hasPanelWidthState(tab_id)) {
    _panelWidthByTab[tab_id] = _initialPanelWidth;
  }
}

/** closeTab 的唯一 AI teardown：先同步封死后续异步 continuation，再清 UI/actor。 */
export async function disposeTab(tab_id: string): Promise<void> {
  _tabGeneration[tab_id] = tabGeneration(tab_id) + 1;
  _disposedTabs.add(tab_id);
  discardPanelState(tab_id);
  if (!_sessionByTab[tab_id] && !_sessionLaunchesByTab[tab_id]?.size) {
    clearSessionState(tab_id);
    const teardown = _sessionTeardownByTab[tab_id];
    if (teardown) await teardown;
    return;
  }
  await beginSessionTeardown(tab_id).barrier;
}

// ─── Input prefill ────────────────────────────────────────────────
// 把一段文本塞进某个 tab 的 ChatPanel 输入框（不发送）。色条"发送到 AI"用它：
// 抽块文本 → openPanel → prefillInput，让用户过目/编辑后再发。
// 每个 tab 独立一个槽；新对象 identity 保证同一段文本重复写入也能触发对应面板的 effect。
let _prefillByTab = $state<Record<string, { text: string }>>({});
export function prefillInput(tab_id: string, text: string) {
  _prefillByTab[tab_id] = { text };
}
export function pendingPrefill(tab_id: string) { return _prefillByTab[tab_id] ?? null; }
export function clearPrefill(tab_id: string) {
  delete _prefillByTab[tab_id];
}

// ─── Session ──────────────────────────────────────────────────────

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
export function tokenUsage(tab_id: string): TokenUsage {
  return _tokensByTab[tab_id] ?? { tokens_in: 0, tokens_out: 0 };
}

function pushChat(tab_id: string, item: ChatItem, persist = true) {
  const arr = _chatByTab[tab_id] ?? [];
  _chatByTab[tab_id] = [...arr, item];
  if (persist) schedulePersist(tab_id);
}

function removeOptimisticUserMessage(tab_id: string, client_id: string): void {
  const items = _chatByTab[tab_id];
  if (!items) return;
  const next = items.filter(
    (item) => item.kind !== "user" || item.client_id !== client_id,
  );
  if (next.length === items.length) return;
  _chatByTab[tab_id] = next;
  schedulePersist(tab_id);
}

function applyContextClear(
  session: SessionInstanceRef,
  clearSequence: number,
): void {
  if (_sessionByTab[session.tabId]?.instance_id !== session.instanceId) return;
  commandApprovals.clearSession(session);
  clearCommandExecutionsForSession(session);
  const items = _chatByTab[session.tabId] ?? [];
  // Sends called after ClearContext carry a larger sequence even if their
  // optimistic bubble was rendered before the actor processed the clear.
  // Preserve exactly those; every pre-clear assistant/command/note belongs to
  // the context the backend just discarded.
  _chatByTab[session.tabId] = items.filter(
    (item) => item.kind === "user"
      && item.client_seq !== undefined
      && item.client_seq > clearSequence,
  );
  _pendingByTab[session.tabId] = null;
  _keyboardLockedByTab[session.tabId] = false;
  schedulePersist(session.tabId);
}

function initializeContextEpoch(session: SessionInstanceRef): void {
  _contextEpochByTab[session.tabId] = {
    instanceId: session.instanceId,
    epoch: 0,
  };
}

function beginContextClear(session: SessionInstanceRef): PendingContextClear | null {
  if (_sessionByTab[session.tabId]?.instance_id !== session.instanceId) return null;
  const current = _contextEpochByTab[session.tabId];
  const pending: PendingContextClear = {
    instanceId: session.instanceId,
    targetEpoch: current?.instanceId === session.instanceId ? current.epoch + 1 : 1,
    bufferedEvents: [],
  };
  _pendingContextClearByTab[session.tabId] = pending;
  return pending;
}

function finishContextClear(
  session: SessionInstanceRef,
  pending: PendingContextClear | null,
  clearSequence: number,
): void {
  // A close can invalidate the session while the clear command is awaiting its
  // processing ack. Never recreate epoch state or replay buffered callbacks for
  // that dead actor.
  if (_sessionByTab[session.tabId]?.instance_id !== session.instanceId) {
    pending?.bufferedEvents.splice(0);
    return;
  }
  const current = _contextEpochByTab[session.tabId];
  const targetEpoch = pending?.targetEpoch
    ?? (current?.instanceId === session.instanceId ? current.epoch + 1 : 1);
  _contextEpochByTab[session.tabId] = {
    instanceId: session.instanceId,
    epoch: targetEpoch,
  };
  // Epoch installation and UI reset are one synchronous transaction from the
  // event loop's point of view. Only after both are complete may epoch+1 events
  // that outran the invoke response be released.
  applyContextClear(session, clearSequence);
  if (pending && _pendingContextClearByTab[session.tabId] === pending) {
    delete _pendingContextClearByTab[session.tabId];
  }
  for (const apply of pending?.bufferedEvents.splice(0) ?? []) apply();
}

function abandonContextClear(session: SessionInstanceRef, pending: PendingContextClear | null): void {
  if (!pending) return;
  pending.bufferedEvents.splice(0);
  if (_pendingContextClearByTab[session.tabId] === pending) {
    delete _pendingContextClearByTab[session.tabId];
  }
}

function contextEpochFromPayload(payload: unknown): number | null | undefined {
  if (
    !payload
    || typeof payload !== "object"
    || !Object.prototype.hasOwnProperty.call(payload, "context_epoch")
  ) return undefined;
  const epoch = (payload as { context_epoch?: unknown }).context_epoch;
  return typeof epoch === "number" && Number.isSafeInteger(epoch) && epoch >= 0
    ? epoch
    : null;
}

function stripContextEpoch<T extends object>(payload: T): T {
  if (!("context_epoch" in payload)) return payload;
  const copy = { ...payload } as T & { context_epoch?: unknown };
  delete copy.context_epoch;
  return copy;
}

function acceptsContextEvent(session: SessionInstanceRef, payload: unknown): boolean {
  if (_sessionByTab[session.tabId]?.instance_id !== session.instanceId) return false;
  const eventEpoch = contextEpochFromPayload(payload);
  // Compatibility with older backends that emitted no context_epoch field.
  if (eventEpoch === undefined) return true;
  if (eventEpoch === null) return false;
  const current = _contextEpochByTab[session.tabId];
  return current?.instanceId === session.instanceId && eventEpoch === current.epoch;
}

function dispatchContextEvent(
  session: SessionInstanceRef,
  payload: unknown,
  apply: () => void,
): void {
  if (_sessionByTab[session.tabId]?.instance_id !== session.instanceId) return;
  const eventEpoch = contextEpochFromPayload(payload);
  if (eventEpoch === undefined) {
    // Compatibility with older backends. Without an epoch there is no safe way
    // to distinguish pre/post-clear delivery, so preserve the legacy behavior.
    apply();
    return;
  }
  if (eventEpoch === null) return;
  const current = _contextEpochByTab[session.tabId];
  if (current?.instanceId !== session.instanceId || eventEpoch < current.epoch) return;
  const pending = _pendingContextClearByTab[session.tabId];
  if (
    pending?.instanceId === session.instanceId
    && eventEpoch >= pending.targetEpoch
  ) {
    // Rust can send the processing ack before it emits the next event, while
    // the two IPC channels can deliver them in the opposite order. Buffer only
    // the new epoch; current-epoch events may render briefly and are then
    // removed by the acknowledged clear.
    pending.bufferedEvents.push(apply);
    return;
  }
  if (eventEpoch !== current.epoch) return;
  apply();
}

function trackConversationMutation(
  tab_id: string,
  mutation: PendingConversationMutation,
): void {
  const mutations = _pendingConversationMutationsByTab[tab_id]
    ?? new Set<PendingConversationMutation>();
  _pendingConversationMutationsByTab[tab_id] = mutations;
  mutations.add(mutation);
}

function untrackConversationMutation(
  tab_id: string,
  mutation: PendingConversationMutation,
): void {
  const mutations = _pendingConversationMutationsByTab[tab_id];
  if (!mutations) return;
  mutations.delete(mutation);
  if (mutations.size === 0 && _pendingConversationMutationsByTab[tab_id] === mutations) {
    delete _pendingConversationMutationsByTab[tab_id];
  }
}

function enqueueConversationMutation(
  tab_id: string,
  run: () => Promise<void>,
): Promise<void> {
  const previous = _conversationMutationTailByTab[tab_id];
  // Start the first action synchronously so a following close cannot overtake
  // an action already initiated in the same turn. Later actions wait for the
  // prior action to settle; rejection must not poison the queue.
  const operation = previous ? previous.then(run, run) : run();
  const tail = operation.catch(() => undefined);
  _conversationMutationTailByTab[tab_id] = tail;
  void tail.then(() => {
    if (_conversationMutationTailByTab[tab_id] === tail) {
      delete _conversationMutationTailByTab[tab_id];
    }
  });
  return operation;
}

// ─── Timeline 自动保存 ─────────────────────────────────────────────
// 后端 actor 自己保存 LLM history；UI timeline 归前端管，在每个改动 chat
// 数组的事件后落库。300ms 防抖把一轮 turn 的事件突发（user_message →
// command_proposed → completed → message_end）合并成一两次写。
// fire-and-forget：持久化是旁路功能，写失败只记 console，不打断对话。

const _persistTimers: Record<string, ReturnType<typeof setTimeout>> = {};
const _persistWritesByTab: Record<string, Promise<void>> = {};

function serializeTimeline(items: ChatItem[]): string {
  // client_id/client_seq are live correlation metadata for pending mutations,
  // not conversation data. Keeping them out of storage also prevents a fresh
  // runtime's sequence counter from comparing against stale persisted values.
  return JSON.stringify(items.map((item) => item.kind === "user"
    ? { kind: "user", text: item.text, at: item.at }
    : item));
}

function queueTimelinePersist(tab_id: string, id: string, timeline: string): Promise<void> {
  // DB writes for one conversation must stay ordered. Otherwise an older slow
  // autosave can finish after the close-time flush and overwrite the final UI
  // timeline with stale content.
  const previous = _persistWritesByTab[tab_id] ?? Promise.resolve();
  const result = previous
    .then(() => invoke("ai_conversation_save_timeline", { id, timeline }))
    .then(() => undefined);
  // Ordering chains must always settle so a failed autosave cannot poison every
  // later write. Return the raw result separately: explicit close owns the final
  // snapshot and must surface its failure after the rest of teardown completes.
  const settled = result.catch((e) => {
    console.error("[ai] persist timeline:", e);
  });
  _persistWritesByTab[tab_id] = settled;
  void settled.then(() => {
    if (_persistWritesByTab[tab_id] === settled) delete _persistWritesByTab[tab_id];
  });
  return result;
}

function schedulePersist(tab_id: string) {
  if (!_sessionByTab[tab_id]) return;
  clearTimeout(_persistTimers[tab_id]);
  _persistTimers[tab_id] = setTimeout(() => {
    delete _persistTimers[tab_id];
    const id = _sessionByTab[tab_id]?.conversation_id;
    const items = _chatByTab[tab_id];
    if (!id || !items) return;
    void queueTimelinePersist(tab_id, id, serializeTimeline(items));
  }, 300);
}

/** 该 profile / 端口下持久化过的历史对话，最近活跃在前。 */
export async function listConversations(
  target_kind: AiTargetKind,
  target_id: string,
): Promise<ConversationMeta[]> {
  return invoke<ConversationMeta[]>("ai_conversations_list", {
    target: { kind: target_kind, id: target_id },
  });
}

export async function deleteConversation(id: string): Promise<void> {
  await invoke("ai_conversation_delete", { id });
}

// ─── Lifecycle ────────────────────────────────────────────────────

export interface StartSessionArgs {
  tabId: string;
  targetKind: AiTargetKind;
  targetId: string;
  skill: string;
  provider: string;
  model: string;
  lease: SessionLease;
}

export async function startSession(args: StartSessionArgs): Promise<AiSessionInfo> {
  return launchSession(args, null);
}

/** 恢复持久化的对话：actor 带旧 history 出生，UI 灌回存储的 timeline。 */
export async function resumeSession(
  args: StartSessionArgs,
  conversationId: string,
): Promise<AiSessionInfo> {
  return launchSession(args, conversationId);
}

async function launchSession(
  args: StartSessionArgs,
  resumeId: string | null,
): Promise<AiSessionInfo> {
  const generation = generationForLease(args.tabId, args.lease);
  assertTabLive(args.tabId, generation);
  // 面板可能刚关闭又打开。旧 actor 未 stop 完之前，同 tab_id 启新 actor
  // 会撞 session_already_exists；先过 teardown barrier，再确认期间没再次关闭。
  const teardown = _sessionTeardownByTab[args.tabId];
  if (teardown) {
    await teardown;
    assertTabLive(args.tabId, generation);
  }
  const launch = launchSessionAtGeneration(args, resumeId, generation);
  const launches = _sessionLaunchesByTab[args.tabId] ?? new Set<Promise<AiSessionInfo>>();
  _sessionLaunchesByTab[args.tabId] = launches;
  launches.add(launch);
  try {
    return await launch;
  } finally {
    launches.delete(launch);
    if (launches.size === 0 && _sessionLaunchesByTab[args.tabId] === launches) {
      delete _sessionLaunchesByTab[args.tabId];
    }
  }
}

async function launchSessionAtGeneration(
  args: StartSessionArgs,
  resumeId: string | null,
  generation: number,
): Promise<AiSessionInfo> {
  let info: AiSessionInfo;
  try {
    info = await invoke<AiSessionInfo>("ai_session_start", {
      tabId: args.tabId,
      target: { kind: args.targetKind, id: args.targetId },
      skill: args.skill,
      provider: args.provider,
      model: args.model,
      locale: currentLocale(),
      resume: resumeId,
    });
  } catch (error) {
    // Closing can cancel a Rust Pending reservation before start returns. The
    // backend then reports reservation_lost; expose the stable frontend
    // lifecycle meaning instead of leaking that implementation detail.
    if (!isTabLive(args.tabId, generation)) throw new SessionClosedError(args.tabId);
    throw error;
  }
  if (!isTabLive(args.tabId, generation)) {
    // close 可能先打到 not_found、start 随后才成功；启动返回后必须再 stop 一次。
    await stopFailedLaunch(info, "abandoned session");
    throw new SessionClosedError(args.tabId);
  }
  try {
    // Resume must claim/activate the conversation before reading its UI blob.
    // Otherwise another tab can read a stale snapshot while the old owner is
    // closing, then win the lease later and overwrite the old owner's final UI.
    let timeline: ChatItem[] = [];
    if (resumeId) {
      const json = await invoke<string>("ai_conversation_timeline", {
        id: resumeId,
        target: { kind: args.targetKind, id: args.targetId },
      });
      assertTabLive(args.tabId, generation);
      timeline = restoreTimeline(json, t("ai.history.stale_command"));
    }
    // info.tab_id 后端权威 —— 跟 args.tabId 一定一致（后端按入参 insert），但用
    // 后端返回值就消除"未来后端 normalize tab_id"导致 cache miss 的隐患。
    _sessionByTab[info.tab_id] = info;
    _targetKindByTab[info.tab_id] = args.targetKind;
    _chatByTab[info.tab_id] = timeline;
    initializeContextEpoch({ tabId: info.tab_id, instanceId: info.instance_id });
    await attachListeners(info, generation);
    assertTabLive(args.tabId, generation);
    return info;
  } catch (error) {
    clearSessionState(info.tab_id);
    await stopFailedLaunch(info, "failed launch");
    if (!isTabLive(args.tabId, generation)) throw new SessionClosedError(args.tabId);
    throw error;
  }
}

async function stopFailedLaunch(info: AiSessionInfo, label: string): Promise<void> {
  // Explicit close owns backend stop and may be holding the conversation lease
  // until its final timeline write settles. Do not release that lease early or
  // wait on the full teardown (which itself waits for this launch).
  if (_sessionTeardownByTab[info.tab_id]) return;
  await invoke("ai_session_stop", {
    tabId: info.tab_id,
    instanceId: info.instance_id,
  }).catch((e) => console.warn(`[ai] stop ${label}:`, e));
}

function clearSessionState(tab_id: string) {
  const session = _sessionByTab[tab_id];
  if (session) {
    commandApprovals.clearSession({ tabId: tab_id, instanceId: session.instance_id });
  }
  detachListeners(tab_id);
  delete _sessionByTab[tab_id];
  delete _pendingByTab[tab_id];
  delete _keyboardLockedByTab[tab_id];
  delete _targetKindByTab[tab_id];
  delete _contextEpochByTab[tab_id];
  const pendingClear = _pendingContextClearByTab[tab_id];
  pendingClear?.bufferedEvents.splice(0);
  delete _pendingContextClearByTab[tab_id];
  delete _chatByTab[tab_id];
  delete _tokensByTab[tab_id];
}

export function stopSession(tab_id: string): Promise<void> {
  return beginSessionTeardown(tab_id).outcome;
}

function beginSessionTeardown(tab_id: string): {
  outcome: Promise<void>;
  barrier: Promise<void>;
} {
  const existing = _sessionTeardownByTab[tab_id];
  if (existing) return { outcome: existing, barrier: existing };
  const outcome = stopSessionNow(tab_id);
  // Coordination consumers need completion, not the initiating close's error.
  // Keeping only this always-settled promise in the map prevents an old final-
  // save failure from poisoning rapid reopen, dispose, or a later close.
  const barrier: Promise<void> = outcome.catch(() => undefined).finally(() => {
    if (_sessionTeardownByTab[tab_id] === barrier) delete _sessionTeardownByTab[tab_id];
  });
  _sessionTeardownByTab[tab_id] = barrier;
  return { outcome, barrier };
}

async function stopSessionNow(tab_id: string): Promise<void> {
  const session = _sessionByTab[tab_id];
  const launches = Array.from(_sessionLaunchesByTab[tab_id] ?? []);
  const pendingMutations = Array.from(
    _pendingConversationMutationsByTab[tab_id] ?? [],
  );

  // Always flush the final visible snapshot before clearing state. Streaming
  // deltas mutate the current bubble in place and intentionally do not restart
  // the 300ms debounce timer per token, so "no pending timer" does not mean the
  // last completed autosave is current.
  const priorPersist = _persistWritesByTab[tab_id] ?? Promise.resolve();
  if (_persistTimers[tab_id]) {
    clearTimeout(_persistTimers[tab_id]);
    delete _persistTimers[tab_id];
  }
  const items = _chatByTab[tab_id];

  // UI reset is synchronous: a rapid reopen must never flash the old session.
  // Backend termination continues below.
  clearSessionState(tab_id);

  const abandonedLaunches = launches.map((launch) =>
    launch.then(() => undefined, () => undefined),
  );

  // A session actor can be blocked waiting for the current tool outcome. Deliver
  // that outcome before asking it to shut down; raw devices abort without any
  // transport write, while shell transports retain their explicit Ctrl+C path.
  const executionEntries = Array.from(_commandExecutions.entries()).filter(
    ([, execution]) => execution.tabId === tab_id
      && (!session || execution.instanceId === session.instance_id),
  );
  const executionResults = await Promise.allSettled(
    executionEntries.map(([, execution]) => execution.teardown()),
  );
  for (const [key, execution] of executionEntries) {
    removeCommandExecution(key, execution);
  }

  // Signal-only shutdown keeps the actor and conversation lease alive. It
  // unblocks tool waits but deliberately does not join/remove the actor; pending
  // Message/ClearContext processing acknowledgements and the final UI timeline
  // must settle before the full stop releases ownership.
  const prepareResults = session
    ? await Promise.allSettled([
        invoke<AiTerminalMutation[]>("ai_session_prepare_stop", {
          tabId: tab_id,
          instanceId: session.instance_id,
        }),
      ])
    : [];
  const terminalMutations = prepareResults[0]?.status === "fulfilled"
    && Array.isArray(prepareResults[0].value)
    ? prepareResults[0].value
    : [];

  const mutationResults = await Promise.allSettled(
    pendingMutations.map((pending) => pending.operation),
  );
  const failedClientIds = new Set<string>();
  let latestSuccessfulClear = -1;
  mutationResults.forEach((result, index) => {
    const mutation = pendingMutations[index];
    if (result.status === "rejected" && mutation.kind === "send") {
      failedClientIds.add(mutation.clientId);
    } else if (result.status === "fulfilled" && mutation.kind === "clear") {
      latestSuccessfulClear = Math.max(latestSuccessfulClear, mutation.sequence);
    }
  });
  const filteredItems = items?.filter(
    (item) => {
      if (item.kind !== "user") return latestSuccessfulClear < 0;
      if (item.client_id && failedClientIds.has(item.client_id)) return false;
      return latestSuccessfulClear < 0
        || (item.client_seq !== undefined && item.client_seq > latestSuccessfulClear);
    },
  );
  const finalItems = filteredItems
    ? applyTerminalMutations(filteredItems, terminalMutations)
    : filteredItems;
  const persist = session?.conversation_id && finalItems
    ? queueTimelinePersist(tab_id, session.conversation_id, serializeTimeline(finalItems))
    : priorPersist;
  const [persistResult] = await Promise.allSettled([persist]);

  // Releasing the actor also releases its conversation lease. The final UI
  // timeline must be durable first; a failed write still proceeds with stop,
  // then is surfaced only to the close caller after every teardown operation.
  const stop = invoke<void>("ai_session_stop", {
    tabId: tab_id,
    ...(session ? { instanceId: session.instance_id } : {}),
  });
  // A launch can still be in pre-activation work and have no backend reservation.
  // In that ordering, not_found is expected; generation invalidation makes the
  // launch abort, while the barrier still waits for it to settle.
  const backendStop = session
    ? stop
    : stop.catch((error) => {
        if (isAiSessionNotFound(error)) return;
        throw error;
      });
  const [backendResult, launchResults] = await Promise.all([
    Promise.allSettled([backendStop]).then(([result]) => result),
    Promise.allSettled(abandonedLaunches),
  ]);
  const results: PromiseSettledResult<unknown>[] = [
    ...executionResults,
    ...prepareResults,
    ...mutationResults,
    persistResult,
    backendResult,
    ...launchResults,
  ];
  if (launches.length > 0) {
    // The first instance-less stop can miss a start that has not reserved its
    // backend slot yet. Once every captured launch settles, no such ABA window
    // remains; the frontend teardown barrier still blocks any legitimate
    // replacement, so one final tab sweep is both necessary and safe.
    const [sweepResult] = await Promise.allSettled([
      invoke<void>("ai_session_stop", { tabId: tab_id }),
    ]);
    if (sweepResult.status === "fulfilled" || !isAiSessionNotFound(sweepResult.reason)) {
      results.push(sweepResult);
    }
  }
  const failure = results.find(
    (result): result is PromiseRejectedResult => result.status === "rejected",
  );
  if (failure) throw failure.reason;
}

function sessionInstanceForLease(tab_id: string, lease: SessionLease): SessionInstanceRef {
  assertLeaseLive(tab_id, lease);
  const info = _sessionByTab[tab_id];
  if (!info) throw new SessionClosedError(tab_id);
  return { tabId: tab_id, instanceId: info.instance_id };
}

export async function sendMessage(tab_id: string, text: string, lease: SessionLease) {
  const session = sessionInstanceForLease(tab_id, lease);
  const sequence = ++_nextConversationMutationId;
  const clientId = `${session.instanceId}:${sequence}`;
  pushChat(tab_id, {
    kind: "user",
    client_id: clientId,
    client_seq: sequence,
    text,
    at: Date.now(),
  }, false);
  const operation = enqueueConversationMutation(tab_id, async () => {
    try {
      await invoke("ai_user_message", {
        tabId: session.tabId,
        instanceId: session.instanceId,
        text,
      });
      // The backend resolves only after the actor processed and persisted this
      // message. The optimistic bubble becomes durable on that processing ack.
      // Close owns its captured final snapshot, so a send that settles during
      // teardown must not recreate an autosave timer against cleared UI state.
      if (
        isTabLive(tab_id, lease.generation)
        && _sessionByTab[tab_id]?.instance_id === session.instanceId
      ) {
        schedulePersist(tab_id);
      }
    } catch (error) {
      // Exact client correlation, never text matching: equal consecutive user
      // messages are distinct, and only the rejected enqueue is rolled back.
      removeOptimisticUserMessage(tab_id, clientId);
      throw error;
    }
  });
  const pending: PendingConversationMutation = {
    kind: "send",
    sequence,
    clientId,
    operation,
  };
  trackConversationMutation(tab_id, pending);
  try {
    await operation;
  } finally {
    untrackConversationMutation(tab_id, pending);
  }
}

/** 清空 actor 的对话历史（audit log 保留）。actor 不死，下条消息从头来过。 */
export async function clearContext(tab_id: string, lease: SessionLease): Promise<void> {
  const session = sessionInstanceForLease(tab_id, lease);
  const sequence = ++_nextConversationMutationId;
  const operation = enqueueConversationMutation(tab_id, async () => {
    const pending = beginContextClear(session);
    try {
      await invoke("ai_session_clear_context", {
        tabId: session.tabId,
        instanceId: session.instanceId,
      });
      // Like user messages, the backend response is a processing ack. Frontend
      // owns the matching UI mutation. Epoch installation + reset happens
      // synchronously, then any new-epoch events that outran this response run.
      finishContextClear(session, pending, sequence);
    } catch (error) {
      abandonContextClear(session, pending);
      throw error;
    }
  });
  const pending: PendingConversationMutation = {
    kind: "clear",
    sequence,
    operation,
  };
  trackConversationMutation(tab_id, pending);
  try {
    await operation;
  } finally {
    untrackConversationMutation(tab_id, pending);
  }
}

/** SSH 重连后调用：让 actor 内部把 target_id + ssh_handle 切到新 SSH 连接。 */
export async function rebindTarget(
  tab_id: string,
  target_kind: AiTargetKind,
  target_id: string,
  lease: SessionLease,
): Promise<void> {
  const generation = generationForLease(tab_id, lease);
  assertTabLive(tab_id, generation);
  const bound = _sessionByTab[tab_id];
  if (!bound) throw new SessionClosedError(tab_id);
  const conversationId = bound.conversation_id;
  await invoke("ai_session_rebind_target", {
    tabId: tab_id,
    instanceId: bound.instance_id,
    target: { kind: target_kind, id: target_id },
    conversationId,
  });
  assertTabLive(tab_id, generation);
  // 同步前端 cache：AiSessionInfo.target_id 也要换，否则下次 sendMessage 走的
  // executeCommand 还会用旧 target_session_id 给 ssh_write —— 拿不到新 PTY。
  const info = _sessionByTab[tab_id];
  if (
    !info
    || info.instance_id !== bound.instance_id
    || info.conversation_id !== conversationId
  ) throw new SessionClosedError(tab_id);
  _sessionByTab[tab_id] = { ...info, target_id };
}

/** 打断 actor 正在跑的 LLM 流式响应。会话上下文（history / pending command / audit）全部保留——
 *  这跟 stopSession（销毁整个会话）是两个语义。actor 不在 chat 时调用是 no-op。 */
export async function cancelStream(tab_id: string, lease: SessionLease): Promise<void> {
  const session = sessionInstanceForLease(tab_id, lease);
  await invoke("ai_cancel_stream", {
    tabId: session.tabId,
    instanceId: session.instanceId,
  });
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

export type CommandExecutionStatus =
  | "running"
  | "reporting"
  | "delivery_failed"
  | "delivered";

type CommandResultReport = {
  tabId: string;
  instanceId: string;
  toolCallId: string;
  exitCode: number;
  output: string;
  timedOut: boolean;
  earlyTerminated: boolean;
};

/** Per-command-card transport + result-delivery state. The entry outlives the PTY
 * run when result delivery fails, because an executed command must never become
 * executable again merely because its acknowledgement could not be stored. */
type Execution = {
  key: string;
  commandId: string;
  tabId: string;
  instanceId: string;
  targetSessionId: string;
  targetKind: AiTargetKind;
  buffer: CappedBuffer;
  status: CommandExecutionStatus;
  result: CommandResultReport | null;
  delivery: Promise<void> | null;
  done: Promise<void>;
  userInterrupted: boolean;
  unlisten: UnlistenFn | null;
  timer: number | null;
  terminate: () => Promise<void>;
  /** Raw devices (serial/telnet) only: user says "done" — report the buffer
   *  as a clean result. */
  submit: () => Promise<void>;
  deliver: () => Promise<void>;
  teardown: () => Promise<void>;
  dispose: () => void;
};

/**
 * Indexed by actor instance + command card id. One provider tool call can emit
 * several sequential cards, so the card id is the only execution correlation.
 * Map keeps transport commitment and result delivery in one exact identity;
 * entries are cleared only by backend completion/rejection or session teardown.
 */
const _commandExecutions: Map<string, Execution> = new Map();
let _commandExecutionStatusByKey = $state<Record<string, CommandExecutionStatus>>({});

function setCommandExecutionStatus(
  execution: Execution,
  status: CommandExecutionStatus,
): void {
  execution.status = status;
  // A processing ack can resolve after the matching command_completed event
  // already removed this execution. Keep the detached object coherent for its
  // awaiting caller, but never resurrect an orphaned reactive status entry.
  if (_commandExecutions.get(execution.key) === execution) {
    _commandExecutionStatusByKey[execution.key] = status;
  }
}

function removeCommandExecution(key: string, expected?: Execution): void {
  const execution = _commandExecutions.get(key);
  if (!execution || (expected && execution !== expected)) return;
  execution.dispose();
  _commandExecutions.delete(key);
  delete _commandExecutionStatusByKey[key];
}

export function isCommandRunning(session: SessionInstanceRef, command_id: string): boolean {
  return commandExecutionStatus(session, command_id) === "running";
}

export function commandExecutionStatus(
  session: SessionInstanceRef,
  command_id: string,
): CommandExecutionStatus | null {
  return _commandExecutionStatusByKey[sessionCommandKey(session, command_id)] ?? null;
}

function clearCommandExecution(session: SessionInstanceRef, command_id: string): void {
  const key = sessionCommandKey(session, command_id);
  removeCommandExecution(key);
}

function clearCommandExecutionsForSession(session: SessionInstanceRef): void {
  for (const [key, execution] of _commandExecutions) {
    if (execution.tabId !== session.tabId || execution.instanceId !== session.instanceId) continue;
    removeCommandExecution(key, execution);
  }
}

/**
 * Execute an AI-proposed command: paste `full_cmd` (with sentinel +
 * exit-code echo) into the active terminal, watch the PTY stream for
 * the sentinel, then report output + exit code to the backend. All
 * front-end; the backend's ai module never executes commands itself.
 */
export async function executeCommand(
  session: SessionInstanceRef,
  proposed: CommandProposed,
  target_kind: AiTargetKind,
  target_session_id: string,
): Promise<void> {
  // Re-entrancy guard: a command card must never be pasted twice. A
  // CommandConfirmDialog remount can lose its local `executing` flag and fire
  // approve() again; without this, the command
  // (possibly rm/reboot) would be pasted a second time and the first exec's
  // listener + timer would leak when `_commandExecutions.set` below overwrites
  // the entry. The map is the single source of truth for "in flight" — honor it.
  // Once transport has run, a retry can only redeliver its recorded result.
  const executionKey = sessionCommandKey(session, proposed.id);
  const existing = _commandExecutions.get(executionKey);
  if (existing) {
    switch (existing.status) {
      case "running": return existing.done;
      case "reporting": return existing.delivery ?? existing.done;
      case "delivery_failed": return existing.deliver();
      case "delivered": return;
    }
  }

  // Transport per kind. Record<AiTargetKind, …> so adding a kind is a compile
  // error here until routed — no silent fall-through to the wrong write command.
  const TRANSPORT: Record<AiTargetKind, { write: string; data: string }> = {
    ssh:    { write: "ssh_write",    data: "ssh:data" },
    local:  { write: "pty_write",    data: "pty:data" },
    serial: { write: "serial_write", data: "serial:data" },
    telnet: { write: "telnet_write", data: "telnet:data" },
  };
  const writeCmd = TRANSPORT[target_kind].write;
  const dataEvent = `${TRANSPORT[target_kind].data}:${target_session_id}`;
  // A raw device may not echo the command back (depends on the device /
  // local-echo / telnet ECHO negotiation), so dropping the first line would
  // silently eat real output. Keep the whole buffer for raw devices; an
  // echoed-command line is harmless noise the LLM ignores.
  const dropEcho = !isRawDeviceKind(target_kind);

  // Returned Promise resolves only when finish() actually runs, so the
  // UI's "executing" state can cover the whole execution window — not
  // just up to the `invoke(writeCmd)` round-trip.
  let resolveDone!: () => void;
  let rejectDone!: (error: unknown) => void;
  const done = new Promise<void>((resolve, reject) => {
    resolveDone = resolve;
    rejectDone = reject;
  });
  let finish!: (output: string, exit_code: number, timed_out: boolean) => Promise<void>;
  let deliver!: () => Promise<void>;

  const exec: Execution = {
    key: executionKey,
    commandId: proposed.id,
    tabId: session.tabId,
    instanceId: session.instanceId,
    targetSessionId: target_session_id,
    targetKind: target_kind,
    buffer: new CappedBuffer(),
    status: "running",
    result: null,
    delivery: null,
    done,
    userInterrupted: false,
    unlisten: null,
    timer: null,
    terminate: async () => {
      if (exec.status !== "running") return exec.teardown();
      exec.userInterrupted = true;
      // A serial/telnet peer is not a shell. Injecting ETX while closing the UI
      // can reboot or reconfigure a bare device; raw teardown only detaches and
      // reports interruption. Shell transports retain the explicit Ctrl+C path.
      if (!isRawDeviceKind(exec.targetKind)) {
        const ctrlC = Array.from(new TextEncoder().encode("\x03"));
        void invoke(writeCmd, { sessionId: target_session_id, data: ctrlC })
            .catch((err) => console.warn("[ai] terminate Ctrl+C failed:", err));
      }
      await finish(extractOutput(exec.buffer.view(), undefined, dropEcho), -1, false);
    },
    submit: async () => {
      if (exec.status !== "running") return;
      // Raw-device completion: the user watched the device and says "done".
      // Report the accumulated output as a NORMAL result — no Ctrl+C (nothing
      // to interrupt), not flagged early-terminated, exit 0 as the placeholder
      // (no exit code exists; the LLM is told via prompt to judge by output).
      await finish(extractOutput(exec.buffer.view(), undefined, dropEcho), 0, false);
    },
    deliver: () => deliver(),
    teardown: () => {
      switch (exec.status) {
        case "running": return exec.terminate();
        case "reporting": return exec.delivery ?? exec.done;
        case "delivery_failed": return exec.deliver();
        case "delivered": return Promise.resolve();
      }
    },
    dispose: () => {
      if (exec.unlisten) {
        exec.unlisten();
        exec.unlisten = null;
      }
      if (exec.timer != null) {
        clearTimeout(exec.timer);
        exec.timer = null;
      }
    },
  };

  // Reserve the in-flight slot synchronously, BEFORE any await. The guard at the
  // top of this function only holds if check-and-reserve is atomic. This set used
  // to live after `await listen()` below, leaving a window where a second call
  // (dialog remount re-firing approve, a double-click) passed the guard before
  // this ran and pasted the command — possibly rm/reboot — twice. There is no
  // await between the guard and here, so the reservation closes that window.
  _commandExecutions.set(executionKey, exec);
  _commandExecutionStatusByKey[executionKey] = "running";

  deliver = () => {
    if (!exec.result || exec.status === "delivered") return Promise.resolve();
    if (exec.delivery) return exec.delivery;
    setCommandExecutionStatus(exec, "reporting");
    const operation = invoke("ai_command_result", exec.result)
      .then(() => {
        setCommandExecutionStatus(exec, "delivered");
      })
      .catch((error) => {
        setCommandExecutionStatus(exec, "delivery_failed");
        console.error("[ai] ai_command_result failed:", error);
        throw error;
      });
    const tracked = operation.finally(() => {
      if (exec.delivery === tracked) exec.delivery = null;
    });
    exec.delivery = tracked;
    return tracked;
  };

  finish = async (output: string, exit_code: number, timed_out: boolean) => {
    if (exec.status !== "running") return exec.done;
    exec.dispose();
    exec.result = {
      tabId: session.tabId,
      instanceId: session.instanceId,
      toolCallId: exec.commandId,
      exitCode: exit_code,
      output,
      timedOut: timed_out,
      earlyTerminated: exec.userInterrupted,
    };
    void deliver().then(resolveDone, rejectDone);
    return exec.done;
  };

  try {
    const unlisten = await listen<number[]>(dataEvent, (e) => {
      if (exec.status !== "running") return;
      const chunk = new TextDecoder("utf-8", { fatal: false }).decode(new Uint8Array(e.payload));
      exec.buffer.append(chunk);
      // Raw devices (serial/telnet) have no sentinel — just accumulate.
      // Completion comes from the user (submit) or the safety timeout below.
      if (isRawDeviceKind(target_kind)) return;
      const hit = findSentinel(exec.buffer.view(), proposed.sentinel);
      if (hit) void finish(hit.output, hit.exitCode, false);
    });
    // close/terminate may have won while listen() was registering. Never paste
    // after that point, and do not leak the listener that arrived too late.
    if (exec.status !== "running") {
      unlisten();
      return done;
    }
    exec.unlisten = unlisten;
  } catch (e) {
    if (exec.status !== "running") return done;
    // listen() failed to register: release the slot reserved above so this
    // command card isn't wedged "in flight" forever.
    removeCommandExecution(executionKey, exec);
    throw new CommandSetupError(e);
  }

  // \r (not \n) is the cross-platform PTY/serial Enter byte: ConPTY/PowerShell
  // only accepts \r; Unix cooked PTY translates \r → \n via ICRNL. Telnet is
  // different: its handle owns the profile's line discipline, so delegate line
  // termination to telnet_write_line instead of duplicating a CRLF default here.
  // If invoke throws (session already closed), listener + execution are
  // already registered → must funnel through finish() to clean up, else
  // isCommandRunning() stays true forever.
  try {
    if (target_kind === "telnet") {
      await invoke("telnet_write_line", { sessionId: target_session_id, text: proposed.full_cmd });
    } else {
      const data = Array.from(new TextEncoder().encode(proposed.full_cmd + "\r"));
      await invoke(writeCmd, { sessionId: target_session_id, data });
    }
  } catch (e) {
    if (exec.status !== "running") return done;
    await finish(`failed to write command: ${errMsg(e)}`, -1, false);
    throw e;
  }

  // terminate can also win while the transport write itself is in flight.
  // finish() already cleaned the listener/map; do not resurrect a timeout.
  if (exec.status !== "running") return done;
  exec.timer = window.setTimeout(() => {
    void finish(extractOutput(exec.buffer.view(), undefined, dropEcho), -1, true);
  }, Math.max(1000, proposed.timeout_s * 1000)) as unknown as number;

  return done;
}

/** Early-terminate one command card: Ctrl+C to target shell + finish(). */
export async function terminateCommand(
  session: SessionInstanceRef,
  command_id: string,
): Promise<void> {
  const exec = _commandExecutions.get(sessionCommandKey(session, command_id));
  if (exec) await exec.terminate();
}

/**
 * Raw-device completion by command card id: the user signals the command is done,
 * so report the accumulated output as a clean result (no Ctrl+C, not early-
 * terminated). On serial/telnet the card shows "submit output"; on ssh/local
 * the equivalent slot is "interrupt" (terminate).
 */
export async function submitCommand(
  session: SessionInstanceRef,
  command_id: string,
): Promise<void> {
  const exec = _commandExecutions.get(sessionCommandKey(session, command_id));
  if (exec) await exec.submit();
}

export async function rejectCommand(
  session: SessionInstanceRef,
  command_id: string,
  reason: string,
) {
  await invoke("ai_command_reject", {
    tabId: session.tabId,
    instanceId: session.instanceId,
    toolCallId: command_id,
    reason,
  });
}

export async function getAudit(session: SessionInstanceRef): Promise<AuditLog> {
  return invoke<AuditLog>("ai_audit_get", {
    tabId: session.tabId,
    instanceId: session.instanceId,
  });
}

export async function saveAudit(session: SessionInstanceRef, file_path: string) {
  return invoke("ai_audit_save", {
    tabId: session.tabId,
    instanceId: session.instanceId,
    filePath: file_path,
  });
}

/** 拿审计 .log 文本，用统一的 saveTextFile 存盘（桌面 / 移动 / 浏览器一套）。 */
export async function saveAuditWithDialog(session: SessionInstanceRef): Promise<string | null> {
  const text = await invoke<string>("ai_audit_log_text", {
    tabId: session.tabId,
    instanceId: session.instanceId,
  });
  return saveTextFile(text, {
    defaultName: `rssh-diagnose-${session.tabId.slice(0, 8)}-${fileStamp()}.log`,
    filters: [{ name: "Log", extensions: ["log", "txt"] }],
  });
}

// ─── Settings ─────────────────────────────────────────────────────

export function settings() { return _settings; }

type AiSettingsPatch = Partial<{
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
}>;

type AutoApprovalSettingKey =
  | "danger_mode"
  | "auto_run_command"
  | "auto_match_file"
  | "auto_download_file"
  | "auto_analyze_locally"
  | "auto_patch_cp"
  | "auto_patch_modify"
  | "auto_patch_diff"
  | "auto_patch_mv";

const AUTO_APPROVAL_PATCH_FIELDS = [
  ["dangerMode", "danger_mode"],
  ["autoRunCommand", "auto_run_command"],
  ["autoMatchFile", "auto_match_file"],
  ["autoDownloadFile", "auto_download_file"],
  ["autoAnalyzeLocally", "auto_analyze_locally"],
  ["autoPatchCp", "auto_patch_cp"],
  ["autoPatchModify", "auto_patch_modify"],
  ["autoPatchDiff", "auto_patch_diff"],
  ["autoPatchMv", "auto_patch_mv"],
] as const satisfies ReadonlyArray<readonly [keyof AiSettingsPatch, AutoApprovalSettingKey]>;

// A disable is a safety action, not merely a persisted preference. Keep its
// local fail-closed overlay across concurrent reloads and save failures; only a
// later explicit successful enable removes the corresponding suspension.
const _autoApprovalSuspensions = new Set<AutoApprovalSettingKey>();

function applyAutoApprovalSuspensions(settings: AiSettings): AiSettings {
  if (_autoApprovalSuspensions.size === 0) return settings;
  const effective = { ...settings };
  for (const key of _autoApprovalSuspensions) effective[key] = false;
  return effective;
}

function installGlobalSettings(snapshot: AiSettings): AiSettings {
  const effective = applyAutoApprovalSuspensions(snapshot);
  _settings = effective;
  revokeDisallowedCommandApprovals(effective);
  return effective;
}

function suspendDisabledAutoApprovals(patch: AiSettingsPatch): void {
  let changed = false;
  for (const [patchKey, settingsKey] of AUTO_APPROVAL_PATCH_FIELDS) {
    if (patch[patchKey] === false) {
      _autoApprovalSuspensions.add(settingsKey);
      changed = true;
    }
  }
  if (changed && _settings) installGlobalSettings(_settings);
}

function releaseEnabledAutoApprovals(patch: AiSettingsPatch): void {
  for (const [patchKey, settingsKey] of AUTO_APPROVAL_PATCH_FIELDS) {
    if (patch[patchKey] === true) _autoApprovalSuspensions.delete(settingsKey);
  }
}

function revokeDisallowedCommandApprovals(settings: AiSettings): void {
  for (const [tabId, session] of Object.entries(_sessionByTab)) {
    for (const item of _chatByTab[tabId] ?? []) {
      if (
        item.kind === "command"
        && !item.result
        && !item.rejected
        && !isAutoApprovalAllowed(settings, item.cmd.kind)
      ) {
        commandApprovals.revokeEligibility(
          { tabId, instanceId: session.instance_id },
          item.cmd.id,
        );
      }
    }
  }
}

/**
 * provider 为空 → 拉 active provider 的快照，**更新**全局 `_settings`（ChatPanel 起 session 读它）；
 * provider 非空 → 仅返回该 provider 的快照，**不动**全局缓存（避免设置页切下拉污染聊天）。
 */
export async function loadSettings(provider?: LlmProvider): Promise<AiSettings> {
  const snapshot = await invoke<AiSettings>("ai_settings_get", { provider: provider || null });
  if (!provider) {
    // Commands can be hidden behind AuditPanel and have no mounted dialog to
    // observe a settings transition. Revocation therefore lives with the
    // global settings snapshot; enabling never grants an existing false entry.
    return installGlobalSettings(snapshot);
  }
  return snapshot;
}
export async function saveSettings(s: AiSettingsPatch) {
  // Disable takes effect before the first await. A command arriving while the
  // DB write is pending must see the safe policy; failure deliberately leaves
  // the suspension in place until an explicit enable succeeds.
  suspendDisabledAutoApprovals(s);
  // Backend takes a single `patch` object (AiSettingsPatch) — every field is
  // "update if present". Wrap the partial settings accordingly.
  await invoke("ai_settings_set", { patch: s });
  const snapshot = await invoke<AiSettings>("ai_settings_get", { provider: null });
  releaseEnabledAutoApprovals(s);
  installGlobalSettings(snapshot);
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

async function attachListeners(info: AiSessionInfo, generation: number) {
  // tab_id 同时是状态字典 key、事件 topic 后缀、internal_command 路由 key —— 单一坐标。
  // info.target_id 不在闭包里捕获 —— 重连后 target_id 变了，internal_command 需要走新的，
  // 闭包里写死会一直发到旧 SSH 会话。运行期通过 _sessionByTab[tab_id].target_id 读最新值。
  const tab = info.tab_id;
  const session = { tabId: tab, instanceId: info.instance_id };
  const u: UnlistenFn[] = [];
  assertTabLive(tab, generation);
  detachListeners(tab);
  // 先登记正在构建的数组。dispose 若发生在任一个 listen await 中，可以立刻
  // 清掉已完成的 listener；迟到的 listener 由下面 catch 再清一次。
  _unlistenersByTab[tab] = u;

  const addListener = async <T>(
    event: string,
    handler: (event: TauriEvent<T>) => void | Promise<void>,
    beforeContextGate?: (event: TauriEvent<T>) => void,
  ) => {
    const unlisten = await listen<T>(event, (payload) => {
      // unlisten 不能撤回已排队的 callback；generation gate 防止关闭后的
      // 迟到事件重新写入 chat/pending/keyboard 状态。context epoch gate
      // separately rejects callbacks queued before a successful context clear.
      if (!isTabLive(tab, generation)) return;
      if (_sessionByTab[tab]?.instance_id !== session.instanceId) return;
      beforeContextGate?.(payload);
      dispatchContextEvent(session, payload.payload, () => {
        if (
          isTabLive(tab, generation)
          && acceptsContextEvent(session, payload.payload)
        ) {
          void handler(payload);
        }
      });
    });
    if (!isTabLive(tab, generation)) {
      unlisten();
      throw new SessionClosedError(tab);
    }
    u.push(unlisten);
  };

  try {
  // 流式：start 创建空气泡，delta append，end 关 streaming 标记
  await addListener<{ id: string }>(`ai:assistant_message_start:${tab}`, (e) => {
    pushChat(tab, { kind: "assistant", id: e.payload.id, text: "", at: Date.now(), streaming: true });
  });

  await addListener<{ id: string; text: string }>(`ai:assistant_delta:${tab}`, (e) => {
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
  });

  await addListener<{
    id: string; text: string; cancelled?: boolean;
    tokens_in?: number | null; tokens_out?: number | null;
  }>(`ai:assistant_message_end:${tab}`, (e) => {
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
        schedulePersist(tab);
        return;
      }
    }
  }, (e) => {
    // Billing belongs to the actor lifetime, not the visible context. Account
    // it before the context gate so a delayed pre-clear terminal event cannot
    // rebuild its bubble but still contributes its already-spent tokens once.
    // Pure tool_use turns (empty text) are billed too; cancelled streams carry
    // no token fields and therefore add nothing.
    const tin = e.payload.tokens_in ?? 0;
    const tout = e.payload.tokens_out ?? 0;
    if (tin > 0 || tout > 0) {
      const cur = _tokensByTab[tab] ?? { tokens_in: 0, tokens_out: 0 };
      _tokensByTab[tab] = { tokens_in: cur.tokens_in + tin, tokens_out: cur.tokens_out + tout };
    }
  });

  await addListener<CommandProposed>(`ai:command_proposed:${tab}`, (e) => {
    const proposed = stripContextEpoch(e.payload);
    // Authorization belongs to command arrival, not component mount. The chat
    // list is unmounted while AuditPanel is visible; reading live settings when
    // it later remounts would let a subsequent enable retro-authorize this cmd.
    commandApprovals.snapshotEligibility(
      { tabId: tab, instanceId: info.instance_id },
      proposed.id,
      isAutoApprovalAllowed(_settings, proposed.kind),
    );
    _pendingByTab[tab] = proposed;
    pushChat(tab, { kind: "command", cmd: proposed, at: Date.now() });
  });

  // internal_command：当前只用于 file_ops 工具的远端能力探测（一行只读 echo "py3=... perl=... diff=..."）。
  // 不弹审批、不入 chat 时间线，直接粘到 PTY 跑——用户在终端历史里看到探测命令滚过，
  // 透明但不打断流程。后续若加其他 read-only 内部命令也走这条路径。
  await addListener<{
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
    if (!currentInfo) {
      // The panel was closed after this callback was queued. There is no safe
      // actor identity left to report to, and tab-only routing could hit the
      // replacement session, so let teardown cancel the old actor.
      return;
    }
    const session = { tabId: tab, instanceId: currentInfo.instance_id };
    if (!kind) {
      // fail-closed：必须给后端回一个 result，否则 wait_command_outcome 永远阻塞，
      // session actor 卡在 file_ops handler 里 await 不出来，整个 AI 会话挂死。
      const msg = `internal_command without target binding for tab ${tab}`;
      console.error("[ai]", msg);
      try {
        await invoke("ai_command_result", {
          tabId: tab,
          instanceId: currentInfo.instance_id,
          toolCallId: e.payload.id,
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
      tool_call_id: e.payload.id,
      cmd: e.payload.cmd,
      full_cmd: e.payload.full_cmd,
      sentinel: e.payload.sentinel,
      explain: "",
      side_effect: "",
      timeout_s: 60,
    };
    try {
      await executeCommand(session, proposed, kind, currentInfo.target_id);
      // Internal probes have no command card and therefore no matching
      // command_completed event to own registry cleanup. ai_command_result is a
      // processing ack, so success is the exact point at which replay is no
      // longer possible and the buffer can be released.
      clearCommandExecution(session, e.payload.id);
    } catch (err) {
      console.error("[ai] internal_command exec failed:", err);
      const execution = _commandExecutions.get(
        sessionCommandKey(session, e.payload.id),
      );
      if (!(err instanceof CommandSetupError)) {
        // Transport already committed. A write failure is reported through
        // finish(), and a result-delivery failure retains that exact payload in
        // the Execution. Never replace it with a second synthetic failure: the
        // actor could consume the wrong result and a later teardown retry would
        // enqueue a stale duplicate. Retry only the recorded report.
        if (execution?.status === "delivery_failed") {
          try {
            await execution.deliver();
            clearCommandExecution(session, e.payload.id);
          } catch (reportErr) {
            console.error("[ai] failed to redeliver internal_command result:", reportErr);
          }
        } else if (execution?.status === "delivered") {
          clearCommandExecution(session, e.payload.id);
        }
        return;
      }
      // listen() can fail before transport starts and before an Execution has a
      // report to retry. Only that setup-failure path synthesizes a result;
      // otherwise wait_command_outcome would remain blocked forever.
      try {
        await invoke("ai_command_result", {
          tabId: tab,
          instanceId: currentInfo.instance_id,
          toolCallId: e.payload.id,
          exitCode: -1,
          output: errMsg(err),
          timedOut: false,
          earlyTerminated: false,
        });
      } catch (reportErr) {
        console.error("[ai] failed to report internal_command exec failure:", reportErr);
      }
    }
  });

  await addListener<{ id: string; lock_keyboard: boolean }>(`ai:command_executing:${tab}`, (e) => {
    _keyboardLockedByTab[tab] = !!e.payload.lock_keyboard;
  });

  await addListener<CommandResult & { lock_keyboard: boolean }>(`ai:command_completed:${tab}`, (e) => {
    _keyboardLockedByTab[tab] = !!e.payload.lock_keyboard;
    _pendingByTab[tab] = null;
    // 给最近一条对应 id 的 command 项填上 result
    const arr = _chatByTab[tab] ?? [];
    for (let i = arr.length - 1; i >= 0; i--) {
      const item = arr[i];
      if (item.kind === "command" && item.cmd.id === e.payload.id) {
        clearCommandExecution(
          { tabId: tab, instanceId: info.instance_id },
          e.payload.id,
        );
        commandApprovals.clear(
          { tabId: tab, instanceId: info.instance_id },
          e.payload.id,
        );
        const replaced: ChatItem = { ...item, result: stripContextEpoch(e.payload) };
        _chatByTab[tab] = [...arr.slice(0, i), replaced, ...arr.slice(i + 1)];
        schedulePersist(tab);
        break;
      }
    }
  });

  // 拒绝路径单独事件 —— complete 跟 reject 是两种语义，复用 command_completed
  // 加 rejected:true 字段会让 listener 分支模糊。后端 RejectCommand 分支 emit
  // 这个，前端清 pending + 标记 ChatItem.rejected。
  await addListener<{ id: string; reason: string }>(`ai:command_rejected:${tab}`, (e) => {
    _pendingByTab[tab] = null;
    const arr = _chatByTab[tab] ?? [];
    for (let i = arr.length - 1; i >= 0; i--) {
      const item = arr[i];
      if (item.kind === "command" && item.cmd.id === e.payload.id) {
        clearCommandExecution(
          { tabId: tab, instanceId: info.instance_id },
          e.payload.id,
        );
        commandApprovals.clear(
          { tabId: tab, instanceId: info.instance_id },
          e.payload.id,
        );
        const replaced: ChatItem = { ...item, rejected: { reason: e.payload.reason } };
        _chatByTab[tab] = [...arr.slice(0, i), replaced, ...arr.slice(i + 1)];
        schedulePersist(tab);
        break;
      }
    }
  });

  // load_skill 成功 —— 后端 emit 这个，聊天面板出一条低调的 note 气泡，告诉用户
  // AI 加载了哪个用户技能（read-only，无审批卡片）。审计日志另有 SkillLoaded 条目。
  await addListener<{ id: string; name: string }>(`ai:skill_loaded:${tab}`, (e) => {
    pushChat(tab, { kind: "note", text: t("ai.note.skill_loaded", { name: e.payload.name }), at: Date.now() });
  });

  // rssh 黑名单直接拦掉命令（命令从未变成审批卡片）—— 后端 emit 这个，否则安全层
  // 静默触发，用户只看到 AI 莫名改了方案。命令可能很长，截断显示。
  await addListener<{ cmd: string; reason: string }>(`ai:command_blocked:${tab}`, (e) => {
    pushChat(tab, { kind: "note", text: t("ai.note.command_blocked", { cmd: truncateCommand(e.payload.cmd), reason: e.payload.reason }), at: Date.now() });
  });

  await addListener<{ message: string }>(`ai:error:${tab}`, (e) => {
    pushChat(tab, { kind: "error", text: e.payload.message, at: Date.now() });
  });

  await addListener<{}>(`ai:session_ended:${tab}`, () => {
    pushChat(tab, { kind: "note", text: t("ai.session.ended_note"), at: Date.now() });
  });

    assertTabLive(tab, generation);
  } catch (error) {
    const listeners = u.splice(0);
    listeners.forEach((fn) => fn());
    if (_unlistenersByTab[tab] === u) delete _unlistenersByTab[tab];
    throw error;
  }
}

function detachListeners(tab_id: string) {
  const arr = _unlistenersByTab[tab_id];
  if (arr) {
    delete _unlistenersByTab[tab_id];
    const listeners = arr.splice(0);
    listeners.forEach(fn => fn());
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

// ─── 命令黑名单管理 ──────────────────────────────────────────────────
// 整类编辑：保存即整类替换。改动只对新会话生效（后端建会话时 snapshot）。
// replaceCommandBlacklist 在后端校验命令名，坏名会抛 blacklist_invalid_name。

export async function listCommandBlacklist(): Promise<CategoryGroup[]> {
  return invoke<CategoryGroup[]>("ai_list_command_blacklist");
}

export async function replaceCommandBlacklist(category: string, names: string[]): Promise<void> {
  return invoke("ai_replace_command_blacklist", { category, names });
}
