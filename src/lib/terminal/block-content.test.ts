import { describe, it, expect } from "vitest";
import {
  extractBlocksText,
  extractRangeLines,
  resolveBlockLines,
  resolveBlockRanges,
} from "./block-content.ts";
import type { CommandBlock } from "./command-blocks.ts";

/* ─────────────────────────────────────────────────────────────
 * Fake xterm Terminal/Buffer/Line/Cell — 最小子集，够测：
 *  - 行 = "字符串 + 是否 wrapped"
 *  - 字符串里特殊语法 "{w2}X" 表示宽字符（getWidth=2 + 下一个 cell width=0）
 * ───────────────────────────────────────────────────────────── */

interface FakeCell {
  ch: string;
  width: 0 | 1 | 2;
}

function lineFromSpec(spec: string, wrapped = false) {
  const cells: FakeCell[] = [];
  let i = 0;
  while (i < spec.length) {
    if (spec.startsWith("{w2}", i)) {
      const ch = spec[i + 4];
      cells.push({ ch, width: 2 });
      cells.push({ ch: "", width: 0 });
      i += 5;
    } else {
      cells.push({ ch: spec[i], width: 1 });
      i++;
    }
  }
  return {
    length: cells.length,
    isWrapped: wrapped,
    getCell(x: number) {
      const c = cells[x];
      if (!c) return undefined;
      return {
        getChars: () => c.ch,
        getWidth: () => c.width,
      };
    },
  };
}

function fakeBuf(lines: ReturnType<typeof lineFromSpec>[]) {
  return {
    baseY: 0,
    cursorY: lines.length - 1,
    getLine(i: number) {
      return lines[i];
    },
  };
}

function fakeTerm(lines: ReturnType<typeof lineFromSpec>[]) {
  return {
    buffer: { active: fakeBuf(lines) },
  } as any;
}

function fakeMarker(line: number, disposed = false) {
  return { line, isDisposed: disposed };
}

function fakeBlock(id: number, startLine: number, endLine: number | null): CommandBlock {
  return {
    id,
    color: "",
    start: fakeMarker(startLine) as any,
    end: endLine === null ? null : (fakeMarker(endLine) as any),
  };
}

/* ───────────────────────── extractRangeLines ───────────────────────── */

describe("extractRangeLines", () => {
  it("trims trailing spaces per logical line", () => {
    const buf = fakeBuf([
      lineFromSpec("hello world          "),
      lineFromSpec("foo                  "),
    ]);
    expect(extractRangeLines(buf as any, 0, 1)).toEqual(["hello world", "foo"]);
  });

  it("merges wrapped continuation rows into one logical line", () => {
    const buf = fakeBuf([
      lineFromSpec("aaa"),
      lineFromSpec("bbb", true), // wrapped continuation of previous
      lineFromSpec("ccc"),
    ]);
    expect(extractRangeLines(buf as any, 0, 2)).toEqual(["aaabbb", "ccc"]);
  });

  it("preserves spaces at wrap boundary inside a logical line", () => {
    // 第 1 行末有 padding 空格，但因为下一行是 wrapped 续行，padding 是真实的
    // 终端列填充——合并后 trimEnd 只对最末逻辑行生效。
    const buf = fakeBuf([
      lineFromSpec("a   "),
      lineFromSpec("b   ", true),
    ]);
    expect(extractRangeLines(buf as any, 0, 1)).toEqual(["a   b"]);
  });

  it("skips width=0 continuation cells of CJK wide chars", () => {
    // "你好" 两个宽字符 → 4 个 cell：[w2,你][w0,""][w2,好][w0,""]
    const buf = fakeBuf([lineFromSpec("{w2}你{w2}好     ")]);
    expect(extractRangeLines(buf as any, 0, 0)).toEqual(["你好"]);
  });

  it("treats empty cells as spaces inside content", () => {
    const buf = fakeBuf([lineFromSpec("a b   c        ")]);
    expect(extractRangeLines(buf as any, 0, 0)).toEqual(["a b   c"]);
  });

  it("returns [] for missing lines", () => {
    const buf = fakeBuf([]);
    expect(extractRangeLines(buf as any, 0, 5)).toEqual([]);
  });
});

