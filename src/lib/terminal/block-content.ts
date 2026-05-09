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

/** 折叠仓库的最小接口，只用 getFold 查 saved body。
 *  避免直接 import FoldStore，让本模块对 folds.ts 的依赖只剩"读"语义。 */
export interface FoldLookup {
  getFold(blockId: number): { savedLines: unknown[] } | undefined;
}

/** 一个块的可视行号范围。end 缺失时 fallback 到 cursor 绝对行（块还在写）。 */
export interface BlockRange {
  id: number;
  startLine: number;
  endLine: number;
}

/** 把活动块的行号范围解析出来。已 disposed 的块跳过。
 *  注意：这个函数不感知折叠——折叠块返回的 endLine 仍是 cursorAbs（fallback），
 *  会包含远超块本身的内容。**复制路径请用 resolveBlockLines 代替**，它感知折叠。
 *  本函数保留为 BlockRect 渲染兜底（视图层只关心可见行）。 */
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

/** 块 → 逻辑行序列（IBufferLine 数组）。**复制管线用这条**。
 *  - 未折叠：从 buffer 取 [start..end] 行（end 缺失时 fallback 到 cursorAbs）
 *  - 已折叠：prompt 行还在 buffer，body 在 fold.savedLines；拼回完整内容
 *
 *  这是修 #5 (folded 块复制拉到 cursorAbs) 的关键——不感知折叠的话，
 *  fold 把 block.end disposed 后，旧路径会 fallback 到 cursorAbs，把
 *  之后所有命令的输出全收进来。 */
export function resolveBlockLines(
  term: Terminal,
  block: CommandBlock,
  foldStore?: FoldLookup,
): IBufferLine[] {
  if (block.start.isDisposed) return [];
  const buf = term.buffer.active;
  const fold = foldStore?.getFold(block.id);
  if (fold) {
    const prompt = buf.getLine(block.start.line);
    const body = fold.savedLines as unknown as IBufferLine[];
    return prompt ? [prompt, ...body] : [...body];
  }
  const cursorAbs = buf.baseY + buf.cursorY;
  const endLine = block.end && !block.end.isDisposed ? block.end.line : cursorAbs;
  if (endLine < block.start.line) return [];
  const out: IBufferLine[] = [];
  for (let y = block.start.line; y <= endLine; y++) {
    const l = buf.getLine(y);
    if (l) out.push(l);
  }
  return out;
}

/** 抽取若干块的纯文本。块间一个 `\n` 分隔，零装饰。
 *  传 foldStore 让折叠块也能正确复制内容（否则会被拉到 cursorAbs）。 */
export function extractBlocksText(
  term: Terminal,
  blocks: ReadonlyArray<CommandBlock>,
  foldStore?: FoldLookup,
): string {
  const sorted = [...blocks]
    .filter((b) => !b.start.isDisposed)
    .sort((a, b) => a.id - b.id);
  if (sorted.length === 0) return "";
  const parts: string[] = [];
  for (const block of sorted) {
    const lines = resolveBlockLines(term, block, foldStore);
    parts.push(linesToLogicalText(lines).join("\n"));
  }
  return parts.join("\n");
}

/** IBufferLine 数组 → 逻辑行字符串数组。处理软换行合并、CJK、行末 trim。 */
export function linesToLogicalText(lines: ReadonlyArray<IBufferLine>): string[] {
  const result: string[] = [];
  for (const line of lines) {
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

/** 行号范围 → 逻辑行数组。保留为 buf-shaped 输入的便利封装（测试用）。 */
export function extractRangeLines(
  buf: { getLine(i: number): IBufferLine | undefined },
  startLine: number,
  endLine: number,
): string[] {
  const lines: IBufferLine[] = [];
  for (let y = startLine; y <= endLine; y++) {
    const line = buf.getLine(y);
    if (line) lines.push(line);
  }
  return linesToLogicalText(lines);
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
