/**
 * Block content extraction — pure functions over xterm.js Buffer.
 *
 * 把命令块的可视内容抽成纯文本，供"复制为文本"和"复制为图片"两条
 * 路径共用基础数据。
 *
 * 核心规则：
 *  1. 多块按 `block.id` 升序输出（时间顺序）
 *  2. 块内按行号 `[start.line .. end.line]` 顺序遍历
 *  3. 软换行（line.isWrapped === true）合并为同一逻辑行——对 shell paste
 *     友好，可重复执行
 *  4. CJK 宽字符：width=2 cell 持有字符，width=0 continuation 跳过
 *  5. 空 cell 视为空格（终端右侧 padding）；逻辑行末尾 trimEnd 去掉
 *  6. ANSI 颜色/属性不进入文本输出（纯文本，shell 粘贴友好）
 */
import type { Terminal, IBufferLine } from "@xterm/xterm";
import type { CommandBlock } from "./command-blocks";

/** 一个块的可视行号范围。end 缺失时 fallback 到 cursor 绝对行（块还在写）。 */
export interface BlockRange {
  id: number;
  startLine: number;
  endLine: number;
}

/** 把活动块的行号范围解析出来。已 disposed 的块跳过。 */
export function resolveBlockRanges(
  term: Terminal,
  blocks: ReadonlyArray<CommandBlock>,
): BlockRange[] {
  const buf = term.buffer.active;
  const cursorAbs = buf.baseY + buf.cursorY;
  const out: BlockRange[] = [];
  for (const b of blocks) {
    if (b.start.isDisposed) continue;
    const endLine =
      b.end && !b.end.isDisposed ? b.end.line : cursorAbs;
    if (endLine < b.start.line) continue;
    out.push({ id: b.id, startLine: b.start.line, endLine });
  }
  out.sort((a, b) => a.id - b.id);
  return out;
}

/** 抽取若干块的纯文本。块间一个 `\n` 分隔，零装饰。 */
export function extractBlocksText(
  term: Terminal,
  blocks: ReadonlyArray<CommandBlock>,
): string {
  const ranges = resolveBlockRanges(term, blocks);
  if (ranges.length === 0) return "";
  const buf = term.buffer.active;
  const parts: string[] = [];
  for (const r of ranges) {
    const lines = extractRangeLines(buf, r.startLine, r.endLine);
    parts.push(lines.join("\n"));
  }
  return parts.join("\n");
}

/** 行号范围 → 逻辑行数组。处理软换行合并、CJK 宽字符、行末空格修剪。 */
export function extractRangeLines(
  buf: { getLine(i: number): IBufferLine | undefined },
  startLine: number,
  endLine: number,
): string[] {
  const result: string[] = [];
  for (let y = startLine; y <= endLine; y++) {
    const line = buf.getLine(y);
    if (!line) continue;
    const raw = extractLineRaw(line);
    if (line.isWrapped && result.length > 0) {
      // 软换行：拼到上一逻辑行尾，**不**插换行符
      result[result.length - 1] += raw;
    } else {
      result.push(raw);
    }
  }
  // 只 trimEnd 逻辑行（不是视觉行），保住中间软换行边界的真实空格
  return result.map((l) => l.trimEnd());
}

/** 单行 cell → 字符串。宽字符 continuation 跳过，空 cell 补空格。 */
function extractLineRaw(line: IBufferLine): string {
  let s = "";
  for (let x = 0; x < line.length; x++) {
    const cell = line.getCell(x);
    if (!cell) continue;
    if (cell.getWidth() === 0) continue;
    const ch = cell.getChars();
    s += ch || " ";
  }
  return s;
}
