/**
 * Keymap store: the user's binding overrides + persistence.
 *
 * Thin reactive glue over the pure helpers in `lib/keyboard/keymap.ts`. Holds the
 * override map ({} = all defaults), persists it to the generic `set_setting` KV
 * store, and resolves effective bindings on demand. Dispatchers (AppShell global
 * shortcuts, TerminalPane xterm handler) read `binding(id)` live on every keydown,
 * so a rebind takes effect immediately with no re-attach.
 */

import { invoke } from "@tauri-apps/api/core";

import {
  ACTIONS,
  defaultBinding,
  effectiveBindings,
  findConflicts,
  formatBinding,
  isDefaultBinding,
  parseOverrides,
  serializeOverrides,
  type ActionId,
  type KeyBinding,
  type Overrides,
} from "../keyboard/keymap.ts";

const SETTING_KEY = "keymap_overrides";

export const isMac =
  typeof navigator !== "undefined" && typeof navigator.platform === "string"
    ? navigator.platform.startsWith("Mac")
    : false;

let _overrides = $state<Overrides>({});
let _recording = $state(false);
let _loaded = false;

/** Load overrides once from the backend. Idempotent; best-effort (defaults on error). */
export async function init(): Promise<void> {
  if (_loaded) return;
  _loaded = true;
  try {
    const raw = await invoke<string | null>("get_setting", { key: SETTING_KEY });
    _overrides = parseOverrides(raw);
  } catch {
    _overrides = {};
  }
}

// Serialize writes through a chain so rapid edits (e.g. reset then re-bind) land
// in call order — otherwise two fire-and-forget invokes could complete reversed
// and persist the older value last, reverting the change on reload. The value is
// snapshotted at call time; each write runs whether the previous one resolved or
// rejected (best-effort, like before).
let _persistChain: Promise<unknown> = Promise.resolve();

function persist(): void {
  const value = serializeOverrides(_overrides);
  const write = () => invoke("set_setting", { key: SETTING_KEY, value });
  _persistChain = _persistChain.then(write, write);
  _persistChain.catch(() => {});
}

/** Effective binding for an action (override if present, else platform default). */
export function binding(id: ActionId): KeyBinding {
  return _overrides[id] ?? defaultBinding(id, isMac);
}

/** Display string for an action's effective binding. */
export function format(id: ActionId): string {
  return formatBinding(binding(id), isMac);
}

export function isOverridden(id: ActionId): boolean {
  return !!_overrides[id];
}

/** Full effective map — for the editor list and conflict checks. */
export function effective(): Record<ActionId, KeyBinding> {
  return effectiveBindings(_overrides, isMac);
}

/** Groups of actions that resolve to the same combo (empty = no conflicts). */
export function conflicts(): ActionId[][] {
  return findConflicts(effective());
}

export function setOverride(id: ActionId, b: KeyBinding): void {
  // Recording the platform default is a reset, not an override — keeps the map to
  // genuine deviations (no spurious "Reset" button, and future default changes
  // still propagate).
  if (isDefaultBinding(id, b, isMac)) {
    reset(id);
    return;
  }
  _overrides = { ..._overrides, [id]: b };
  persist();
}

export function reset(id: ActionId): void {
  const next = { ..._overrides };
  delete next[id];
  _overrides = next;
  persist();
}

export function resetAll(): void {
  _overrides = {};
  persist();
}

/** True while the editor is capturing a new combo. */
export function recording(): boolean {
  return _recording;
}

export function setRecording(v: boolean): void {
  _recording = v;
}

export { ACTIONS };
