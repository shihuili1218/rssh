/**
 * Terminal palette presets — applied to xterm.js independently of the
 * UI palette. Users can also paste a custom xterm.js theme JSON.
 *
 * Source: https://github.com/mbadolato/iTerm2-Color-Schemes (xterm.js folder)
 * — that repo is the canonical place to find more presets in xterm.js JSON
 * format. Drop one in here or paste it via the "Custom" UI.
 *
 * The 8 presets below cover the schemes most commonly requested by
 * terminal users. We don't aim to be exhaustive — exhaustiveness lives
 * in iTerm2-Color-Schemes upstream; we ship a curated default set.
 */

import type { PaletteTerm } from "./palettes.ts";

export interface TermPalettePreset {
  id: string;
  label: string;
  term: PaletteTerm;
}

/* ────────────────────────────────────────────────────────────
   Built-in presets
   ──────────────────────────────────────────────────────────── */

const DRACULA: TermPalettePreset = {
  id: "dracula",
  label: "Dracula",
  term: {
    background: "#282A36", foreground: "#F8F8F2", cursor: "#BD93F9",
    selectionBackground: "rgba(189,147,249,0.3)",
    black: "#21222C", white: "#F8F8F2",
    red: "#FF5555", green: "#50FA7B", yellow: "#F1FA8C",
    blue: "#BD93F9", magenta: "#FF79C6", cyan: "#8BE9FD",
    brightBlack: "#6272A4", brightWhite: "#FFFFFF",
    brightRed: "#FF6E6E", brightGreen: "#69FF94", brightYellow: "#FFFFA5",
    brightBlue: "#D6ACFF", brightMagenta: "#FF92DF", brightCyan: "#A4FFFF",
  },
};

const SOLARIZED_DARK: TermPalettePreset = {
  id: "solarized-dark",
  label: "Solarized Dark",
  term: {
    background: "#002B36", foreground: "#839496", cursor: "#93A1A1",
    selectionBackground: "rgba(38,139,210,0.3)",
    black: "#073642", white: "#EEE8D5",
    red: "#DC322F", green: "#859900", yellow: "#B58900",
    blue: "#268BD2", magenta: "#D33682", cyan: "#2AA198",
    brightBlack: "#586E75", brightWhite: "#FDF6E3",
    brightRed: "#CB4B16", brightGreen: "#586E75", brightYellow: "#657B83",
    brightBlue: "#839496", brightMagenta: "#6C71C4", brightCyan: "#93A1A1",
  },
};

const SOLARIZED_LIGHT: TermPalettePreset = {
  id: "solarized-light",
  label: "Solarized Light",
  term: {
    background: "#FDF6E3", foreground: "#657B83", cursor: "#586E75",
    selectionBackground: "rgba(38,139,210,0.25)",
    black: "#073642", white: "#EEE8D5",
    red: "#DC322F", green: "#859900", yellow: "#B58900",
    blue: "#268BD2", magenta: "#D33682", cyan: "#2AA198",
    brightBlack: "#002B36", brightWhite: "#FDF6E3",
    brightRed: "#CB4B16", brightGreen: "#586E75", brightYellow: "#657B83",
    brightBlue: "#839496", brightMagenta: "#6C71C4", brightCyan: "#93A1A1",
  },
};

const GRUVBOX_DARK: TermPalettePreset = {
  id: "gruvbox-dark",
  label: "Gruvbox Dark",
  term: {
    background: "#282828", foreground: "#EBDBB2", cursor: "#EBDBB2",
    selectionBackground: "rgba(168,153,132,0.3)",
    black: "#282828", white: "#A89984",
    red: "#CC241D", green: "#98971A", yellow: "#D79921",
    blue: "#458588", magenta: "#B16286", cyan: "#689D6A",
    brightBlack: "#928374", brightWhite: "#EBDBB2",
    brightRed: "#FB4934", brightGreen: "#B8BB26", brightYellow: "#FABD2F",
    brightBlue: "#83A598", brightMagenta: "#D3869B", brightCyan: "#8EC07C",
  },
};

const NORD: TermPalettePreset = {
  id: "nord",
  label: "Nord",
  term: {
    background: "#2E3440", foreground: "#D8DEE9", cursor: "#D8DEE9",
    selectionBackground: "rgba(67,76,94,0.6)",
    black: "#3B4252", white: "#E5E9F0",
    red: "#BF616A", green: "#A3BE8C", yellow: "#EBCB8B",
    blue: "#81A1C1", magenta: "#B48EAD", cyan: "#88C0D0",
    brightBlack: "#4C566A", brightWhite: "#ECEFF4",
    brightRed: "#BF616A", brightGreen: "#A3BE8C", brightYellow: "#EBCB8B",
    brightBlue: "#81A1C1", brightMagenta: "#B48EAD", brightCyan: "#8FBCBB",
  },
};

const TOKYO_NIGHT: TermPalettePreset = {
  id: "tokyo-night",
  label: "Tokyo Night",
  term: {
    background: "#1A1B26", foreground: "#A9B1D6", cursor: "#C0CAF5",
    selectionBackground: "rgba(40,52,87,0.7)",
    black: "#15161E", white: "#A9B1D6",
    red: "#F7768E", green: "#9ECE6A", yellow: "#E0AF68",
    blue: "#7AA2F7", magenta: "#BB9AF7", cyan: "#7DCFFF",
    brightBlack: "#414868", brightWhite: "#C0CAF5",
    brightRed: "#F7768E", brightGreen: "#9ECE6A", brightYellow: "#E0AF68",
    brightBlue: "#7AA2F7", brightMagenta: "#BB9AF7", brightCyan: "#7DCFFF",
  },
};

