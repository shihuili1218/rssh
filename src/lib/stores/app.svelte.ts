import { invoke } from "@tauri-apps/api/core";

/* ═══════════════════════════════════════════════════════
   Platform
   ═══════════════════════════════════════════════════════ */
export const isMobile = /Android|iPhone|iPad/i.test(navigator.userAgent);

/* ═══════════════════════════════════════════════════════
   Types
   ═══════════════════════════════════════════════════════ */
export type TabType = "home" | "ssh" | "local" | "forward" | "edit";
export interface Tab {
  id: string;
  type: TabType;
  label: string;
  meta?: Record<string, string>;
}

/** Settings sub-pages (rendered inside the settings tab) */
export type SettingsPage =
  | "menu"
  | "profiles"
  | "profile-edit"
  | "credentials"
  | "credential-edit"
  | "forwards"
  | "forward-edit"
  | "snippets"
  | "highlights"
  | "github-sync"
  | "import-export"
  | "recording-settings"
  | "playback"
  | "shell-settings"
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
export interface Profile {
  id: string; name: string; host: string; port: number;
  credential_id: string | null; bastion_profile_id: string | null; init_command: string | null;
  group_id: string | null;
}
export interface Credential {
  id: string; name: string; username: string;
  type: string; secret: string | null; save_to_remote: boolean;
  passphrase: string | null;
}
export interface Forward {
  id: string; name: string; type: string;
  local_port: number; remote_host: string; remote_port: number; profile_id: string;
}
export interface Snippet { name: string; command: string; }
export interface HighlightRule { keyword: string; color: string; enabled: boolean; }
export interface RemoteEntry { name: string; is_dir: boolean; size: number; }

/* ═══════════════════════════════════════════════════════
   Reactive state
   ═══════════════════════════════════════════════════════ */
let _tabs = $state<Tab[]>([{ id: "home", type: "home", label: "Home" }]);
let _activeTabId = $state("home");
let _settingsActive = $state(false);
let _settingsPage = $state<SettingsPage>("menu");
let _editingId = $state<string | null>(null);

/* SFTP overlay (opened from terminal via ⌘O) */
let _sftpOpen = $state(false);
let _pinnedProfileIds = $state<string[]>(JSON.parse(localStorage.getItem("pinned_profiles") ?? "[]"));

/* Terminal title (from remote shell OSC sequence), separate from tab label */
let _terminalTitles = $state<Record<string, string>>({});

/* ─── Getters ─── */
export function tabs() { return _tabs; }
export function activeTabId() { return _activeTabId; }
export function activeTab() { return _tabs.find(t => t.id === _activeTabId); }
export function settingsActive() { return _settingsActive; }
export function settingsPage() { return _settingsPage; }
export function editingId() { return _editingId; }
export function sftpOpen() { return _sftpOpen; }
export function pinnedProfileIds() { return _pinnedProfileIds; }
export function terminalTitle(tabId: string) { return _terminalTitles[tabId]; }

/* ─── Tab Operations ─── */
export function setActiveTab(id: string) {
  _activeTabId = id;
  _settingsActive = false;
  _sftpOpen = false;
}

export function addTab(tab: Tab) {
  _tabs.push(tab);
  _activeTabId = tab.id;
  _settingsActive = false;
  _sftpOpen = false;
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
  if (wasActive) {
    _activeTabId = _tabs[Math.min(idx, _tabs.length - 1)]?.id ?? "home";
    _sftpOpen = false;
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
  _sftpOpen = false;
}

export function settingsNavigate(page: SettingsPage, editId?: string) {
  _settingsPage = page;
  _editingId = editId ?? null;
}

export function settingsBack() {
  if (_settingsPage === "profile-edit") _settingsPage = "profiles";
  else if (_settingsPage === "credential-edit") _settingsPage = "credentials";
  else if (_settingsPage === "forward-edit") _settingsPage = "forwards";
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
let _sidebarPosDesktop = $state<SidebarPosition>(_loadSidebarPos(_SB_KEY_DESKTOP, "left"));
let _sidebarPosMobile = $state<SidebarPosition>(_loadSidebarPos(_SB_KEY_MOBILE, "top"));
export function sidebarPosition(): SidebarPosition {
  return isMobile ? _sidebarPosMobile : _sidebarPosDesktop;
}
export function setSidebarPosition(pos: SidebarPosition) {
  if (isMobile) {
    _sidebarPosMobile = pos;
    localStorage.setItem(_SB_KEY_MOBILE, pos);
  } else {
    _sidebarPosDesktop = pos;
    localStorage.setItem(_SB_KEY_DESKTOP, pos);
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

/** Read system clipboard. On desktop, goes through Rust to bypass
 *  WebKit's permission prompt for externally-sourced content. */
export async function readClipboard(): Promise<string> {
  if (isMobile) {
    return navigator.clipboard.readText().catch(() => "");
  }
  return invoke<string>("clipboard_read").catch(() => "");
}

/* ─── Session registry (for broadcast) ─── */
interface SessionEntry {
  tabId: string;
  sessionId: string;
  type: "ssh" | "local";
}
export interface SessionInfo extends SessionEntry {
  label: string;
}
let _sessions = $state<SessionEntry[]>([]);

export function registerSession(info: SessionEntry) {
  _sessions = [..._sessions.filter(s => s.tabId !== info.tabId), info];
}
export function unregisterSession(tabId: string) {
  _sessions = _sessions.filter(s => s.tabId !== tabId);
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
  const data = Array.from(new TextEncoder().encode(text));
  for (const tabId of tabIds) {
    const s = _sessions.find(x => x.tabId === tabId);
    if (!s) continue;
    const cmd = s.type === "local" ? "pty_write" : "ssh_write";
    invoke(cmd, { sessionId: s.sessionId, data });
  }
}

/* ─── Snippet picker ─── */
let _snippetPickerOpen = $state(false);
export function snippetPickerOpen() { return _snippetPickerOpen; }
export function openSnippetPicker() { _snippetPickerOpen = true; }
export function closeSnippetPicker() { _snippetPickerOpen = false; }

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

/* ─── Per-tab search pulse (context menu → TerminalPane.openSearch) ─── */
let _searchRequest = $state<{ tabId: string; n: number } | null>(null);
export function searchRequest() { return _searchRequest; }
export function requestSearch(tabId: string) {
  _searchRequest = { tabId, n: (_searchRequest?.n ?? 0) + 1 };
}

/* ─── SFTP overlay (desktop only — rfd has no Android native dialog) ─── */
export function openSftp() { if (!isMobile) _sftpOpen = true; }
export function closeSftp() { _sftpOpen = false; }

/* ─── Pinned profiles ─── */
function savePins() { localStorage.setItem("pinned_profiles", JSON.stringify(_pinnedProfileIds)); }
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
export async function loadSnippets(): Promise<Snippet[]> {
  return invoke<Snippet[]>("load_snippets");
}
export async function loadHighlights(): Promise<HighlightRule[]> {
  return invoke<HighlightRule[]>("list_highlights");
}
export async function loadGroups(): Promise<Group[]> {
  return invoke<Group[]>("list_groups");
}
