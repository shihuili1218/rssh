import { describe, it, expect } from "vitest";
import {
  paletteToColor,
  resolveFg,
  resolveBg,
  extractImageRows,
} from "./block-to-image.ts";
import type { ITheme, IBufferCell } from "@xterm/xterm";
import type { CommandBlock } from "./command-blocks.ts";

const THEME: ITheme = {
  foreground: "#eeeeee",
  background: "#111111",
  black: "#000000",
  red: "#cc0000",
  green: "#00cc00",
  yellow: "#cccc00",
  blue: "#0000cc",
  magenta: "#cc00cc",
  cyan: "#00cccc",
  white: "#cccccc",
  brightBlack: "#666666",
  brightRed: "#ff5555",
  brightGreen: "#55ff55",
  brightYellow: "#ffff55",
  brightBlue: "#5555ff",
  brightMagenta: "#ff55ff",
  brightCyan: "#55ffff",
  brightWhite: "#ffffff",
};

/* ───────────────────────── paletteToColor ───────────────────────── */

describe("paletteToColor", () => {
  const FALLBACK = "#fallback";
  it("maps 0..7 to ANSI 8 base colors via theme", () => {
    expect(paletteToColor(0, THEME, FALLBACK)).toBe("#000000");
    expect(paletteToColor(1, THEME, FALLBACK)).toBe("#cc0000");
    expect(paletteToColor(7, THEME, FALLBACK)).toBe("#cccccc");
  });

  it("maps 8..15 to bright ANSI via theme", () => {
    expect(paletteToColor(8, THEME, FALLBACK)).toBe("#666666");
    expect(paletteToColor(15, THEME, FALLBACK)).toBe("#ffffff");
  });

  it("maps 16 (cube origin) to pure black", () => {
    expect(paletteToColor(16, THEME, FALLBACK)).toBe("rgb(0,0,0)");
  });

  it("maps 231 (cube max) to pure white-ish (5,5,5 → 255 each)", () => {
    expect(paletteToColor(231, THEME, FALLBACK)).toBe("rgb(255,255,255)");
  });

  it("cube formula: idx 196 → red corner (5,0,0)", () => {
    expect(paletteToColor(196, THEME, FALLBACK)).toBe("rgb(255,0,0)");
  });

  it("maps grayscale 232..255", () => {
    expect(paletteToColor(232, THEME, FALLBACK)).toBe("rgb(8,8,8)");
    expect(paletteToColor(255, THEME, FALLBACK)).toBe("rgb(238,238,238)");
  });

  it("returns caller-supplied fallback for out-of-range indices", () => {
    expect(paletteToColor(-1, THEME, FALLBACK)).toBe(FALLBACK);
    expect(paletteToColor(999, THEME, FALLBACK)).toBe(FALLBACK);
  });
});

/* ───────────────────────── resolveFg / resolveBg ───────────────────────── */

function makeCell(opts: Partial<{
  fgDefault: boolean;
  fgRGB: boolean;
  fgPalette: boolean;
  fgColor: number;
  bgDefault: boolean;
  bgRGB: boolean;
  bgPalette: boolean;
  bgColor: number;
}>): IBufferCell {
  const fgDefault = opts.fgDefault ?? (!opts.fgRGB && !opts.fgPalette);
  const bgDefault = opts.bgDefault ?? (!opts.bgRGB && !opts.bgPalette);
  return {
    isFgDefault: () => (fgDefault ? 1 : 0),
    isFgRGB: () => (opts.fgRGB ? 1 : 0),
    isFgPalette: () => (opts.fgPalette ? 1 : 0),
    getFgColor: () => opts.fgColor ?? 0,
    isBgDefault: () => (bgDefault ? 1 : 0),
    isBgRGB: () => (opts.bgRGB ? 1 : 0),
    isBgPalette: () => (opts.bgPalette ? 1 : 0),
    getBgColor: () => opts.bgColor ?? 0,
    isBold: () => 0,
    isItalic: () => 0,
    isUnderline: () => 0,
    isInverse: () => 0,
    isDim: () => 0,
    isBlink: () => 0,
    isStrikethrough: () => 0,
    isInvisible: () => 0,
    isOverline: () => 0,
    getChars: () => "x",
    getCode: () => "x".charCodeAt(0),
    getWidth: () => 1,
    isAttributeDefault: () => 1,
    isFgPaletteHigh: () => 0,
    isBgPaletteHigh: () => 0,
    getFgColorMode: () => 0,
    getBgColorMode: () => 0,
  } as unknown as IBufferCell;
}

