import { invoke } from "@tauri-apps/api/core";
import * as ai from "../ai/store.svelte.ts";
import type { ViewportSnapshot } from "../terminal/viewport-snapshot.ts";

/* ═══════════════════════════════════════════════════════
   Platform
   ═══════════════════════════════════════════════════════ */
// `navigator` is absent in the node test env; guard so importing this module
// from a unit test doesn't throw. Browsers always have it → behavior unchanged.
export const isMobile =
  typeof navigator !== "undefined" && /Android|iPhone|iPad/i.test(navigator.userAgent);

/* ═══════════════════════════════════════════════════════
   Types
   ═══════════════════════════════════════════════════════ */
export type TabType = "home" | "ssh" | "local" | "serial" | "telnet" | "docker_exec" | "kubectl_exec" | "forward" | "edit";
/** Tab types that render a TerminalPane (byte-stream terminals). */
export type TerminalTabType = Exclude<TabType, "home" | "forward" | "edit">;
export function isTerminalTabType(type: TabType): type is TerminalTabType {
  return type === "ssh" || type === "local" || type === "serial" || type === "telnet"
    || type === "docker_exec" || type === "kubectl_exec";
}
export type AiCapableTabType = "ssh" | "local" | "serial" | "telnet";
export function isAiCapableTabType(type: TabType): type is AiCapableTabType {
  return type === "ssh" || type === "local" || type === "serial" || type === "telnet";
}
export interface Tab {
  id: string;
  type: TabType;
  label: string;
  meta?: Record<string, string>;
}

/** Settings sub-pages (rendered inside the settings tab) */
export type SettingsPage =
  | "menu"
  | "connections"
  | "connection-edit"
  | "credentials"
  | "credential-edit"
  | "dynamic-discovery"
  | "snippets"
  | "highlights"
  | "sync"
  | "import-export"
  | "import-ssh-config"
  | "recording-settings"
  | "playback"
  | "shell-settings"
  | "command-blocks"
  | "groups"
  | "group-edit"
  | "cli"
  | "shortcuts"
  | "appearance"
  | "ai"
  | "about";

export interface Group {
  id: string; name: string; color: string; sort_order: number;
}
export type ConnectionKind = "ssh" | "forward" | "serial" | "telnet";
export type ConnectionEditorIntent =
  | { mode: "create"; kind: ConnectionKind; sourceId: null }
  | { mode: "edit" | "copy"; kind: ConnectionKind; sourceId: string };
export interface SshAlgorithms {
  kex: string[];
  key: string[];
  cipher: string[];
  mac: string[];
  compression: string[];
}
export interface SshAlgorithmCatalog {
  defaults: SshAlgorithms;
  supported: SshAlgorithms;
}
export interface Profile {
  id: string; name: string; host: string; port: number;
  credential_id: string; bastion_profile_id: string | null; init_command: string | null;
  group_id: string | null;
  algorithms: SshAlgorithms;
}
export interface Credential {
  id: string; name: string; username: string;
  type: string; secret: string | null; save_to_remote: boolean;
  passphrase: string | null;
}
export interface Forward {
  id: string; name: string; type: string;
  local_port: number; remote_host: string; remote_port: number; profile_id: string;
  group_id: string | null;
}
export interface SerialProfile {
  id: string; name: string; port: string;
  baud_rate: number; data_bits: number; parity: string;
  stop_bits: number; flow_control: string;
  // Tabby-style extras (xany is a wire flag; the rest are terminal-layer)
  xany: boolean;
  input_newline: string; output_newline: string;
  local_echo: boolean; backspace: string; slow_send: boolean;
  input_mode: string; output_mode: string; login_script: string;
  group_id: string | null;
}
export interface TelnetProfile {
  id: string; name: string; host: string; port: number;
  // Line-discipline knobs shared with serial (no UART-isms: no baud/hex/slow_send)
  input_newline: string; output_newline: string;
  /** Explicit Telnet echo policy. Missing on legacy sync payloads. */
  echo_mode?: "auto" | "on" | "off";
  local_echo: boolean; backspace: string; login_script: string;
  save_script_to_remote?: boolean;
  group_id: string | null;
}
export type DynamicPlatform = "docker" | "k8s";
export type ConnectorSpec =
  | { type: "docker_exec"; context: string; container_id: string; container_name: string; shell: string }
  | { type: "kubectl_exec"; context: string; namespace: string; pod: string; container: string | null; shell: string };
export type DynamicDiscoverySource =
  | { id: string; name: string; enabled: boolean; platform: "docker"; context: string; shell: string }
  | { id: string; name: string; enabled: boolean; platform: "k8s"; context: string; namespace: string | null; shell: string };
