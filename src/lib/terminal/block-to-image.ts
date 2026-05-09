/**
 * Block to image rendering.
 *
 * 离屏 canvas 重渲染选中块到 PNG Blob，供"复制为图片"用。
 *
 * 为什么自己渲染：xterm v5 用 canvas/webgl 渲染，没有"按行号区域导出"的 API。
 * 直接抓 xterm 内部 canvas 涉及私有 API + 滚动偏移坑；自己渲染干净、可控、
 * 可加 padding / 重新设计 bar 视觉，且对 renderer 类型透明。
 *
 * 处理：
 *   - fg/bg 颜色：default / ANSI 16 / 256 调色板 / 24-bit RGB
 *   - inverse：在数据层 swap fg/bg，渲染层零分支（消除特殊情况）
 *   - bold / italic / underline
 *   - CJK 宽字符（width=2 cell 占两列，width=0 continuation 跳过）
 *   - DPR：高 DPI 屏幕清晰
 *   - 字体加载：await document.fonts.ready，否则首次 measureText 错位
 *
 * 不处理：dim / blink / strikethrough / overline / 自定义 underlineStyle
 *   （投入产出比低，绝大多数命令输出用不到）。
 */
import type { Terminal, ITheme, IBufferCell } from "@xterm/xterm";
import type { CommandBlock } from "./command-blocks";
import { resolveBlockLines, type FoldLookup } from "./block-content";

export interface RenderOptions {
  /** 彩色竖线宽度 px。默认 4。 */
  barWidth?: number;
  /** 竖线和文字之间的间距 px。默认 10。 */
  gutter?: number;
  /** 整体外边距 px。默认 14。 */
  outerPad?: number;
}

export interface ImageCell {
  ch: string;
  /** 1 = 普通字符，2 = CJK 宽字符 */
  width: 1 | 2;
  /** CSS 颜色字符串（已经处理过 inverse 的 swap） */
  fg: string;
  bg: string;
  bold: boolean;
  italic: boolean;
  underline: boolean;
}

export interface ImageRow {
  blockId: number;
  blockColor: string;
  cells: ImageCell[];
}

/* ───────────────────────── 入口 ───────────────────────── */

/** 渲染选中块到 PNG Blob。空集合 / 渲染失败 / 非 DOM 环境 → null。
 *  传 foldStore 让折叠块走 saved body 路径——否则会拉到 cursorAbs。 */
export async function renderBlocksToBlob(
  term: Terminal,
  blocks: ReadonlyArray<CommandBlock>,
  opts: RenderOptions = {},
  foldStore?: FoldLookup,
): Promise<Blob | null> {
  if (blocks.length === 0) return null;
  // 非 DOM 环境（vitest node / SSR）→ null。否则下方 createElement 会 throw，
  // 跟函数契约（"失败返回 null"）不符。
  if (typeof document === "undefined") return null;
  // 字体未 ready 时 measureText 会拿 fallback 字宽，整图错位。必等。
  if (document.fonts?.ready) {
    await document.fonts.ready;
  }
  const rows = extractImageRows(term, blocks, foldStore);
  if (rows.length === 0) return null;
  return renderRowsToBlob(rows, term, opts);
}

/* ───────────────────────── 数据抽取（纯函数，可测） ───────────────────────── */

/** 选中块 → 视觉行序列。视觉行不合并软换行（图片是"截图"，要保留视觉布局）。
 *  传 foldStore 让折叠块用 saved body 而非 buffer cursorAbs。 */
export function extractImageRows(
  term: Terminal,
  blocks: ReadonlyArray<CommandBlock>,
  foldStore?: FoldLookup,
): ImageRow[] {
  const sorted = [...blocks]
    .filter((b) => !b.start.isDisposed)
    .sort((a, b) => a.id - b.id);
  if (sorted.length === 0) return [];
  const theme: ITheme = term.options.theme ?? {};
  const out: ImageRow[] = [];
  for (const block of sorted) {
    const lines = resolveBlockLines(term, block, foldStore);
    for (const line of lines) {
      const cells: ImageCell[] = [];
      for (let x = 0; x < line.length; x++) {
        const cell = line.getCell(x);
        if (!cell) continue;
        if (cell.getWidth() === 0) continue;
        cells.push(cellToImageCell(cell, theme));
      }
      // 行尾 trim：连续的"空格 + 默认背景"剪掉，让图变窄
      while (cells.length > 0) {
        const last = cells[cells.length - 1];
        if (last.ch === " " && last.bg === defaultBg(theme)) cells.pop();
        else break;
      }
      out.push({ blockId: block.id, blockColor: block.color, cells });
    }
  }
  return out;
}