describe("resolveFg / resolveBg", () => {
  it("default fg returns theme.foreground", () => {
    expect(resolveFg(makeCell({ fgDefault: true }), THEME)).toBe("#eeeeee");
  });

  it("default bg returns theme.background", () => {
    expect(resolveBg(makeCell({ bgDefault: true }), THEME)).toBe("#111111");
  });

  it("RGB fg unpacks 24-bit int to rgb()", () => {
    // 0xff0000 = pure red
    expect(resolveFg(makeCell({ fgRGB: true, fgColor: 0xff0000 }), THEME))
      .toBe("rgb(255,0,0)");
  });

  it("palette fg routes to ansi16 / cube / gray", () => {
    expect(resolveFg(makeCell({ fgPalette: true, fgColor: 1 }), THEME))
      .toBe("#cc0000");
    expect(resolveFg(makeCell({ fgPalette: true, fgColor: 196 }), THEME))
      .toBe("rgb(255,0,0)");
  });
});

/* ───────────────────────── extractImageRows ───────────────────────── */

interface SpecCell {
  ch: string;
  width: 1 | 2;
  fg?: string;
  bg?: string;
  inverse?: boolean;
  bold?: boolean;
  underline?: boolean;
}

function specToCell(spec: SpecCell): IBufferCell {
  const fgRGB = spec.fg !== undefined;
  const bgRGB = spec.bg !== undefined;
  return {
    isFgDefault: () => (fgRGB ? 0 : 1),
    isFgRGB: () => (fgRGB ? 1 : 0),
    isFgPalette: () => 0,
    getFgColor: () => spec.fg ? rgbStringToInt(spec.fg) : 0,
    isBgDefault: () => (bgRGB ? 0 : 1),
    isBgRGB: () => (bgRGB ? 1 : 0),
    isBgPalette: () => 0,
    getBgColor: () => spec.bg ? rgbStringToInt(spec.bg) : 0,
    isBold: () => (spec.bold ? 1 : 0),
    isItalic: () => 0,
    isUnderline: () => (spec.underline ? 1 : 0),
    isInverse: () => (spec.inverse ? 1 : 0),
    isDim: () => 0,
    isBlink: () => 0,
    isStrikethrough: () => 0,
    isInvisible: () => 0,
    isOverline: () => 0,
    getChars: () => spec.ch,
    getCode: () => spec.ch.charCodeAt(0) || 0,
    getWidth: () => spec.width,
  } as unknown as IBufferCell;
}

