/**
 * Theme store — applies palette to :root CSS variables and notifies
 * xterm instances of theme changes.
 *
 * Design:
 *   • Palette is the single source of truth (see palettes.ts).
 *   • UI consumes CSS variables; this module writes them on :root.
 *   • xterm instances register a callback that receives the current
 *     palette immediately and on every change.
 *   • Persistence uses Tauri get_setting/set_setting (cross-device,
 *     consistent with command_block_bar / shell settings).
 *
 * Init order: main.ts calls `init()` once before mount so the user's
 * saved palette is applied before first paint (or close to it).
 */

import { invoke } from "@tauri-apps/api/core";
import {
  DEFAULT_PALETTE_ID,
  PALETTES,
  paletteById,
  type Palette,
  type PaletteId,
  type PaletteTerm,
} from "./palettes.ts";
import {
  DEFAULT_TERM_REF,
  TERM_PRESETS,
  isTermPaletteRef,
  termPresetById,
  type TermPaletteRef,
} from "./term-palettes.ts";
import { composeTermFontStack } from "./term-font.ts";

const SETTING_KEY_PALETTE        = "theme.palette";
const SETTING_KEY_SHAPE          = "theme.shape";
const SETTING_KEY_DENSITY        = "theme.density";
const SETTING_KEY_TERM           = "theme.term-palette";
const SETTING_KEY_TERM_BG_FOLLOW = "theme.term-bg-follow";
const SETTING_KEY_TERM_FONT      = "theme.term-font";
const SETTING_KEY_TERM_FONT_SIZE = "theme.term-font-size";

let _paletteId = $state<PaletteId>(DEFAULT_PALETTE_ID);

/* ───────────────────────────────────────────────────────────────
   Shape — visual language. Switches the box-shadow / border /
   backdrop-filter rules without touching components. The actual
   CSS lives in src/styles/shapes/*.css, scoped under
   [data-shape="..."] on <html>.
   ─────────────────────────────────────────────────────────────── */

export type ShapeId = "neumorphism" | "flat" | "material";

export const SHAPES: readonly { id: ShapeId; label: string }[] = [
  { id: "neumorphism", label: "Neumorphism" },
  { id: "flat",        label: "Flat" },
  { id: "material",    label: "Material" },
];

const DEFAULT_SHAPE_ID: ShapeId = "neumorphism";

let _shapeId = $state<ShapeId>(DEFAULT_SHAPE_ID);

function applyShape(id: ShapeId): void {
  document.documentElement.dataset.shape = id;
}

export function shapeId(): ShapeId { return _shapeId; }
export function listShapes(): readonly { id: ShapeId; label: string }[] { return SHAPES; }

export async function setShape(id: ShapeId): Promise<void> {
  _shapeId = id;
  applyShape(id);
  try {
    await invoke("set_setting", { key: SETTING_KEY_SHAPE, value: id });
  } catch {
    // Persistence failure is non-fatal.
  }
}

/* ───────────────────────────────────────────────────────────────
   Density — padding/gap multiplier driven by --density CSS var.
   Compact = 0.85, Cozy = 1.0 (default), Comfortable = 1.15.
   ─────────────────────────────────────────────────────────────── */

export type DensityId = "compact" | "cozy" | "comfortable";

export const DENSITIES: readonly { id: DensityId; label: string }[] = [
  { id: "compact",     label: "Compact" },
  { id: "cozy",        label: "Cozy" },
  { id: "comfortable", label: "Comfortable" },
];

const DEFAULT_DENSITY_ID: DensityId = "cozy";

let _densityId = $state<DensityId>(DEFAULT_DENSITY_ID);

function applyDensity(id: DensityId): void {
  document.documentElement.dataset.density = id;
}

export function densityId(): DensityId { return _densityId; }
export function listDensities(): readonly { id: DensityId; label: string }[] { return DENSITIES; }

export async function setDensity(id: DensityId): Promise<void> {
  _densityId = id;
  applyDensity(id);
  try {
    await invoke("set_setting", { key: SETTING_KEY_DENSITY, value: id });
  } catch {
    // Persistence failure is non-fatal.
  }
}

function isDensityId(v: string | null | undefined): v is DensityId {
  return v === "compact" || v === "cozy" || v === "comfortable";
}

