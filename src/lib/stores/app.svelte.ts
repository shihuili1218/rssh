import { invoke } from "@tauri-apps/api/core";

/* ═══════════════════════════════════════════════════════
   Platform
   ═══════════════════════════════════════════════════════ */
export const isMobile = /Android|iPhone|iPad/i.test(navigator.userAgent);

/* ═══════════════════════════════════════════════════════
   Types
   ═══════════════════════════════════════════════════════ */
export type TabType = "home" | "ssh" | "local" | "forward";
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
  | "cli"
  | "help";

export interface Profile {
  id: string; name: string; host: string; port: number;
  credential_id: string | null; bastion_profile_id: string | null; init_command: string | null;
}
export interface Credential {
  id: string; name: string; username: string;
  type: string; secret: string | null; save_to_remote: boolean;
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

/* ─── Getters ─── */
export function tabs() { return _tabs; }
export function activeTabId() { return _activeTabId; }
export function activeTab() { return _tabs.find(t => t.id === _activeTabId); }
export function settingsActive() { return _settingsActive; }
export function settingsPage() { return _settingsPage; }
export function editingId() { return _editingId; }
export function sftpOpen() { return _sftpOpen; }
export function pinnedProfileIds() { return _pinnedProfileIds; }

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

export function closeTab(id: string) {
  const idx = _tabs.findIndex(t => t.id === id);
  if (idx < 0 || _tabs[idx].type === "home") return;
  _tabs.splice(idx, 1);
  if (_activeTabId === id) {
    _activeTabId = _tabs[Math.min(idx, _tabs.length - 1)]?.id ?? "home";
  }
}

export function updateTabLabel(id: string, label: string) {
  const tab = _tabs.find(t => t.id === id);
  if (tab) tab.label = label;
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

/* ─── Snippet picker ─── */
let _snippetPickerOpen = $state(false);
export function snippetPickerOpen() { return _snippetPickerOpen; }
export function openSnippetPicker() { _snippetPickerOpen = true; }
export function closeSnippetPicker() { _snippetPickerOpen = false; }

/* ─── SFTP overlay ─── */
export function openSftp() { _sftpOpen = true; }
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