function cellToImageCell(cell: IBufferCell, theme: ITheme): ImageCell {
  let fg = resolveFg(cell, theme);
  let bg = resolveBg(cell, theme);
  // inverse 在数据层处理：swap fg/bg，渲染层不再判断
  if (cell.isInverse()) {
    const t = fg;
    fg = bg;
    bg = t;
  }
  return {
    ch: cell.getChars() || " ",
    width: cell.getWidth() === 2 ? 2 : 1,
    fg,
    bg,
    bold: !!cell.isBold(),
    italic: !!cell.isItalic(),
    underline: !!cell.isUnderline(),
  };
}

/* ───────────────────────── 颜色解析（纯函数，可测） ───────────────────────── */

export function resolveFg(cell: IBufferCell, theme: ITheme): string {
  if (cell.isFgDefault()) return defaultFg(theme);
  if (cell.isFgRGB()) return rgbFromInt(cell.getFgColor());
  if (cell.isFgPalette()) return paletteToColor(cell.getFgColor(), theme, defaultFg(theme));
  return defaultFg(theme);
}

export function resolveBg(cell: IBufferCell, theme: ITheme): string {
  if (cell.isBgDefault()) return defaultBg(theme);
  if (cell.isBgRGB()) return rgbFromInt(cell.getBgColor());
  if (cell.isBgPalette()) return paletteToColor(cell.getBgColor(), theme, defaultBg(theme));
  return defaultBg(theme);
}

function defaultFg(theme: ITheme): string {
  return theme.foreground ?? "#ffffff";
}
function defaultBg(theme: ITheme): string {
  return theme.background ?? "#000000";
}

/** xterm getFgColor() 在 RGB 模式下返回 24-bit 整数。 */
function rgbFromInt(n: number): string {
  const r = (n >> 16) & 0xff;
  const g = (n >> 8) & 0xff;
  const b = n & 0xff;
  return `rgb(${r},${g},${b})`;
}

/** 256 色调色板：0-15 走 theme，16-231 是 6×6×6 立方体，232-255 是灰阶。
 *  fallback 用于越界 idx——前景调用方传 defaultFg，背景传 defaultBg，
 *  不能在这里硬编码方向（之前 bg 越界时错跑成 defaultFg 是个 bug）。 */
export function paletteToColor(idx: number, theme: ITheme, fallback: string): string {
  if (idx < 0) return fallback;
  if (idx < 16) return ansi16(idx, theme);
  if (idx < 232) return ansi256Cube(idx);
  if (idx < 256) return ansi256Gray(idx);
  return fallback;
}

function ansi16(idx: number, theme: ITheme): string {
  const slots: (string | undefined)[] = [
    theme.black, theme.red, theme.green, theme.yellow,
    theme.blue, theme.magenta, theme.cyan, theme.white,
    theme.brightBlack, theme.brightRed, theme.brightGreen, theme.brightYellow,
    theme.brightBlue, theme.brightMagenta, theme.brightCyan, theme.brightWhite,
  ];
  return slots[idx] ?? "#888888";
}

function ansi256Cube(idx: number): string {
  const i = idx - 16;
  const r = (i / 36) | 0;
  const g = ((i / 6) | 0) % 6;
  const b = i % 6;
  const conv = (x: number) => (x === 0 ? 0 : 55 + x * 40);
  return `rgb(${conv(r)},${conv(g)},${conv(b)})`;
}

function ansi256Gray(idx: number): string {
  const v = 8 + (idx - 232) * 10;
  return `rgb(${v},${v},${v})`;
}

/* ───────────────────────── Canvas 渲染 ───────────────────────── */

interface BlockGroup {
  blockId: number;
  color: string;
  startRow: number;
  rowCount: number;
}

function groupRows(rows: ImageRow[]): BlockGroup[] {
  const out: BlockGroup[] = [];
  let cur: BlockGroup | null = null;
  rows.forEach((r, i) => {
    if (!cur || cur.blockId !== r.blockId) {
      cur = { blockId: r.blockId, color: r.blockColor, startRow: i, rowCount: 1 };
      out.push(cur);
    } else {
      cur.rowCount++;
    }
  });
  return out;
}