/* ───────────────────────────────────────────────────────────────
   Terminal palette — independent of the UI palette. Default is
   "inherit" (use the UI palette's term part). Users can pick a
   built-in preset or paste a custom xterm.js JSON.
   ─────────────────────────────────────────────────────────────── */

let _termRef = $state<TermPaletteRef>(DEFAULT_TERM_REF);
let _termBgFollowsTheme = $state<boolean>(true);

/**
 * Resolve the current term ref to an actual PaletteTerm — what xterm
 * should actually display. inherit → UI palette term; preset → looked
 * up; custom → embedded.
 *
 * When `termBgFollowsTheme` is on (default), the background is overridden
 * to the UI palette's --bg so the terminal visually merges with the
 * surrounding chrome. When off, the preset/custom keeps its own
 * background — useful when the user wants a Solarized terminal inside
 * a neutral chrome, even at the cost of a visible seam.
 */
export function currentTermTheme(): PaletteTerm {
  const ui = paletteById(_paletteId);
  let term: PaletteTerm;
  if (_termRef.kind === "preset") {
    const preset = termPresetById(_termRef.id);
    term = preset?.term ?? ui.term;
  } else if (_termRef.kind === "custom") {
    term = _termRef.term;
  } else {
    term = ui.term;
  }
  return _termBgFollowsTheme ? { ...term, background: ui.ui.bg } : term;
}

export function termPaletteRef(): TermPaletteRef { return _termRef; }
export function listTermPresets() { return TERM_PRESETS; }
export function termBgFollowsTheme(): boolean { return _termBgFollowsTheme; }

export async function setTermPalette(ref: TermPaletteRef): Promise<void> {
  _termRef = ref;
  writeTermVars();
  notifyXterms();
  try {
    await invoke("set_setting", { key: SETTING_KEY_TERM, value: JSON.stringify(ref) });
  } catch {
    // Persistence failure is non-fatal.
  }
}

export async function setTermBgFollowsTheme(on: boolean): Promise<void> {
  _termBgFollowsTheme = on;
  writeTermVars();
  notifyXterms();
  try {
    await invoke("set_setting", { key: SETTING_KEY_TERM_BG_FOLLOW, value: String(on) });
  } catch {
    // Persistence failure is non-fatal.
  }
}

/* ───────────────────────────────────────────────────────────────
   xterm theme subscribers — terminal panes register here
   so they are notified whenever the palette changes.
   ─────────────────────────────────────────────────────────────── */

type XtermThemeListener = (term: PaletteTerm) => void;
const _xtermListeners = new Set<XtermThemeListener>();

/**
 * Register a callback that receives the xterm theme on every palette
 * or term-palette change. Called once immediately with the current
 * effective term theme so the caller does not need to read the store.
 *
 * Returns an unregister function.
 */
export function registerXtermThemeListener(fn: XtermThemeListener): () => void {
  _xtermListeners.add(fn);
  fn(currentTermTheme());
  return () => { _xtermListeners.delete(fn); };
}

/* ───────────────────────────────────────────────────────────────
   Terminal font — the user picks one installed family (prepended to
   BASE_FONT_STACK, see term-font.ts) and a pixel size. Both alter
   xterm's cell metrics, so a change requires a refit — one listener
   carries family + size together. Stored values: family name (empty =
   base stack = historical default) and size (default 13 = the value
   xterm was hardcoded to before this was configurable).
   Independent of palette: own listener set, notified on change.
   ─────────────────────────────────────────────────────────────── */

const DEFAULT_TERM_FONT_SIZE = 13;
export const termFontSizeBounds = { min: 8, max: 32, def: DEFAULT_TERM_FONT_SIZE } as const;

function clampFontSize(px: number): number {
  if (!Number.isFinite(px)) return DEFAULT_TERM_FONT_SIZE;
  return Math.max(termFontSizeBounds.min, Math.min(termFontSizeBounds.max, Math.round(px)));
}

let _termFont = $state<string>("");
let _termFontSize = $state<number>(DEFAULT_TERM_FONT_SIZE);

export function termFont(): string { return _termFont; }
export function termFontSize(): number { return _termFontSize; }
export function currentTermFontStack(): string { return composeTermFontStack(_termFont); }

export async function setTermFont(name: string): Promise<void> {
  _termFont = name;
  notifyXtermFonts();
  try {
    await invoke("set_setting", { key: SETTING_KEY_TERM_FONT, value: name });
  } catch {
    // Persistence failure is non-fatal.
  }
}