/* ───────────────────────── resolveBlockRanges ───────────────────────── */

describe("resolveBlockRanges", () => {
  it("uses end.line when end exists and not disposed", () => {
    const term = fakeTerm([
      lineFromSpec("a"),
      lineFromSpec("b"),
      lineFromSpec("c"),
    ]);
    const ranges = resolveBlockRanges(term, [fakeBlock(1, 0, 2)]);
    expect(ranges).toEqual([{ id: 1, startLine: 0, endLine: 2 }]);
  });

  it("falls back to cursor abs when end is null (block still growing)", () => {
    const term = fakeTerm([
      lineFromSpec("a"),
      lineFromSpec("b"),
      lineFromSpec("c"),
    ]);
    // cursorY = lines.length - 1 = 2，baseY = 0 → cursorAbs = 2
    const ranges = resolveBlockRanges(term, [fakeBlock(7, 0, null)]);
    expect(ranges).toEqual([{ id: 7, startLine: 0, endLine: 2 }]);
  });

  it("skips blocks whose start marker is disposed", () => {
    const term = fakeTerm([lineFromSpec("a")]);
    const block: CommandBlock = {
      id: 1,
      color: "",
      start: { line: 0, isDisposed: true } as any,
      end: null,
    };
    expect(resolveBlockRanges(term, [block])).toEqual([]);
  });

  it("falls back when end is disposed", () => {
    const term = fakeTerm([lineFromSpec("a"), lineFromSpec("b")]);
    const block: CommandBlock = {
      id: 1,
      color: "",
      start: fakeMarker(0) as any,
      end: { line: 99, isDisposed: true } as any,
    };
    // disposed end → 用 cursorAbs (1)
    expect(resolveBlockRanges(term, [block])).toEqual([
      { id: 1, startLine: 0, endLine: 1 },
    ]);
  });

  it("skips ranges where endLine < startLine", () => {
    const term = fakeTerm([lineFromSpec("a")]);
    // baseY=0, cursorY=0 → cursorAbs=0；start.line=5 → endLine(0) < startLine(5)
    expect(resolveBlockRanges(term, [fakeBlock(1, 5, null)])).toEqual([]);
  });

  it("returns ranges sorted by id ascending (time order)", () => {
    const term = fakeTerm([
      lineFromSpec("a"),
      lineFromSpec("b"),
      lineFromSpec("c"),
    ]);
    const ranges = resolveBlockRanges(term, [
      fakeBlock(3, 2, 2),
      fakeBlock(1, 0, 0),
      fakeBlock(2, 1, 1),
    ]);
    expect(ranges.map((r) => r.id)).toEqual([1, 2, 3]);
  });
});

/* ───────────────────────── extractBlocksText ───────────────────────── */

describe("extractBlocksText", () => {
  it("returns empty string for empty input", () => {
    const term = fakeTerm([lineFromSpec("a")]);
    expect(extractBlocksText(term, [])).toBe("");
  });

  it("single block — outputs its trimmed lines joined by \\n", () => {
    const term = fakeTerm([
      lineFromSpec("$ ls           "),
      lineFromSpec("a.txt b.txt    "),
      lineFromSpec("$              "),
    ]);
    expect(extractBlocksText(term, [fakeBlock(1, 0, 1)])).toBe("$ ls\na.txt b.txt");
  });

  it("multi block — id-ascending, single \\n between blocks, no decoration", () => {
    const term = fakeTerm([
      lineFromSpec("$ pwd     "),
      lineFromSpec("/tmp      "),
      lineFromSpec("$ whoami  "),
      lineFromSpec("linus     "),
    ]);
    const text = extractBlocksText(term, [
      fakeBlock(2, 2, 3), // 故意倒序传入 — 函数内部应排序
      fakeBlock(1, 0, 1),
    ]);
    expect(text).toBe("$ pwd\n/tmp\n$ whoami\nlinus");
  });

  it("merges wrapped output within a block", () => {
    const term = fakeTerm([
      lineFromSpec("$ echo aaaa    "),
      lineFromSpec("aaaaaa"),
      lineFromSpec("bbbbbb", true), // wrapped
    ]);
    expect(extractBlocksText(term, [fakeBlock(1, 0, 2)])).toBe("$ echo aaaa\naaaaaabbbbbb");
  });

  it("preserves CJK wide chars across blocks", () => {
    const term = fakeTerm([
      lineFromSpec("$ echo {w2}你{w2}好     "),
      lineFromSpec("{w2}你{w2}好           "),
    ]);
    expect(extractBlocksText(term, [fakeBlock(1, 0, 1)])).toBe("$ echo 你好\n你好");
  });
});