function rgbStringToInt(rgb: string): number {
  const m = rgb.match(/^#([0-9a-f]{6})$/i);
  if (m) return parseInt(m[1], 16);
  return 0;
}

function fakeLine(cells: SpecCell[]) {
  // 模拟 CJK：width=2 后跟一个 width=0
  const expanded: (IBufferCell | undefined)[] = [];
  for (const c of cells) {
    expanded.push(specToCell(c));
    if (c.width === 2) {
      expanded.push(specToCell({ ch: "", width: 1 }) /* placeholder */);
      // 替换 width 为 0
      const last = expanded[expanded.length - 1] as any;
      const orig = last.getWidth;
      last.getWidth = () => 0;
    }
  }
  return {
    length: expanded.length,
    isWrapped: false,
    getCell: (x: number) => expanded[x],
  };
}

function fakeTerm(lines: ReturnType<typeof fakeLine>[]): any {
  return {
    options: { theme: THEME },
    buffer: {
      active: {
        baseY: 0,
        cursorY: lines.length - 1,
        getLine: (i: number) => lines[i],
      },
    },
  };
}

function fakeBlock(id: number, color: string, startLine: number, endLine: number | null): CommandBlock {
  return {
    id,
    color,
    start: { line: startLine, isDisposed: false } as any,
    end: endLine === null ? null : ({ line: endLine, isDisposed: false } as any),
  };
}

describe("extractImageRows", () => {
  it("trims trailing default-bg spaces from row cells", () => {
    const term = fakeTerm([
      fakeLine([
        { ch: "h", width: 1 }, { ch: "i", width: 1 },
        { ch: " ", width: 1 }, { ch: " ", width: 1 }, { ch: " ", width: 1 },
      ]),
    ]);
    const rows = extractImageRows(term, [fakeBlock(1, "#abc", 0, 0)]);
    expect(rows).toHaveLength(1);
    expect(rows[0].cells.map((c) => c.ch)).toEqual(["h", "i"]);
  });

  it("preserves trailing spaces if their bg is non-default", () => {
    const term = fakeTerm([
      fakeLine([
        { ch: "x", width: 1 },
        { ch: " ", width: 1, bg: "#ff0000" }, // 红底空格 — 视觉上有意义
      ]),
    ]);
    const rows = extractImageRows(term, [fakeBlock(1, "#abc", 0, 0)]);
    expect(rows[0].cells).toHaveLength(2);
    expect(rows[0].cells[1].ch).toBe(" ");
  });

  it("skips width=0 continuation cells", () => {
    const term = fakeTerm([
      fakeLine([{ ch: "你", width: 2 }, { ch: "好", width: 2 }]),
    ]);
    const rows = extractImageRows(term, [fakeBlock(1, "#abc", 0, 0)]);
    expect(rows[0].cells.map((c) => c.ch)).toEqual(["你", "好"]);
    expect(rows[0].cells.map((c) => c.width)).toEqual([2, 2]);
  });

  it("inverse swaps fg/bg in the data layer", () => {
    const term = fakeTerm([
      fakeLine([{ ch: "x", width: 1, fg: "#ff0000", bg: "#0000ff", inverse: true }]),
    ]);
    const rows = extractImageRows(term, [fakeBlock(1, "#abc", 0, 0)]);
    const cell = rows[0].cells[0];
    // swap：fg 变 bg(#0000ff -> rgb)，bg 变 fg(#ff0000 -> rgb)
    expect(cell.fg).toBe("rgb(0,0,255)");
    expect(cell.bg).toBe("rgb(255,0,0)");
  });

  it("preserves bold / underline flags", () => {
    const term = fakeTerm([
      fakeLine([{ ch: "x", width: 1, bold: true, underline: true }]),
    ]);
    const rows = extractImageRows(term, [fakeBlock(1, "#abc", 0, 0)]);
    expect(rows[0].cells[0].bold).toBe(true);
    expect(rows[0].cells[0].underline).toBe(true);
  });

  it("stitches multi-block rows in id-ascending order with block colors", () => {
    const term = fakeTerm([
      fakeLine([{ ch: "a", width: 1 }]),
      fakeLine([{ ch: "b", width: 1 }]),
      fakeLine([{ ch: "c", width: 1 }]),
    ]);
    const rows = extractImageRows(term, [
      fakeBlock(2, "#222", 1, 1),
      fakeBlock(1, "#111", 0, 0),
      fakeBlock(3, "#333", 2, 2),
    ]);
    expect(rows.map((r) => r.blockId)).toEqual([1, 2, 3]);
    expect(rows.map((r) => r.blockColor)).toEqual(["#111", "#222", "#333"]);
  });
});