export interface DynamicDiscoveryContext {
  platform: DynamicPlatform;
  name: string;
  current: boolean;
}
export interface DynamicDiscoveryToolStatus {
  platform: DynamicPlatform;
  available: boolean;
  version: string | null;
  error: string | null;
}
export interface DynamicDiscoveredTarget {
  id: string;
  source_id: string;
  source_name: string;
  platform: DynamicPlatform;
  name: string;
  sub: string;
  connector_spec: ConnectorSpec;
}
export interface DynamicDiscoveryError {
  source_id: string;
  source_name: string;
  platform: DynamicPlatform;
  message: string;
}
export interface DynamicDiscoverySnapshot {
  targets: DynamicDiscoveredTarget[];
  errors: DynamicDiscoveryError[];
}
export interface Snippet { name: string; command: string; }
export interface HighlightRule { keyword: string; name: string; color: string; enabled: boolean; is_regex: boolean; is_case_sensitive: boolean; }
export interface RemoteEntry {
    name: string;
    is_dir: boolean;
    is_symlink: boolean;
    size: number;
    /** unix epoch seconds; 0 means server didn't provide mtime */
    mtime: number;
}

/* ═══════════════════════════════════════════════════════
   Reactive state
   ═══════════════════════════════════════════════════════ */
let _tabs = $state<Tab[]>([{ id: "home", type: "home", label: "Home" }]);
let _activeTabId = $state("home");
let _settingsActive = $state(false);
let _settingsPage = $state<SettingsPage>("menu");
let _editingId = $state<string | null>(null);
let _connectionEditorIntent = $state<ConnectionEditorIntent>({ mode: "create", kind: "ssh", sourceId: null });

/* SFTP per-tab：每个 ssh tab 独立 open/close（local PTY 没远端 fs，openSftp gate 掉）。
   SFTP 共用对应 tab 的 SSH 连接；切 tab 不影响其他 tab 已打开的 SFTP；
   新开 tab 不自动开 SFTP——每个 tab 手动开。
   (老的全局 _sftpOpen 已废，那是 fullscreen overlay 时代的产物。) */
let _sftpOpenByTab = $state<Record<string, boolean>>({});
/* Transfers popover: an overlay, no longer a sibling route of Settings.
   State is independent — switching tabs / opening Settings does not close it;
   the user must dismiss explicitly (X / click outside / Esc / re-click entry).
   The variable name keeps `_downloadsActive` because the sidebar entry id is
   still "downloads" — renaming buys little. */
let _downloadsActive = $state(false);
/**
 * Read a JSON-encoded array of strings from localStorage. Returns [] on any
 * failure (key missing, value not JSON, value not an array). Module-load
 * code path — a raw `JSON.parse` would throw and white-screen the app on
 * any corruption (extension wrote garbage, user fiddled in DevTools).
 */
function loadStringArray(key: string): string[] {
    try {
        return parseStringArray(localStorage.getItem(key));
    } catch {
        return [];
    }
}

function parseStringArray(raw: string | null): string[] {
    if (raw === null) return [];
    try {
        const parsed = JSON.parse(raw);
        return Array.isArray(parsed) ? parsed.filter((v) => typeof v === "string") : [];
    } catch {
        return [];
    }
}

/**
 * Best-effort localStorage write. Symmetric with `loadStringArray` on the
 * read side. Swallows QuotaExceededError (Safari private mode caps), SecurityError
 * (enterprise GPO disables storage), and any other DOMException. The state in
 * memory is already updated by the caller — failing to persist degrades to
 * "preference resets on next reload", not "UI throws and Promise chain breaks".
 *
 * Use for preference-style writes where a lost setting is annoying but not
 * destructive. NOT for anything where losing the write means losing data.
 */
function safeSetItem(key: string, value: string) {
    try {
        localStorage.setItem(key, value);
    } catch (e) {
        // One warn per failure is enough; don't spam if quota stays exceeded.
        console.warn(`[app] localStorage setItem(${key}) failed:`, e);
    }
}

const RECENT_HOME_ITEMS_KEY = "home.recent_items.v1";
const MAX_RECENT_HOME_ITEMS = 256;

function normalizeRecentHomeItemIds(ids: string[]): string[] {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const id of ids) {
    if (!id || seen.has(id)) continue;
    seen.add(id);
    normalized.push(id);
    if (normalized.length === MAX_RECENT_HOME_ITEMS) break;
  }
  return normalized;
}

let _pinnedProfileIds = $state<string[]>(loadStringArray("pinned_profiles"));
let _recentHomeItemIds = $state<string[]>(
  normalizeRecentHomeItemIds(loadStringArray(RECENT_HOME_ITEMS_KEY)),
);
if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    if (event.key !== RECENT_HOME_ITEMS_KEY) return;
    _recentHomeItemIds = normalizeRecentHomeItemIds(parseStringArray(event.newValue));
  });
}

/* Terminal title (from remote shell OSC sequence), separate from tab label */
let _terminalTitles = $state<Record<string, string>>({});