export async function setTermFontSize(px: number): Promise<void> {
  _termFontSize = clampFontSize(px);
  notifyXtermFonts();
  try {
    await invoke("set_setting", { key: SETTING_KEY_TERM_FONT_SIZE, value: String(_termFontSize) });
  } catch {
    // Persistence failure is non-fatal.
  }
}

type XtermFont = { family: string; size: number };
type XtermFontListener = (font: XtermFont) => void;
const _xtermFontListeners = new Set<XtermFontListener>();

function currentXtermFont(): XtermFont {
  return { family: currentTermFontStack(), size: _termFontSize };
}

/**
 * Register a callback that receives the xterm font (family + size) now and on
 * every font/size change. Returns an unregister function. Mirrors
 * registerXtermThemeListener; both fields change cell metrics, so the caller
 * must refit after applying.
 */
export function registerXtermFontListener(fn: XtermFontListener): () => void {
  _xtermFontListeners.add(fn);
  fn(currentXtermFont());
  return () => { _xtermFontListeners.delete(fn); };
}

function notifyXtermFonts(): void {
  const font = currentXtermFont();
  for (const fn of _xtermFontListeners) fn(font);
}

/* ───────────────────────────────────────────────────────────────
   Apply: write palette → :root CSS variables + notify xterm.
   ─────────────────────────────────────────────────────────────── */

function writeRootVars(p: Palette): void {
  const root = document.documentElement;
  const ui = p.ui;
  root.style.setProperty("--bg",           ui.bg);
  root.style.setProperty("--surface",      ui.surface);
  root.style.setProperty("--shadow-dark",  ui.shadowDark);
  root.style.setProperty("--shadow-light", ui.shadowLight);
  root.style.setProperty("--divider",      ui.divider);
  root.style.setProperty("--text",         ui.text);
  root.style.setProperty("--text-sub",     ui.textSub);
  root.style.setProperty("--text-dim",     ui.textDim);
  root.style.setProperty("--accent",       ui.accent);
  root.style.setProperty("--error",        ui.error);
  root.style.setProperty("--success",      ui.success);
  root.style.setProperty("--warning",      ui.warning);
  root.style.setProperty("--magenta",      ui.magenta);
  root.style.setProperty("--purple",       ui.purple);
  // Derived: --accent-soft is accent at --alpha-soft.
  // We compute via color-mix so it auto-tracks.
  root.style.setProperty("--accent-soft", `color-mix(in srgb, ${ui.accent} 15%, transparent)`);
  // Mode hint (light/dark) on <html> for any future shape variants.
  root.dataset.mode = p.mode;
}

/**
 * Write --term-* CSS variables from the *effective* term theme (which
 * may be a preset/custom, not the UI palette's term). Called whenever
 * either the UI palette or the term-palette ref changes — keeps CSS
 * in sync with what xterm actually displays. */
function writeTermVars(): void {
  const root = document.documentElement;
  const t = currentTermTheme();
  root.style.setProperty("--term-bg",     t.background);
  root.style.setProperty("--term-fg",     t.foreground);
  root.style.setProperty("--term-cursor", t.cursor ?? t.foreground);
  root.style.setProperty("--term-sel",    t.selectionBackground ?? "");
  // ANSI 16 色 —— 给 CodeMirror / SnippetPicker 等"想跟终端配色走"的组件用。
  // PaletteTerm 字段全是 optional（custom palette 可能没填），缺省用 fg 兜底
  // —— 比硬塞个 fallback 颜色（如 "#888"）更可控：用户没配的色不会出现在画面里。
  const fb = t.foreground;
  root.style.setProperty("--term-black",          t.black          ?? fb);
  root.style.setProperty("--term-red",            t.red            ?? fb);
  root.style.setProperty("--term-green",          t.green          ?? fb);
  root.style.setProperty("--term-yellow",         t.yellow         ?? fb);
  root.style.setProperty("--term-blue",           t.blue           ?? fb);
  root.style.setProperty("--term-magenta",        t.magenta        ?? fb);
  root.style.setProperty("--term-cyan",           t.cyan           ?? fb);
  root.style.setProperty("--term-white",          t.white          ?? fb);
  root.style.setProperty("--term-bright-black",   t.brightBlack    ?? t.black   ?? fb);
  root.style.setProperty("--term-bright-red",     t.brightRed      ?? t.red     ?? fb);
  root.style.setProperty("--term-bright-green",   t.brightGreen    ?? t.green   ?? fb);
  root.style.setProperty("--term-bright-yellow",  t.brightYellow   ?? t.yellow  ?? fb);
  root.style.setProperty("--term-bright-blue",    t.brightBlue     ?? t.blue    ?? fb);
  root.style.setProperty("--term-bright-magenta", t.brightMagenta  ?? t.magenta ?? fb);
  root.style.setProperty("--term-bright-cyan",    t.brightCyan     ?? t.cyan    ?? fb);
  root.style.setProperty("--term-bright-white",   t.brightWhite    ?? t.white   ?? fb);
}

