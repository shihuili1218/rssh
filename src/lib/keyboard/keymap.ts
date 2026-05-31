/**
 * Keyboard shortcuts as data.
 *
 * A shortcut used to be a hand-written predicate (`e => e.metaKey && e.key==="w"`)
 * duplicated across the registry, the xterm handler, and the help screen. Here a
 * shortcut is split into a stable `ActionId` (behaviour, lives in code) and a
 * `KeyBinding` (which keys, pure data). `match` and the display label are derived
 * from the binding, so there is exactly one source of truth and the binding can be
 * overridden by the user.
 *
 * Matching is EXACT per modifier — ⌘W and Ctrl+W are different combos. This is
 * deliberate (D1): it stops app shortcuts from shadowing shell keys like Ctrl+W
 * (werase) on macOS.
 */

import type { MessageKey } from "../i18n/locales/en";

export interface KeyBinding {
  /** Normalized: single chars lower-cased ("w"), named keys verbatim ("Tab"). */
  key: string;
  meta?: boolean;
  ctrl?: boolean;
  alt?: boolean;
  shift?: boolean;
}

export type ActionId =
  | "tab.close"
  | "tab.clone"
  | "tab.openNewWindow"
  | "ai.toggle"
  | "term.search"
  | "term.sftp"
  | "term.snippet"
  | "term.paste"
  | "term.copy";

export type Surface = "global" | "terminal";

export interface ActionMeta {
  id: ActionId;
  labelKey: MessageKey;
  surface: Surface;
  /** Default binding on macOS. */
  mac: KeyBinding;
  /** Default binding everywhere else. */
  other: KeyBinding;
}

/** The "primary" modifier differs by platform: ⌘ on mac, Ctrl elsewhere. */
function primary(key: string, isMac: boolean, extra?: Omit<KeyBinding, "key">): KeyBinding {
  return isMac ? { key, meta: true, ...extra } : { key, ctrl: true, ...extra };
}

export const ACTIONS: readonly ActionMeta[] = [
  { id: "tab.close",         labelKey: "shortcut.tab.close",           surface: "global",   mac: primary("w", true),                         other: primary("w", false) },
  { id: "tab.clone",         labelKey: "shortcut.tab.clone",           surface: "global",   mac: primary("d", true, { shift: true }),        other: primary("d", false, { shift: true }) },
  { id: "tab.openNewWindow", labelKey: "shortcut.tab.open_new_window", surface: "global",   mac: primary("n", true, { shift: true }),        other: primary("n", false, { shift: true }) },
  { id: "ai.toggle",         labelKey: "shortcut.ai.toggle",           surface: "global",   mac: primary("a", true, { shift: true }),        other: primary("a", false, { shift: true }) },
  { id: "term.search",       labelKey: "shortcuts.search",             surface: "terminal", mac: primary("f", true),                         other: primary("f", false) },
  { id: "term.sftp",         labelKey: "shortcuts.open_sftp",          surface: "terminal", mac: primary("o", true),                         other: primary("o", false) },
  { id: "term.snippet",      labelKey: "shortcuts.snippet",            surface: "terminal", mac: primary("s", true),                         other: primary("s", false) },
  // Copy/paste keep the literal Ctrl+Shift convention on every platform
  // (⌘C/⌘V still reach xterm natively on mac because they don't match these).
  { id: "term.paste",        labelKey: "shortcut.term.paste",          surface: "terminal", mac: { key: "v", ctrl: true, shift: true },       other: { key: "v", ctrl: true, shift: true } },
  { id: "term.copy",         labelKey: "shortcut.term.copy",           surface: "terminal", mac: { key: "c", ctrl: true, shift: true },       other: { key: "c", ctrl: true, shift: true } },
];

const ACTION_IDS = new Set<string>(ACTIONS.map((a) => a.id));

export function defaultBinding(id: ActionId, isMac: boolean): KeyBinding {
  const a = ACTIONS.find((x) => x.id === id);
  if (!a) throw new Error(`unknown action: ${id}`);
  return { ...(isMac ? a.mac : a.other) };
}

/** The subset of a KeyboardEvent we need — keeps the module test-friendly. */
export type KeyEventLike = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">;

export function matchBinding(e: KeyEventLike, b: KeyBinding): boolean {
  return (
    e.key.toLowerCase() === b.key.toLowerCase() &&
    e.metaKey === !!b.meta &&
    e.ctrlKey === !!b.ctrl &&
    e.altKey === !!b.alt &&
    e.shiftKey === !!b.shift
  );
}

/** Turn a captured keydown into a binding (for the "record a shortcut" UI). */
export function eventToBinding(e: KeyEventLike): KeyBinding {
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  const b: KeyBinding = { key };
  if (e.metaKey) b.meta = true;
  if (e.ctrlKey) b.ctrl = true;
  if (e.altKey) b.alt = true;
  if (e.shiftKey) b.shift = true;
  return b;
}

function keyLabel(key: string): string {
  return key.length === 1 ? key.toUpperCase() : key;
}

export function formatBinding(b: KeyBinding, isMac: boolean): string {
  if (isMac) {
    let s = "";
    if (b.meta) s += "⌘";
    if (b.ctrl) s += "⌃";
    if (b.alt) s += "⌥";
    if (b.shift) s += "⇧";
    return s + keyLabel(b.key);
  }
  const parts: string[] = [];
  if (b.ctrl) parts.push("Ctrl");
  if (b.alt) parts.push("Alt");
  if (b.shift) parts.push("Shift");
  if (b.meta) parts.push("Meta");
  parts.push(keyLabel(b.key));
  return parts.join("+");
}