/* ─── Getters ─── */
export function tabs() { return _tabs; }
export function activeTabId() { return _activeTabId; }
export function activeTab() { return _tabs.find(t => t.id === _activeTabId); }
export function settingsActive() { return _settingsActive; }
export function settingsPage() { return _settingsPage; }
export function editingId() { return _editingId; }
export function connectionEditorIntent(): ConnectionEditorIntent { return _connectionEditorIntent; }
export function connectionTypeLocked() { return _connectionEditorIntent.mode !== "create"; }
export function connectionUpdateId(): string | null {
  return _connectionEditorIntent.mode === "edit" ? _connectionEditorIntent.sourceId : null;
}
/** 当前活跃 tab 的 SFTP 是否打开（toolbar / Esc / × 按钮等用这个）。 */
export function sftpOpen() { return !!_sftpOpenByTab[_activeTabId]; }
/** 任意 tab 是否查询；用 tab id 显式问。 */
export function sftpOpenForTab(tabId: string) { return !!_sftpOpenByTab[tabId]; }
/** 模板 {#each} 遍历所有"开了 SFTP"的 tab 用——保持 SftpBrowser 实例存活以便切回时 cwd 不丢。 */
export function tabsWithSftp(): Tab[] { return _tabs.filter(t => _sftpOpenByTab[t.id]); }
export function downloadsActive() { return _downloadsActive; }
export function pinnedProfileIds() { return _pinnedProfileIds; }
export function recentHomeItemIds() { return _recentHomeItemIds; }
export function terminalTitle(tabId: string) { return _terminalTitles[tabId]; }

/* ─── Tab Operations ─── */
export function setActiveTab(id: string) {
  _activeTabId = id;
  _settingsActive = false;
  // MRU (when enabled): bring the just-focused session tab to the front of the
  // session region (index 1, right after the fixed home tab). Reuses the
  // drag-reorder primitive. home (index 0) and an already-front tab (index 1)
  // are no-ops; _tabMru off leaves the order untouched.
  const idx = _tabs.findIndex((t) => t.id === id);
  if (_tabMru && idx > 1) moveTab(idx, 1);
  // SFTP per-tab：切 tab 不动其他 tab 的 SFTP 状态（mirror AI panel 的"跨导航持久"模型）
  // Transfers popover state persists across tab switches; closed only by user.
}

export function addTab(tab: Tab) {
  // MRU on: new tab is the most-recently-focused → front of the session region
  // (index 1, right after the fixed home tab), no "freshly created but not at
  // front" special case. MRU off: append at the end (pre-MRU behavior).
  _tabs.splice(_tabMru ? 1 : _tabs.length, 0, tab);
  _activeTabId = tab.id;
  _settingsActive = false;
  recordRecentHomeItem(tab);
}

function recordRecentHomeItem(tab: Tab): void {
  const itemId = homeItemIdForTab(tab);
  if (!itemId) return;
  _recentHomeItemIds = normalizeRecentHomeItemIds([
    itemId,
    ...loadStringArray(RECENT_HOME_ITEMS_KEY),
    ..._recentHomeItemIds.filter((id) => id !== itemId),
  ]);
  safeSetItem(RECENT_HOME_ITEMS_KEY, JSON.stringify(_recentHomeItemIds));
}

function homeItemIdForTab(tab: Tab): string | null {
  const meta = tab.meta;
  switch (tab.type) {
    case "ssh":
      return meta?.profileId ? `ssh:${meta.profileId}` : null;
    case "forward":
      return meta?.forwardId ? `forward:${meta.forwardId}` : null;
    case "serial":
      return meta?.serialProfileId ? `serial:${meta.serialProfileId}` : null;
    case "telnet":
      return meta?.profileId ? `telnet:${meta.profileId}` : null;
    case "docker_exec":
    case "kubectl_exec":
      return meta?.sourceId && meta.dynamicTargetId
        ? `dynamic:${meta.sourceId}:${meta.dynamicTargetId}`
        : null;
    case "home":
    case "local":
    case "edit":
      return null;
  }
}

export function moveTab(fromIdx: number, toIdx: number) {
  if (fromIdx === toIdx || fromIdx < 0 || toIdx < 0) return;
  if (fromIdx >= _tabs.length || toIdx >= _tabs.length) return;
  const next = [..._tabs];
  const [tab] = next.splice(fromIdx, 1);
  next.splice(toIdx, 0, tab);
  _tabs = next;
}

export function closeTab(id: string) {
  const idx = _tabs.findIndex(t => t.id === id);
  if (idx < 0 || _tabs[idx].type === "home") return;
  const wasActive = _activeTabId === id;
  _tabs.splice(idx, 1);
  delete _terminalTitles[id];
  // tab 自身没了，对应的 SFTP 实例也得 unmount —— 删 map entry 让 {#each} 收掉
  if (_sftpOpenByTab[id]) {
    const next = { ..._sftpOpenByTab };
    delete next[id];
    _sftpOpenByTab = next;
  }
  // AI actor 跟 tab 同寿命。fire-and-forget —— UI 拆完不必等 actor stop ack。
  // sessionForTab undefined（这个 tab 从没起过 AI）也走 stopSession 没事，
  // 后端会返回 ai_session_not_found，前端 catch 吞掉。
  if (ai.sessionForTab(id)) {
    ai.stopSession(id).catch((e) => console.warn("[ai] stop on tab close:", e));
  }
  if (wasActive) {
    _activeTabId = _tabs[Math.min(idx, _tabs.length - 1)]?.id ?? "home";
  }
}