const ONE_DARK: TermPalettePreset = {
  id: "one-dark",
  label: "One Dark",
  term: {
    background: "#282C34", foreground: "#ABB2BF", cursor: "#528BFF",
    selectionBackground: "rgba(63,68,82,0.7)",
    black: "#282C34", white: "#ABB2BF",
    red: "#E06C75", green: "#98C379", yellow: "#E5C07B",
    blue: "#61AFEF", magenta: "#C678DD", cyan: "#56B6C2",
    brightBlack: "#5C6370", brightWhite: "#FFFFFF",
    brightRed: "#E06C75", brightGreen: "#98C379", brightYellow: "#E5C07B",
    brightBlue: "#61AFEF", brightMagenta: "#C678DD", brightCyan: "#56B6C2",
  },
};

const MONOKAI: TermPalettePreset = {
  id: "monokai",
  label: "Monokai",
  term: {
    background: "#272822", foreground: "#F8F8F2", cursor: "#F8F8F0",
    selectionBackground: "rgba(73,72,62,0.7)",
    black: "#272822", white: "#F8F8F2",
    red: "#F92672", green: "#A6E22E", yellow: "#F4BF75",
    blue: "#66D9EF", magenta: "#AE81FF", cyan: "#A1EFE4",
    brightBlack: "#75715E", brightWhite: "#F9F8F5",
    brightRed: "#F92672", brightGreen: "#A6E22E", brightYellow: "#F4BF75",
    brightBlue: "#66D9EF", brightMagenta: "#AE81FF", brightCyan: "#A1EFE4",
  },
};

export const TERM_PRESETS: readonly TermPalettePreset[] = [
  DRACULA,
  SOLARIZED_DARK,
  SOLARIZED_LIGHT,
  GRUVBOX_DARK,
  NORD,
  TOKYO_NIGHT,
  ONE_DARK,
  MONOKAI,
];

export function termPresetById(id: string): TermPalettePreset | undefined {
  return TERM_PRESETS.find((p) => p.id === id);
}

/* ────────────────────────────────────────────────────────────
   Selection ref — the persisted user choice
   ──────────────────────────────────────────────────────────── */

export type TermPaletteRef =
  | { kind: "inherit" }
  | { kind: "preset"; id: string }
  | { kind: "custom"; term: PaletteTerm };

export const DEFAULT_TERM_REF: TermPaletteRef = { kind: "inherit" };

export function isTermPaletteRef(v: unknown): v is TermPaletteRef {
  if (!v || typeof v !== "object") return false;
  const r = v as Record<string, unknown>;
  if (r.kind === "inherit") return true;
  if (r.kind === "preset" && typeof r.id === "string") return true;
  if (r.kind === "custom" && r.term && typeof r.term === "object") {
    // Minimal sanity: require background + foreground at least.
    const t = r.term as Record<string, unknown>;
    return typeof t.background === "string" && typeof t.foreground === "string";
  }
  return false;
}

/**
 * Parse a JSON string the user pasted into the "Custom" textarea.
 * Returns the validated term object or throws with a user-readable error.
 *
 * Accepts the xterm.js ITheme JSON shape directly — that's what the
 * iTerm2-Color-Schemes/xterm folder exports. Required keys: background,
 * foreground. ANSI 16 are recommended; missing ones fall back to xterm
 * defaults at render time, so we don't reject on those — and PaletteTerm
 * marks them optional. We validate background/foreground above and drop
 * any non-string fields below, so the runtime shape conforms to PaletteTerm;
 * the `unknown` hop in the final cast is purely a TS overlap-rule workaround
 * (TS can't see "we've cleaned this Record" without it), not a force cast.
 */
export function parseCustomTermJson(raw: string): PaletteTerm {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (e: any) {
    throw new Error(`Invalid JSON: ${e.message}`);
  }
  if (!parsed || typeof parsed !== "object") {
    throw new Error("Expected a JSON object");
  }
  const obj = parsed as Record<string, unknown>;
  if (typeof obj.background !== "string" || typeof obj.foreground !== "string") {
    throw new Error("JSON must include 'background' and 'foreground' (hex strings)");
  }
  // Normalise common field aliases so users can paste JSON from the
  // iTerm2-Color-Schemes/windowsterminal folder directly:
  //   cursorColor      → cursor                (Windows Terminal naming)
  //   purple/brightPurple → magenta/brightMagenta (WT vs xterm.js)
  //   selection        → selectionBackground   (legacy xterm.js)
  const term = { ...obj } as Record<string, unknown>;
  const aliases: Record<string, string> = {
    cursorColor: "cursor",
    purple: "magenta",
    brightPurple: "brightMagenta",
    selection: "selectionBackground",
  };
  for (const [from, to] of Object.entries(aliases)) {
    if (typeof term[from] === "string" && term[to] === undefined) {
      term[to] = term[from];
    }
    delete term[from];
  }
  // Drop any keys with non-string values to keep the shape clean.
  for (const k of Object.keys(term)) {
    if (typeof term[k] !== "string") delete term[k];
  }
  // 经过上面两轮清洗（alias 重命名 + 非字符串字段删除）+ 顶部 background/foreground 校验，
  // `term` 此刻只含 string 字段且 background/foreground 必存。`Record<string, unknown>`
  // 与 `PaletteTerm` 在 TS 看来没足够 overlap（前者完全开放、后者有具名 optional 字段），
  // 所以需要先绕一道 `unknown` —— 不是 hack，是显式承认"运行期校验过、类型层无法表达"。
  return term as unknown as PaletteTerm;
}