/** Canonical identity of a combo, for conflict grouping / equality. */
export function bindingKey(b: KeyBinding): string {
  return `${b.meta ? 1 : 0}${b.ctrl ? 1 : 0}${b.alt ? 1 : 0}${b.shift ? 1 : 0}:${b.key.toLowerCase()}`;
}

const MODIFIER_KEYS = new Set(["control", "shift", "alt", "meta"]);

/**
 * True if `key` is a modifier key pressed on its own (Control/Shift/Alt/Meta),
 * case-insensitive. Single source for "not a usable binding key" — the recorder
 * ignores these while capturing, and validateBinding rejects them on load.
 */
export function isModifierKey(key: string): boolean {
  return MODIFIER_KEYS.has(key.toLowerCase());
}

/**
 * A customizable binding MUST carry a real modifier (⌘/Ctrl/Alt) and MUST NOT be
 * a lone modifier key. A bare/shift-only key would silently eat shell input; a
 * modifier-key binding (e.g. {key:"Control"}) would fire on every Control press.
 */
export function validateBinding(b: KeyBinding): { ok: boolean; reason?: "no-modifier" | "modifier-key" } {
  if (isModifierKey(b.key)) return { ok: false, reason: "modifier-key" };
  if (b.meta || b.ctrl || b.alt) return { ok: true };
  return { ok: false, reason: "no-modifier" };
}

/** True if `b` is exactly this action's default on the given platform. */
export function isDefaultBinding(id: ActionId, b: KeyBinding, isMac: boolean): boolean {
  return bindingKey(b) === bindingKey(defaultBinding(id, isMac));
}

export type Overrides = Partial<Record<ActionId, KeyBinding>>;

export function effectiveBindings(overrides: Overrides, isMac: boolean): Record<ActionId, KeyBinding> {
  const out = {} as Record<ActionId, KeyBinding>;
  for (const a of ACTIONS) {
    out[a.id] = overrides[a.id] ? { ...overrides[a.id]! } : defaultBinding(a.id, isMac);
  }
  return out;
}

/**
 * The combos the global tab-cycler owns: Ctrl+Tab (forward) and Ctrl+Shift+Tab
 * (back). SINGLE SOURCE OF TRUTH — AppShell's cycler matches via
 * `TAB_CYCLE.some(b => matchBinding(e, b))` and RESERVED is derived from it, so
 * the two can't drift and a customizable binding can never silently land on a
 * combo the (now exact-matched) cycler eats.
 */
export const TAB_CYCLE: readonly KeyBinding[] = [
  { key: "Tab", ctrl: true },
  { key: "Tab", ctrl: true, shift: true },
];

/**
 * Combos owned by fixed (non-customizable) interactions. A customizable binding
 * must not steal these. Only combos that survive `validateBinding` need listing;
 * bare/no-modifier ones (Esc, arrows, Enter) are already rejected.
 */
export const RESERVED: readonly { labelKey: MessageKey; binding: KeyBinding }[] =
  TAB_CYCLE.map((binding) => ({ labelKey: "shortcut.tab.cycle" as MessageKey, binding }));

/** If `b` matches a reserved fixed combo, returns its label key; else null. */
export function reservedConflict(b: KeyBinding): MessageKey | null {
  const k = bindingKey(b);
  const hit = RESERVED.find((r) => bindingKey(r.binding) === k);
  return hit ? hit.labelKey : null;
}

/**
 * The other action whose effective binding equals `b` (skipping `id`), or null.
 * The single collision primitive shared by the record guard and the reset guard,
 * so neither path can quietly create a duplicate binding.
 */
export function collidingAction(
  id: ActionId,
  b: KeyBinding,
  eff: Record<ActionId, KeyBinding>,
): ActionId | null {
  const k = bindingKey(b);
  for (const a of ACTIONS) {
    if (a.id === id) continue;
    if (bindingKey(eff[a.id]) === k) return a.id;
  }
  return null;
}

/** Groups of actions (>=2) that resolve to the same combo. */
export function findConflicts(eff: Record<ActionId, KeyBinding>): ActionId[][] {
  const byCombo = new Map<string, ActionId[]>();
  for (const a of ACTIONS) {
    const k = bindingKey(eff[a.id]);
    const group = byCombo.get(k);
    if (group) group.push(a.id);
    else byCombo.set(k, [a.id]);
  }
  return [...byCombo.values()].filter((g) => g.length > 1);
}

export function serializeOverrides(o: Overrides): string {
  return JSON.stringify(o);
}

export function parseOverrides(raw: string | null | undefined): Overrides {
  if (!raw) return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {};
  }
  if (!parsed || typeof parsed !== "object") return {};
  const out: Overrides = {};
  for (const [id, val] of Object.entries(parsed as Record<string, unknown>)) {
    if (!ACTION_IDS.has(id)) continue;
    if (!val || typeof val !== "object") continue;
    const key = (val as { key?: unknown }).key;
    if (typeof key !== "string" || key.length === 0) continue;
    const b = val as Record<string, unknown>;
    const clean: KeyBinding = { key };
    if (b.meta === true) clean.meta = true;
    if (b.ctrl === true) clean.ctrl = true;
    if (b.alt === true) clean.alt = true;
    if (b.shift === true) clean.shift = true;
    // Apply the same per-binding guards as the recorder, so a corrupt or
    // hand-edited setting can't enable an invalid/unreachable binding.
    if (!validateBinding(clean).ok) continue;
    if (reservedConflict(clean)) continue;
    out[id as ActionId] = clean;
  }
  return out;
}