export function updateTabLabel(id: string, label: string) {
  const tab = _tabs.find(t => t.id === id);
  if (tab) tab.label = label;
}

export function setTerminalTitle(tabId: string, title: string) {
  _terminalTitles[tabId] = title;
}

/* ─── Settings Navigation ─── */
export function openSettings() {
  _settingsActive = true;
  // SFTP 不强制关 —— settings 路径下走可见性 derived 隐藏，state 保留
  // Transfers popover state is independent — leave it untouched here.
}

/** Open the transfers popover. State is independent from settings/tab — the
 *  popover is itself an overlay. */
export function openDownloads() { _downloadsActive = true; }
export function closeDownloads() { _downloadsActive = false; }
export function toggleDownloads() { _downloadsActive = !_downloadsActive; }

export function settingsNavigate(page: SettingsPage, editId?: string) {
  _settingsPage = page;
  _editingId = editId ?? null;
}

export function openConnectionCreate(kind: ConnectionKind = "ssh") {
  _connectionEditorIntent = { mode: "create", kind, sourceId: null };
  navigate("connection-edit");
}

export function openConnectionEdit(kind: ConnectionKind, sourceId: string) {
  _connectionEditorIntent = { mode: "edit", kind, sourceId };
  navigate("connection-edit");
}

export function openConnectionCopy(kind: ConnectionKind, sourceId: string) {
  _connectionEditorIntent = { mode: "copy", kind, sourceId };
  navigate("connection-edit");
}

export function settingsBack() {
  if (_settingsPage === "connection-edit") _settingsPage = "connections";
  else if (_settingsPage === "credential-edit") _settingsPage = "credentials";
  else if (_settingsPage === "import-ssh-config") _settingsPage = "import-export";
  else _settingsPage = "menu";
}

/* ─── Sidebar position (per-device) ─── */
export type SidebarPosition = "left" | "right" | "top" | "bottom";
const _SB_KEY_DESKTOP = "sidebar.position.desktop";
const _SB_KEY_MOBILE = "sidebar.position.mobile";
function _loadSidebarPos(key: string, fallback: SidebarPosition): SidebarPosition {
  const v = localStorage.getItem(key);
  return v === "left" || v === "right" || v === "top" || v === "bottom" ? v : fallback;
}
let _sidebarPosDesktop = $state<SidebarPosition>(_loadSidebarPos(_SB_KEY_DESKTOP, "top"));
let _sidebarPosMobile = $state<SidebarPosition>(_loadSidebarPos(_SB_KEY_MOBILE, "top"));
export function sidebarPosition(): SidebarPosition {
  return isMobile ? _sidebarPosMobile : _sidebarPosDesktop;
}
export function setSidebarPosition(pos: SidebarPosition) {
  if (isMobile) {
    _sidebarPosMobile = pos;
    safeSetItem(_SB_KEY_MOBILE, pos);
  } else {
    _sidebarPosDesktop = pos;
    safeSetItem(_SB_KEY_DESKTOP, pos);
  }
}

/* ─── Mobile key modifiers (sticky Ctrl/Alt) ─── */
let _ctrlActive = $state(false);
let _altActive = $state(false);
export function ctrlActive() { return _ctrlActive; }
export function altActive() { return _altActive; }
export function setCtrl(v: boolean) { _ctrlActive = v; }
export function setAlt(v: boolean) { _altActive = v; }
export function clearModifiers() { _ctrlActive = false; _altActive = false; }

/* ─── Send to active terminal ─── */
let _terminalWriter: ((text: string) => void) | null = null;
export function registerTerminalWriter(fn: (text: string) => void) { _terminalWriter = fn; }
export function unregisterTerminalWriter() { _terminalWriter = null; }
export function sendToTerminal(text: string) { _terminalWriter?.(text); }

/** Arrow keys need DECCKM-aware encoding (CSI vs SS3). The terminal owner
 *  holds that state, so it registers an encoder-sender here. */
export type ArrowDir = "A" | "B" | "C" | "D";
let _terminalArrowSender: ((dir: ArrowDir, mod: number) => void) | null = null;
export function registerTerminalArrowSender(fn: (dir: ArrowDir, mod: number) => void) { _terminalArrowSender = fn; }
export function unregisterTerminalArrowSender() { _terminalArrowSender = null; }
export function sendArrow(dir: ArrowDir, mod: number) { _terminalArrowSender?.(dir, mod); }

