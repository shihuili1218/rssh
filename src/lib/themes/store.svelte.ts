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

const SETTING_KEY_PALETTE   = "theme.palette";
const SETTING_KEY_SHAPE     = "theme.shape";
const SETTING_KEY_DENSITY   = "theme.density";
const SETTING_KEY_TERM      = "theme.term-palette";

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

/**
 * Resolve the current term ref to an actual PaletteTerm — what xterm
 * should actually display. inherit → UI palette term; preset → looked
 * up; custom → embedded.
 *
 * Background is ALWAYS overridden to the UI palette's --bg so the
 * terminal visually merges with the surrounding chrome regardless of
 * which ANSI scheme the user picks. (Without this, a Solarized Light
 * scheme inside a dark UI shell looks like two apps glued together.)
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
  return { ...term, background: ui.ui.bg };
}

export function termPaletteRef(): TermPaletteRef { return _termRef; }
export function listTermPresets() { return TERM_PRESETS; }

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
  const [palette, shape, density, termRaw] = await Promise.all([
    invoke<string | null>("get_setting", { key: SETTING_KEY_PALETTE }).catch(() => null),
    invoke<string | null>("get_setting", { key: SETTING_KEY_SHAPE   }).catch(() => null),
    invoke<string | null>("get_setting", { key: SETTING_KEY_DENSITY }).catch(() => null),
    invoke<string | null>("get_setting", { key: SETTING_KEY_TERM    }).catch(() => null),
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
  apply(paletteById(_paletteId));
  applyShape(_shapeId);
  applyDensity(_densityId);
}