async function renderRowsToBlob(
  rows: ImageRow[],
  term: Terminal,
  opts: RenderOptions,
): Promise<Blob | null> {
  const fontSize = term.options.fontSize ?? 13;
  const fontFamily = term.options.fontFamily ?? "monospace";
  const lineHeightMul = term.options.lineHeight ?? 1.0;
  // 1.3 是经验值——xterm 自身行距偏紧，截图里稍宽一点视觉更舒服
  const lineHeight = Math.round(fontSize * Math.max(lineHeightMul, 1.0) * 1.3);
  const theme: ITheme = term.options.theme ?? {};
  const bgColor = defaultBg(theme);

  const outerPad = opts.outerPad ?? 14;
  const barWidth = opts.barWidth ?? 4;
  const gutter = opts.gutter ?? 10;

  // 测量 cell 宽度（'M' 在等宽字体下是标准基准）
  const measureCanvas = document.createElement("canvas");
  const mctx = measureCanvas.getContext("2d");
  if (!mctx) return null;
  mctx.font = `${fontSize}px ${fontFamily}`;
  const cellWidth = Math.max(1, Math.ceil(mctx.measureText("M").width));

  // 最长行字符数（宽字符算 2）
  let maxCells = 0;
  for (const r of rows) {
    let w = 0;
    for (const c of r.cells) w += c.width;
    if (w > maxCells) maxCells = w;
  }
  if (maxCells === 0) maxCells = 1;

  const groups = groupRows(rows);

  const canvasW = outerPad * 2 + barWidth + gutter + maxCells * cellWidth;
  const canvasH = outerPad * 2 + rows.length * lineHeight;

  const dpr = (typeof window !== "undefined" && window.devicePixelRatio) || 1;
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(canvasW * dpr);
  canvas.height = Math.ceil(canvasH * dpr);
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.scale(dpr, dpr);

  // 整体背景
  ctx.fillStyle = bgColor;
  ctx.fillRect(0, 0, canvasW, canvasH);

  ctx.textBaseline = "top";

  // bar 紧贴文本：覆盖块文本 N 行的精确 Y 范围，无上下 pad、块间无 gap。
  // 不同色 bar 直接相邻——颜色已经是分隔，多余空白只是噪音。
  const textStartX = outerPad + barWidth + gutter;
  for (const grp of groups) {
    const barX = outerPad;
    const barY = outerPad + grp.startRow * lineHeight;
    const barH = grp.rowCount * lineHeight;
    ctx.fillStyle = grp.color;
    roundRect(ctx, barX, barY, barWidth, barH, barWidth / 2);
    ctx.fill();

    for (let i = 0; i < grp.rowCount; i++) {
      const row = rows[grp.startRow + i];
      const rowY = outerPad + (grp.startRow + i) * lineHeight;
      drawRow(ctx, row, rowY, lineHeight, cellWidth, fontSize, fontFamily, textStartX, bgColor);
    }
  }

  return new Promise((resolve) => {
    canvas.toBlob((blob) => resolve(blob), "image/png");
  });
}

function drawRow(
  ctx: CanvasRenderingContext2D,
  row: ImageRow,
  y: number,
  lineHeight: number,
  cellWidth: number,
  fontSize: number,
  fontFamily: string,
  startX: number,
  defaultBgColor: string,
) {
  // Pass 1: 背景 — 合并连续同色 run，跳过 default bg（已被整图底色填过）
  let x = startX;
  let runStart = x;
  let runColor: string | null = null;
  const flush = (endX: number) => {
    if (runColor !== null && runColor !== defaultBgColor) {
      ctx.fillStyle = runColor;
      ctx.fillRect(runStart, y, endX - runStart, lineHeight);
    }
  };
  for (const c of row.cells) {
    const cw = cellWidth * c.width;
    if (runColor === null) {
      runColor = c.bg;
      runStart = x;
    } else if (c.bg !== runColor) {
      flush(x);
      runColor = c.bg;
      runStart = x;
    }
    x += cw;
  }
  flush(x);

  // Pass 2: 字符 — 字体每个 cell 都可能不同（bold/italic）
  x = startX;
  // 文字垂直居中：baseline=top 时 y 偏移 = (lineHeight - fontSize) / 2
  const textY = y + Math.max(0, (lineHeight - fontSize) / 2);
  for (const c of row.cells) {
    const cw = cellWidth * c.width;
    if (c.ch !== " " && c.ch.trim() !== "") {
      const weight = c.bold ? "bold" : "normal";
      const style = c.italic ? "italic" : "normal";
      ctx.font = `${style} ${weight} ${fontSize}px ${fontFamily}`;
      ctx.fillStyle = c.fg;
      ctx.fillText(c.ch, x, textY);
    }
    x += cw;
  }

  // Pass 3: 下划线
  x = startX;
  for (const c of row.cells) {
    const cw = cellWidth * c.width;
    if (c.underline) {
      ctx.fillStyle = c.fg;
      ctx.fillRect(x, y + lineHeight - 2, cw, 1);
    }
    x += cw;
  }
}

function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number, r: number,
) {
  const rad = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + rad, y);
  ctx.lineTo(x + w - rad, y);
  ctx.quadraticCurveTo(x + w, y, x + w, y + rad);
  ctx.lineTo(x + w, y + h - rad);
  ctx.quadraticCurveTo(x + w, y + h, x + w - rad, y + h);
  ctx.lineTo(x + rad, y + h);
  ctx.quadraticCurveTo(x, y + h, x, y + h - rad);
  ctx.lineTo(x, y + rad);
  ctx.quadraticCurveTo(x, y, x + rad, y);
  ctx.closePath();
}