/* ─── Per-tab terminal copy/paste controls ─── */
interface TerminalControls {
  getSelection(): string;
  paste(text: string): void;
  /** Inject user text as input (snippet / broadcast). The pane applies its own
   *  transport rules — serial EOL transform + slow-send — so callers here stay
   *  transport-agnostic. NOT for control bytes (arrows/Esc/Tab): those are raw,
   *  see sendToTerminal. */
  sendText(text: string): void;
  focus(): void;
  /** Read-only minimap projection of the visible viewport, for the broadcast
   *  editor's session previews. Optional: panes that predate it just show blank. */
  readViewport?(): ViewportSnapshot | null;
  /** Read-only text of the visible viewport, one string per row, for the
   *  hover preview. Optional, same as readViewport. */
  readViewportText?(): string[] | null;
}
const _terminalControls = new Map<string, TerminalControls>();
export function registerTerminalControls(tabId: string, controls: TerminalControls) {
  _terminalControls.set(tabId, controls);
}
export function unregisterTerminalControls(tabId: string) {
  _terminalControls.delete(tabId);
}
export function terminalGetSelection(tabId: string): string {
  return _terminalControls.get(tabId)?.getSelection() ?? "";
}
export function terminalPaste(tabId: string, text: string) {
  _terminalControls.get(tabId)?.paste(text);
}
/** Snippet picker (and any "run this text" action): send user text to the
 *  active terminal honoring its EOL. Distinct from sendToTerminal, which is for
 *  raw control sequences and must never transform line endings. */
export function sendTextToActiveTerminal(text: string) {
  _terminalControls.get(_activeTabId)?.sendText(text);
}
/** Return keyboard focus to a tab's terminal — used by modals (snippet picker,
 *  search) that steal focus and must hand it back on close, else focus falls
 *  to document.body and the user can't type until they click the terminal. */
export function terminalFocus(tabId: string) {
  _terminalControls.get(tabId)?.focus();
}

/** Read-only viewport minimap for a tab, or null if the pane can't provide one.
 *  Reuses the tab's existing terminal buffer — no new connection, no replay. */
export function readTerminalViewport(tabId: string): ViewportSnapshot | null {
  return _terminalControls.get(tabId)?.readViewport?.() ?? null;
}

/** Read-only viewport text (one string per row) for a tab's hover preview, or
 *  null if unavailable. Reuses the tab's existing terminal buffer. */
export function readTerminalViewportText(tabId: string): string[] | null {
  return _terminalControls.get(tabId)?.readViewportText?.() ?? null;
}

/** Read system clipboard. On desktop, goes through Rust to bypass
 *  WebKit's permission prompt for externally-sourced content. */
export async function readClipboard(): Promise<string> {
  if (isMobile) {
    return navigator.clipboard.readText().catch(() => "");
  }
  return invoke<string>("clipboard_read").catch(() => "");
}

/** Write text to the system clipboard. On desktop goes through Rust (arboard)
 *  — WKWebView's `navigator.clipboard.writeText` silently fails from a
 *  right-click / unfocused context. Mobile uses the web API. */
export async function writeClipboard(text: string): Promise<void> {
  if (isMobile) {
    await navigator.clipboard.writeText(text).catch(() => {});
    return;
  }
  await invoke("clipboard_write", { text }).catch(() => {});
}

/* ─── Session registry (for broadcast) ─── */
interface SessionEntry {
  tabId: string;
  sessionId: string;
  type: TerminalTabType;
}
export interface SessionInfo extends SessionEntry {
  label: string;
}
let _sessions = $state<SessionEntry[]>([]);

/**
 * Pending `waitForSession` calls keyed by tabId. A poll-loop in AppShell
 * used to busy-check `sessionIdForTab` every 300 ms for up to 30 s; we
 * own the state, so we can just notify when registerSession fires.
 */
const _sessionWaiters: Map<string, Array<(sid: string | null) => void>> = new Map();

export function registerSession(info: SessionEntry) {
  _sessions = [..._sessions.filter(s => s.tabId !== info.tabId), info];
  const waiters = _sessionWaiters.get(info.tabId);
  if (waiters && waiters.length) {
    _sessionWaiters.delete(info.tabId);
    for (const fn of waiters) fn(info.sessionId);
  }
}
export function unregisterSession(tabId: string) {
  _sessions = _sessions.filter(s => s.tabId !== tabId);
}

/**
 * Resolve when a session is registered for `tabId`. `null` after `timeoutMs`
 * if the session never appears (e.g. PTY spawn failure). Replaces a 30s
 * setInterval poll — the store is the source of truth, no reason to spin.
 */
export function waitForSession(tabId: string, timeoutMs = 30000): Promise<string | null> {
  const existing = sessionIdForTab(tabId);
  if (existing) return Promise.resolve(existing);
  return new Promise((resolve) => {
    let done = false;
    const fire = (val: string | null) => {
      if (done) return;
      done = true;
      resolve(val);
    };
    const arr = _sessionWaiters.get(tabId) ?? [];
    arr.push(fire);
    _sessionWaiters.set(tabId, arr);
    setTimeout(() => fire(null), timeoutMs);
  });
}
export function connectedSessions(): SessionInfo[] {
  return _sessions.map(s => ({
    ...s,
    label: _tabs.find(t => t.id === s.tabId)?.label ?? s.tabId,
  }));
}
export function sessionIdForTab(tabId: string): string | undefined {
  return _sessions.find(s => s.tabId === tabId)?.sessionId;
}