function notifyXterms(): void {
  const t = currentTermTheme();
  for (const fn of _xtermListeners) fn(t);
}

function apply(p: Palette): void {
  writeRootVars(p);
  writeTermVars();
  notifyXterms();
}

/* ───────────────────────────────────────────────────────────────
   Public API
   ─────────────────────────────────────────────────────────────── */

export function paletteId(): PaletteId { return _paletteId; }
export function currentPalette(): Palette { return paletteById(_paletteId); }
export function listPalettes(): readonly Palette[] { return PALETTES; }

export async function setPalette(id: PaletteId): Promise<void> {
  _paletteId = id;
  apply(paletteById(id));
  try {
    await invoke("set_setting", { key: SETTING_KEY_PALETTE, value: id });
  } catch {
    // Persistence failure is non-fatal — the in-memory state is still applied.
  }
}

function isShapeId(v: string | null | undefined): v is ShapeId {
  return v === "neumorphism" || v === "flat" || v === "material";
}

/**
 * Initialise from persisted settings and apply. Call once at startup,
 * before mounting the Svelte tree, so first paint reflects the user's
 * choice (the :root literal defaults will be overwritten in-place).
 *
 * Loads palette + shape + density in parallel — all are independent.
 */
export async function init(): Promise<void> {
  const [palette, shape, density, termRaw, termBgFollow, termFontRaw, termFontSizeRaw] = await Promise.all([
    invoke<string | null>("get_setting", { key: SETTING_KEY_PALETTE        }).catch(() => null),
    invoke<string | null>("get_setting", { key: SETTING_KEY_SHAPE          }).catch(() => null),
    invoke<string | null>("get_setting", { key: SETTING_KEY_DENSITY        }).catch(() => null),
    invoke<string | null>("get_setting", { key: SETTING_KEY_TERM           }).catch(() => null),
    invoke<string | null>("get_setting", { key: SETTING_KEY_TERM_BG_FOLLOW }).catch(() => null),
    invoke<string | null>("get_setting", { key: SETTING_KEY_TERM_FONT      }).catch(() => null),
    invoke<string | null>("get_setting", { key: SETTING_KEY_TERM_FONT_SIZE }).catch(() => null),
  ]);
  if (palette && PALETTES.some((p) => p.id === palette)) {
    _paletteId = palette as PaletteId;
  }
  if (isShapeId(shape)) {
    _shapeId = shape;
  }
  if (isDensityId(density)) {
    _densityId = density;
  }
  if (termRaw) {
    try {
      const parsed = JSON.parse(termRaw);
      if (isTermPaletteRef(parsed)) _termRef = parsed;
    } catch {
      // Corrupted persisted value — keep default inherit.
    }
  }
  // Default true — keeps the existing "terminal merges with chrome" look
  // for users who haven't touched the new toggle. Only an explicit "false"
  // string opts out.
  if (termBgFollow === "false") _termBgFollowsTheme = false;
  if (termFontRaw) _termFont = termFontRaw;
  if (termFontSizeRaw) _termFontSize = clampFontSize(parseInt(termFontSizeRaw, 10));
  apply(paletteById(_paletteId));
  applyShape(_shapeId);
  applyDensity(_densityId);
  // init() is not awaited before mount (see main.ts), so a terminal may
  // register its font listener before this resolves — with the default stack.
  // Notify now so any already-mounted terminal picks up the persisted font.
  // Mirrors apply()'s notifyXterms() for the palette.
  notifyXtermFonts();
}
