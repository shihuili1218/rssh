/**
 * Palette presets — single source of truth for both UI tokens
 * (written to :root CSS variables) and xterm theme (passed via
 * `terminal.options.theme`).
 *
 * Adding a preset:
 *   1. Pick or define 16 ANSI colors + foreground/background.
 *   2. Map UI roles (accent/error/success/...) to colors that
 *      look good against the chosen surfaces.
 *   3. Append to PALETTES below.
 *
 * iTerm2-Color-Schemes is a good reference for the ANSI 16:
 *   https://github.com/mbadolato/iTerm2-Color-Schemes
 */

export type PaletteId =
  | "dark-neumorphism"
  | "light-soft"
  | "dracula"
  | "solarized-dark"
  | "solarized-light"
  | "tomorrow-night";

/** UI tokens — mapped 1:1 onto :root CSS variables. */
export interface PaletteUi {
  bg: string;
  surface: string;
  shadowDark: string;
  shadowLight: string;
  divider: string;
  text: string;
  textSub: string;
  textDim: string;
  accent: string;
  error: string;
  success: string;
  warning: string;
  magenta: string;
  purple: string;
}

/** xterm theme — passed to xterm.options.theme.
 *
 *  Only background/foreground are strictly required; everything else
 *  is optional because (a) xterm.js ITheme is itself partial-tolerant
 *  and (b) custom palettes pasted by users may omit ANSI keys. Built-in
 *  presets always supply the full set. */
export interface PaletteTerm {
  background: string;
  foreground: string;
  cursor?: string;
  selectionBackground?: string;
  black?: string;
  white?: string;
  red?: string;
  green?: string;
  yellow?: string;
  blue?: string;
  magenta?: string;
  cyan?: string;
  brightBlack?: string;
  brightWhite?: string;
  brightRed?: string;
  brightGreen?: string;
  brightYellow?: string;
  brightBlue?: string;
  brightMagenta?: string;
  brightCyan?: string;
}

export interface Palette {
  id: PaletteId;
  label: string;
  /** Tonal mode hint. Available for shape implementations that need
   *  to key off light/dark, though current shapes derive everything
   *  from the palette tokens directly. */
  mode: "dark" | "light";
  ui: PaletteUi;
  term: PaletteTerm;
}

/* ────────────────────────────────────────────────────────────
   Presets
   ──────────────────────────────────────────────────────────── */

const DARK_NEUMORPHISM: Palette = {
  id: "dark-neumorphism",
  label: "Dark Neumorphism",
  mode: "dark",
  ui: {
    bg:          "#2B2D3A",
    surface:     "#32343F",
    shadowDark:  "#1E2028",
    shadowLight: "#383B4A",
    divider:     "#3C3F50",
    text:        "#E0E5EC",
    textSub:     "#A0A8BB",
    textDim:     "#6B7A99",
    accent:      "#4A6CF7",
    error:       "#E05555",
    success:     "#4CB88A",
    warning:     "#DDAA33",
    magenta:     "#9B72E4",
    purple:      "#A855F7",
  },
  term: {
    background: "#2B2D3A", foreground: "#E0E5EC", cursor: "#4A6CF7",
    selectionBackground: "rgba(74,108,247,0.3)",
    black: "#1E2028", white: "#E0E5EC",
    red: "#E05555", green: "#4CB88A", yellow: "#DDAA33",
    blue: "#4A6CF7", magenta: "#9B72E4", cyan: "#2898AC",
    brightBlack: "#6B7A99", brightWhite: "#FFFFFF",
    brightRed: "#FF6B6B", brightGreen: "#6EDAA0", brightYellow: "#FFD060",
    brightBlue: "#6B8FF8", brightMagenta: "#B894F6", brightCyan: "#40C8E0",
  },
};