export function broadcastToSessions(tabIds: string[], text: string) {
  // Route through each pane's sendText (registered while mounted — all tabs
  // mount, only the active one is visible). The pane owns its transport + the
  // serial EOL/slow-send transform, so no per-type invoke switch belongs here.
  for (const tabId of tabIds) {
    _terminalControls.get(tabId)?.sendText(text);
  }
}

/* ─── Snippet picker ─── */
let _snippetPickerOpen = $state(false);
export function snippetPickerOpen() { return _snippetPickerOpen; }
export function openSnippetPicker() { _snippetPickerOpen = true; }
export function closeSnippetPicker() { _snippetPickerOpen = false; }

/** Open a terminal tab from a saved serial profile (Home cards + the manager
 *  use this). meta carries the config in snake_case — TerminalPane feeds it to
 *  serial_open verbatim, no remapping. id is ignored (ad-hoc connects pass ""). */
export function connectSerialProfile(sp: SerialProfile) {
  addTab({
    id: `serial:${crypto.randomUUID()}`,
    type: "serial",
    label: sp.name,
    meta: {
      serialProfileId: sp.id,
      port: sp.port,
      baud_rate: String(sp.baud_rate),
      data_bits: String(sp.data_bits),
      parity: sp.parity,
      stop_bits: String(sp.stop_bits),
      flow_control: sp.flow_control,
      xany: String(sp.xany),
      input_newline: sp.input_newline,
      output_newline: sp.output_newline,
      local_echo: String(sp.local_echo),
      backspace: sp.backspace,
      slow_send: String(sp.slow_send),
      input_mode: sp.input_mode,
      output_mode: sp.output_mode,
      login_script: sp.login_script,
    },
  });
}

/** Open a terminal tab from a saved telnet profile. Non-secret connection
 *  settings live in tab meta; the login script is fetched by profileId only
 *  after TerminalPane mounts, so window-clone persistence never sees it. */
export function connectTelnetProfile(tp: TelnetProfile) {
  addTab({
    id: `telnet:${crypto.randomUUID()}`,
    type: "telnet",
    label: tp.name,
    meta: {
      profileId: tp.id,
      host: tp.host,
      port: String(tp.port),
      input_newline: tp.input_newline,
      output_newline: tp.output_newline,
      local_echo: String(tp.local_echo),
      echo_mode: tp.echo_mode ?? (tp.local_echo ? "on" : "off"),
      backspace: tp.backspace,
    },
  });
}

export function connectDynamicTarget(target: DynamicDiscoveredTarget) {
  const spec = target.connector_spec;
  const tabType = connectorTabType(spec);
  addTab({
    id: `${tabType}:${crypto.randomUUID()}`,
    type: tabType,
    label: target.name,
    meta: {
      connectorSpec: JSON.stringify(spec),
      dynamicTargetId: target.id,
      sourceId: target.source_id,
      sourceName: target.source_name,
    },
  });
}

function connectorTabType(spec: ConnectorSpec): Extract<TerminalTabType, ConnectorSpec["type"]> {
  switch (spec.type) {
    case "docker_exec":
      return "docker_exec";
    case "kubectl_exec":
      return "kubectl_exec";
  }

  const _exhaustive: never = spec;
  throw new Error(`Unsupported connector spec: ${JSON.stringify(_exhaustive)}`);
}

/* ─── Terminal command block side-bar ─── */
let _commandBlockBar = $state(true);
let _cbbLoaded = false;
export function commandBlockBar() { return _commandBlockBar; }
export async function loadCommandBlockBar(): Promise<boolean> {
  if (!_cbbLoaded) {
    _cbbLoaded = true;
    try {
      const v = await invoke<string | null>("get_setting", { key: "command_block_bar" });
      _commandBlockBar = v !== "false";
    } catch {}
  }
  return _commandBlockBar;
}
export async function setCommandBlockBar(v: boolean) {
  _commandBlockBar = v;
  _cbbLoaded = true;
  await invoke("set_setting", { key: "command_block_bar", value: String(v) });
}

/* ─── Auto-color every command block ─── */
// When on, each new block is colored automatically (same effect as right-click
// "color"); right-click can still uncolor an individual block. Default false:
// only an explicit "true" enables, same encoding as copyOnSelect.
let _autoColorBlocks = $state(false);
let _acbLoaded = false;
export function autoColorBlocks() { return _autoColorBlocks; }
export async function loadAutoColorBlocks(): Promise<boolean> {
  if (!_acbLoaded) {
    _acbLoaded = true;
    try {
      const v = await invoke<string | null>("get_setting", { key: "command_block_auto_color" });
      _autoColorBlocks = v === "true";
    } catch {}
  }
  return _autoColorBlocks;
}
export async function setAutoColorBlocks(v: boolean) {
  _autoColorBlocks = v;
  _acbLoaded = true;
  await invoke("set_setting", { key: "command_block_auto_color", value: String(v) });
}

