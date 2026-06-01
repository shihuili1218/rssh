/**
 * Terminal font: the user picks one installed family; it is prepended to a
 * fixed base stack. The base stack provides Nerd Font icons, CJK (PingFang)
 * and emoji coverage, so a chosen font that lacks those glyphs still renders
 * them via CSS per-glyph fallback — picking a font never breaks powerline
 * prompts or CJK output.
 *
 * The stored value is just the chosen family name (empty = use the base stack
 * as-is = the historical default). Persistence/state lives in store.svelte.ts;
 * the picker's source list comes from the Rust `list_fonts` command.
 */

/** One installed font family + whether it is monospaced (from `list_fonts`). */
export type FontInfo = { family: string; monospaced: boolean };

/**
 * The base fallback chain — Nerd Fonts first (powerline glyphs), then system
 * monospace, then CJK + emoji, ending in the generic `monospace`. This is the
 * historical hardcoded TerminalPane value; kept byte-identical so that the
 * default (no font chosen) renders exactly as before.
 */
export const BASE_FONT_STACK =
  "'JetBrainsMono Nerd Font', 'FiraCode Nerd Font', 'Hack Nerd Font', 'MesloLGS NF', 'Symbols Nerd Font Mono', Menlo, Monaco, 'Apple Color Emoji', 'Apple Symbols', 'PingFang SC', 'Courier New', monospace";

/**
 * Compose the xterm `fontFamily` string: the chosen family, quoted and
 * prepended to BASE_FONT_STACK. A blank choice yields the base stack
 * unchanged. The result always ends with BASE_FONT_STACK, so glyph coverage
 * (icons / CJK / emoji) is never lost regardless of what the user picks.
 */
export function composeTermFontStack(chosen: string): string {
  const name = chosen.trim();
  return name ? `"${name}", ${BASE_FONT_STACK}` : BASE_FONT_STACK;
}