const LIGHT_SOFT: Palette = {
  id: "light-soft",
  label: "Light Soft",
  mode: "light",
  ui: {
    bg:          "#ECEFF4",
    surface:     "#E0E4EC",
    shadowDark:  "#C5CAD3",
    shadowLight: "#FFFFFF",
    divider:     "#D2D7DF",
    text:        "#2E3440",
    textSub:     "#4C566A",
    textDim:     "#7A8493",
    accent:      "#5267E0",
    error:       "#C0392B",
    success:     "#3CA875",
    warning:     "#B58900",
    magenta:     "#7E57C2",
    purple:      "#8E44AD",
  },
  term: {
    background: "#ECEFF4", foreground: "#2E3440", cursor: "#5267E0",
    selectionBackground: "rgba(82,103,224,0.25)",
    // Light palette: ANSI white must read as "light" against a light bg,
    // not be aliased to the same dark color as black/foreground.
    black: "#2E3440", white: "#D2D7DF",
    red: "#C0392B", green: "#3CA875", yellow: "#B58900",
    blue: "#5267E0", magenta: "#7E57C2", cyan: "#0590A0",
    brightBlack: "#7A8493", brightWhite: "#FFFFFF",
    brightRed: "#D14B3C", brightGreen: "#4FBE8E", brightYellow: "#D4A017",
    brightBlue: "#6A82F5", brightMagenta: "#9968D6", brightCyan: "#1FA8B8",
  },
};

const DRACULA: Palette = {
  id: "dracula",
  label: "Dracula",
  mode: "dark",
  ui: {
    bg:          "#282A36",
    surface:     "#343746",
    shadowDark:  "#1A1B23",
    shadowLight: "#3D4154",
    divider:     "#44475A",
    text:        "#F8F8F2",
    textSub:     "#BFBFB0",
    textDim:     "#6272A4",
    accent:      "#BD93F9",
    error:       "#FF5555",
    success:     "#50FA7B",
    warning:     "#F1FA8C",
    magenta:     "#FF79C6",
    purple:      "#BD93F9",
  },
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

const SOLARIZED_DARK: Palette = {
  id: "solarized-dark",
  label: "Solarized Dark",
  mode: "dark",
  ui: {
    bg:          "#002B36",
    surface:     "#073642",
    shadowDark:  "#001E26",
    shadowLight: "#0E4651",
    divider:     "#0E4651",
    text:        "#EEE8D5",
    textSub:     "#93A1A1",
    textDim:     "#586E75",
    accent:      "#268BD2",
    error:       "#DC322F",
    success:     "#859900",
    warning:     "#B58900",
    magenta:     "#D33682",
    purple:      "#6C71C4",
  },
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

const SOLARIZED_LIGHT: Palette = {
  id: "solarized-light",
  label: "Solarized Light",
  mode: "light",
  ui: {
    bg:          "#FDF6E3",
    surface:     "#EEE8D5",
    shadowDark:  "#D8D2BD",
    shadowLight: "#FFFFFF",
    divider:     "#D8D2BD",
    text:        "#073642",
    textSub:     "#586E75",
    textDim:     "#93A1A1",
    accent:      "#268BD2",
    error:       "#DC322F",
    success:     "#859900",
    warning:     "#B58900",
    magenta:     "#D33682",
    purple:      "#6C71C4",
  },
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

const TOMORROW_NIGHT: Palette = {
  id: "tomorrow-night",
  label: "Tomorrow Night",
  mode: "dark",
  ui: {
    bg:          "#1D1F21",
    surface:     "#282A2E",
    shadowDark:  "#101113",
    shadowLight: "#373B41",
    divider:     "#373B41",
    text:        "#C5C8C6",
    textSub:     "#969896",
    textDim:     "#707880",
    accent:      "#81A2BE",
    error:       "#CC6666",
    success:     "#B5BD68",
    warning:     "#F0C674",
    magenta:     "#B294BB",
    purple:      "#B294BB",
  },
  term: {
    background: "#1D1F21", foreground: "#C5C8C6", cursor: "#C5C8C6",
    selectionBackground: "rgba(129,162,190,0.3)",
    black: "#1D1F21", white: "#C5C8C6",
    red: "#CC6666", green: "#B5BD68", yellow: "#F0C674",
    blue: "#81A2BE", magenta: "#B294BB", cyan: "#8ABEB7",
    brightBlack: "#707880", brightWhite: "#FFFFFF",
    brightRed: "#D54E53", brightGreen: "#B9CA4A", brightYellow: "#E7C547",
    brightBlue: "#7AA6DA", brightMagenta: "#C397D8", brightCyan: "#70C0B1",
  },
};

export const PALETTES: readonly Palette[] = [
  DARK_NEUMORPHISM,
  LIGHT_SOFT,
  DRACULA,
  SOLARIZED_DARK,
  SOLARIZED_LIGHT,
  TOMORROW_NIGHT,
];

export const DEFAULT_PALETTE_ID: PaletteId = "dark-neumorphism";

export function paletteById(id: string): Palette {
  return PALETTES.find((p) => p.id === id) ?? DARK_NEUMORPHISM;
}