/* ─── Copy selected terminal text on selection ─── */
// Default false: never silently touch the clipboard unless the user opts in,
// so encoding is inverted vs commandBlockBar (only an explicit "true" enables).
let _copyOnSelect = $state(false);
let _cosLoaded = false;
export function copyOnSelect() { return _copyOnSelect; }
export async function loadCopyOnSelect(): Promise<boolean> {
  if (!_cosLoaded) {
    _cosLoaded = true;
    try {
      const v = await invoke<string | null>("get_setting", { key: "copy_on_select" });
      _copyOnSelect = v === "true";
    } catch {}
  }
  return _copyOnSelect;
}
export async function setCopyOnSelect(v: boolean) {
  _copyOnSelect = v;
  _cosLoaded = true;
  await invoke("set_setting", { key: "copy_on_select", value: String(v) });
}

/* ─── Terminal right-click action (single choice) ─── */
// menu = keep the native system menu; paste = paste; copyPaste = copy the
// selection if any, else paste (PuTTY convention).
export type RightClickAction = "menu" | "paste" | "copyPaste";
let _rightClickAction = $state<RightClickAction>("menu");
let _rcaLoaded = false;
export function rightClickAction() { return _rightClickAction; }
export async function loadRightClickAction(): Promise<RightClickAction> {
  if (!_rcaLoaded) {
    _rcaLoaded = true;
    try {
      const v = await invoke<string | null>("get_setting", { key: "right_click_action" });
      if (v === "paste" || v === "copyPaste" || v === "menu") _rightClickAction = v;
    } catch {}
  }
  return _rightClickAction;
}
export async function setRightClickAction(v: RightClickAction) {
  _rightClickAction = v;
  _rcaLoaded = true;
  await invoke("set_setting", { key: "right_click_action", value: v });
}

/* ─── Confirm before closing a tab ─── */
// Off by default so existing close behavior is unchanged; only an explicit
// "true" turns the confirmation on. Same encoding as copyOnSelect.
let _confirmCloseTab = $state(false);
let _cctLoaded = false;
export function confirmCloseTab() { return _confirmCloseTab; }
export async function loadConfirmCloseTab(): Promise<boolean> {
  if (!_cctLoaded) {
    _cctLoaded = true;
    try {
      const v = await invoke<string | null>("get_setting", { key: "confirm_close_tab" });
      _confirmCloseTab = v === "true";
    } catch {}
  }
  return _confirmCloseTab;
}
export async function setConfirmCloseTab(v: boolean) {
  _confirmCloseTab = v;
  _cctLoaded = true;
  await invoke("set_setting", { key: "confirm_close_tab", value: String(v) });
}

/* ─── MRU tab reorder ─── */
// Off by default: tabs keep their insertion order (see setActiveTab / addTab).
// On makes focusing a session tab move it to the front of the strip. Default-
// false encoding: only an explicit "true" enables — so a user who previously
// chose either way (stored "true"/"false") keeps that choice; only the never-
// touched default flips to off.
let _tabMru = $state(false);
let _tabMruLoaded = false;
export function tabMru() { return _tabMru; }
export async function loadTabMru(): Promise<boolean> {
  if (!_tabMruLoaded) {
    _tabMruLoaded = true;
    try {
      const v = await invoke<string | null>("get_setting", { key: "tab_mru_reorder" });
      _tabMru = v === "true";
    } catch {}
  }
  return _tabMru;
}
export async function setTabMru(v: boolean) {
  _tabMru = v;
  _tabMruLoaded = true;
  await invoke("set_setting", { key: "tab_mru_reorder", value: String(v) });
}

/* ─── Per-tab search pulse (context menu → TerminalPane.openSearch) ─── */
let _searchRequest = $state<{ tabId: string; n: number } | null>(null);
export function searchRequest() { return _searchRequest; }
export function requestSearch(tabId: string) {
  _searchRequest = { tabId, n: (_searchRequest?.n ?? 0) + 1 };
}

/* ─── SFTP overlay (folder pick + multi-select are desktop-only; single-file
   transfer also works on mobile via plugin-fs + content:// URIs) ─── */
/** 给当前活跃 tab 开 SFTP。仅 ssh tab 有意义（共用其 SSH channel；local PTY
 *  没有远端文件系统）。这里 gate tab.type，防止键盘 navigate("sftp") 等路径
 *  绕过 UI，把 home/local/edit tab 错误标为 open。 */
export function openSftp() {
  if (!_activeTabId) return;
  const tab = _tabs.find(t => t.id === _activeTabId);
  if (!tab || tab.type !== "ssh") return;
  _sftpOpenByTab = { ..._sftpOpenByTab, [_activeTabId]: true };
}
export function closeSftp() {
  if (!_activeTabId || !_sftpOpenByTab[_activeTabId]) return;
  const next = { ..._sftpOpenByTab };
  delete next[_activeTabId];
  _sftpOpenByTab = next;
}