/* ───────────────────────── resolveBlockLines + folded path ───────────────────────── */

describe("resolveBlockLines (folded blocks)", () => {
  it("without foldStore: returns lines from buffer [start..end]", () => {
    const term = fakeTerm([
      lineFromSpec("$ ls"),
      lineFromSpec("a.txt"),
      lineFromSpec("b.txt"),
    ]);
    const lines = resolveBlockLines(term as any, fakeBlock(1, 0, 2));
    expect(lines).toHaveLength(3);
  });

  it("with foldStore: folded block returns prompt + savedLines", () => {
    // Folded 状态：fold() disposed 了 block.end，buffer 里只剩 prompt 行
    // 没 foldStore 时旧路径会 fallback 到 cursorAbs 把后续命令也卷进来 (#5 bug)
    const promptLine = lineFromSpec("$ npm install   ");
    const savedBody1 = lineFromSpec("added 234 packages");
    const savedBody2 = lineFromSpec("done in 3.2s");
    const term = fakeTerm([
      promptLine,
      lineFromSpec("$ ls"),                // 后续命令——不该被卷进来
      lineFromSpec("a.txt"),
    ]);
    // folded block: end disposed, body 在 fold.savedLines
    const block: CommandBlock = {
      id: 7,
      color: "#abc",
      start: { line: 0, isDisposed: false } as any,
      end: { line: 99, isDisposed: true } as any, // disposed by fold()
    };
    const foldStore = {
      getFold: (id: number) =>
        id === 7 ? { savedLines: [savedBody1, savedBody2] } : undefined,
    };
    const lines = resolveBlockLines(term as any, block, foldStore);
    expect(lines).toHaveLength(3); // prompt + 2 saved body lines
    expect(lines[0]).toBe(promptLine);
    expect(lines[1]).toBe(savedBody1);
    expect(lines[2]).toBe(savedBody2);
  });

  it("with foldStore but block not folded: falls back to buffer range", () => {
    const term = fakeTerm([
      lineFromSpec("$ ls"),
      lineFromSpec("a.txt"),
    ]);
    const foldStore = { getFold: () => undefined };
    const lines = resolveBlockLines(term as any, fakeBlock(1, 0, 1), foldStore);
    expect(lines).toHaveLength(2);
  });
});

describe("extractBlocksText with foldStore", () => {
  it("folded block: copies prompt + saved body, NOT cursorAbs fallback", () => {
    // 这是 PR #24 reviewer 报的真 bug：折叠块复制会拉到 cursorAbs，
    // 把后续命令的输出全卷进来。foldStore-aware 修了这个
    const term = fakeTerm([
      lineFromSpec("$ npm install     "),
      lineFromSpec("$ ls              "),     // 后续命令，不该出现在复制结果
      lineFromSpec("a.txt b.txt       "),
    ]);
    const block: CommandBlock = {
      id: 1,
      color: "#abc",
      start: { line: 0, isDisposed: false } as any,
      end: { line: 99, isDisposed: true } as any,
    };
    const foldStore = {
      getFold: (id: number) =>
        id === 1
          ? {
              savedLines: [
                lineFromSpec("added 234 packages"),
                lineFromSpec("done in 3.2s"),
              ],
            }
          : undefined,
    };
    const text = extractBlocksText(term as any, [block], foldStore);
    expect(text).toBe("$ npm install\nadded 234 packages\ndone in 3.2s");
    // 没卷进 "$ ls" 也没卷进 "a.txt b.txt"
    expect(text).not.toContain("ls");
    expect(text).not.toContain("a.txt");
  });
});