/* ─── Pinned profiles ─── */
function savePins() { safeSetItem("pinned_profiles", JSON.stringify(_pinnedProfileIds)); }
export function pinProfile(id: string) {
  if (!_pinnedProfileIds.includes(id)) { _pinnedProfileIds.push(id); savePins(); }
}
export function unpinProfile(id: string) {
  _pinnedProfileIds = _pinnedProfileIds.filter(x => x !== id); savePins();
}
export function isProfilePinned(id: string) { return _pinnedProfileIds.includes(id); }

/* ─── Legacy navigate (redirect to settings) ─── */
export function navigate(s: string, editId?: string) {
  if (s === "main") { _settingsActive = false; return; }
  if (s === "settings") { openSettings(); _settingsPage = "menu"; return; }
  if (s === "sftp") { openSftp(); return; }
  openSettings();
  settingsNavigate(s as SettingsPage, editId);
}
export function goBack() { settingsBack(); }

/* ═══════════════════════════════════════════════════════
   Data fetching helpers
   ═══════════════════════════════════════════════════════ */
export async function loadProfiles(): Promise<Profile[]> {
  return invoke<Profile[]>("list_profiles");
}
export async function loadCredentials(): Promise<Credential[]> {
  return invoke<Credential[]>("list_credentials");
}
export async function loadForwards(): Promise<Forward[]> {
  return invoke<Forward[]>("list_forwards");
}
export async function loadSerialProfiles(): Promise<SerialProfile[]> {
  // Desktop-only: the command isn't registered on Android. Degrade to [] rather
  // than rejecting, so callers (e.g. HomeScreen's Promise.all) don't break on mobile.
  // On desktop the command IS registered, so a failure is a real problem (DB /
  // serialization) — log it so it's diagnosable instead of silently showing "no
  // profiles". Mobile stays quiet (expected "not registered").
  return invoke<SerialProfile[]>("list_serial_profiles").catch((e) => {
    if (!isMobile) console.warn("[serial] list_serial_profiles failed:", e);
    return [];
  });
}
export async function loadTelnetProfiles(): Promise<TelnetProfile[]> {
  // Registered on every platform (plain TCP), so any failure is a real problem
  // (DB / serialization) — log it instead of silently showing "no profiles".
  return invoke<TelnetProfile[]>("list_telnet_profiles").catch((e) => {
    console.warn("[telnet] list_telnet_profiles failed:", e);
    return [];
  });
}
export async function loadDynamicDiscoverySources(): Promise<DynamicDiscoverySource[]> {
  return invoke<DynamicDiscoverySource[]>("list_dynamic_discovery_sources");
}
export async function saveDynamicDiscoverySources(sources: DynamicDiscoverySource[]): Promise<void> {
  await invoke("save_dynamic_discovery_sources", { sources });
}
export async function dynamicDiscoveryToolStatus(platform: DynamicPlatform): Promise<DynamicDiscoveryToolStatus> {
  return invoke<DynamicDiscoveryToolStatus>("dynamic_discovery_tool_status", { platform });
}
export async function loadDynamicDiscoveryContexts(platform: DynamicPlatform): Promise<DynamicDiscoveryContext[]> {
  return invoke<DynamicDiscoveryContext[]>("list_dynamic_discovery_contexts", { platform });
}
export async function discoverDynamicTargets(): Promise<DynamicDiscoverySnapshot> {
  return invoke<DynamicDiscoverySnapshot>("discover_dynamic_targets").catch((e) => {
    console.warn("[dynamic-discovery] discover failed:", e);
    return { targets: [], errors: [{ source_id: "", source_name: "dynamic discovery", platform: "docker", message: String(e) }] };
  });
}
export async function loadSnippets(): Promise<Snippet[]> {
  return invoke<Snippet[]>("load_snippets");
}
export async function loadHighlights(): Promise<HighlightRule[]> {
  return invoke<HighlightRule[]>("list_highlights");
}

/**
 * Bumped whenever the user adds/removes/resets a highlight rule via
 * HighlightManager. TerminalPane reads this in a `$effect` and reloads
 * its rule cache + recompiles its regex. Without this, every highlight
 * edit silently fails until the user reconnects the terminal — the kind
 * of "did this even save?" bug that erodes trust.
 *
 * A revision counter (not the full rule list) lives in the store so
 * consumers can subscribe with a single reactive dep regardless of how
 * the underlying list mutates, and so we don't double-store the rules
 * (DB is the source of truth; this is just a "go re-read" signal).
 */
let _highlightsRevision = $state(0);
export function highlightsRevision(): number { return _highlightsRevision; }
export function bumpHighlights() { _highlightsRevision += 1; }
export async function loadGroups(): Promise<Group[]> {
  return invoke<Group[]>("list_groups");
}
