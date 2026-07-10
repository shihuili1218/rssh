import { describe, it, expect } from "vitest";
import { createFoldStore } from "./folds.ts";
import type { CommandBlock, CommandBlockTracker } from "./command-blocks.ts";

/* ─────────────────────────────────────────────────────────────
 * Fake xterm.js Terminal —— 复刻 folds.ts 用到的全部私有/公有 API。
 * 关键忠实点：
 *   - buf.lines.splice 触发 marker 自动迁移（行号修正 + 范围内 dispose）
 *     这部分 xterm 自己写在 Buffer.addMarker 里；我们这里照抄行为，
 *     才能验证 folds.ts 不重复维护 marker
 *   - 同时维护 ybase/ydisp/y 的语义不变量
 * ───────────────────────────────────────────────────────────── */

interface FakeMarker {
  id: number;
  line: number;
  isDisposed: boolean;
  onDispose(fn: () => void): { dispose: () => void };
  dispose(): void;
}

interface FakeLine {
  content: string;
  isWrapped?: boolean;
  getTrimmedLength?: () => number;
}

function fakeBlankLine(): FakeLine {
  const line: FakeLine = { content: "<blank>", isWrapped: false };
  line.getTrimmedLength = () => line.content === "<blank>" ? 0 : line.content.length;
  return line;
}

function fakeTerm(opts: { rows: number; initialLines: number; cursorY: number; ybase?: number; maxLength?: number }) {
  const rows = opts.rows;
  const maxLength = opts.maxLength;
  let ybase = opts.ybase ?? 0;
  let ydisp = ybase;
  let y = opts.cursorY;
  const lineArray: FakeLine[] = [];
  for (let i = 0; i < opts.initialLines; i++) lineArray.push({ content: `L${i}` });

  let markerSeq = 0;
  const markers: FakeMarker[] = [];

  const makeMarker = (line: number): FakeMarker => {
    const onDisposeFns: Array<() => void> = [];
    const m: FakeMarker = {
      id: ++markerSeq,
      line,
      isDisposed: false,
      onDispose(fn) {
        onDisposeFns.push(fn);
        return { dispose: () => {} };
      },
      dispose() {
        if (this.isDisposed) return;
        this.isDisposed = true;
        this.line = -1;
        onDisposeFns.forEach((f) => f());
      },
    };
    markers.push(m);
    return m;
  };

  function trimHead(count: number) {
    if (count <= 0) return;
    lineArray.splice(0, count);
    for (const m of markers) {
      if (m.isDisposed) continue;
      m.line -= count;
      if (m.line < 0) m.dispose();
    }
  }

  function enforceMaxLength() {
    if (!maxLength || lineArray.length <= maxLength) return;
    trimHead(lineArray.length - maxLength);
  }

  // CircularList 的子集 + xterm Buffer.addMarker 内嵌的迁移逻辑
  const lines = {
    get length() {
      return lineArray.length;
    },
    get(i: number) {
      return lineArray[i];
    },
    splice(start: number, deleteCount: number, ...items: FakeLine[]) {
      // Delete 阶段
      if (deleteCount > 0) {
        const removed = lineArray.splice(start, deleteCount);
        for (const m of markers) {
          if (m.isDisposed) continue;
          if (m.line >= start && m.line < start + deleteCount) m.dispose();
          else if (m.line >= start + deleteCount) m.line -= deleteCount;
        }
        void removed;
      }
      // Insert 阶段
      if (items.length > 0) {
        lineArray.splice(start, 0, ...items);
        for (const m of markers) {
          if (m.isDisposed) continue;
          if (m.line >= start) m.line += items.length;
        }
        enforceMaxLength();
      }
    },
    push(item: FakeLine) {
      lineArray.push(item);
    },
    pop() {
      return lineArray.pop();
    },
  };

  const buffer = {
    lines,
    get ybase() {
      return ybase;
    },
    set ybase(v: number) {
      ybase = v;
    },
    get ydisp() {
      return ydisp;
    },
    set ydisp(v: number) {
      ydisp = v;
    },
    get y() {
      return y;
    },
    set y(v: number) {
      y = v;
    },
    getBlankLine: (_attr: unknown) => fakeBlankLine(),
    addMarker: (line: number) => makeMarker(line),
  };

  const resizeListeners = new Set<() => void>();
  const cursorMoveListeners = new Set<() => void>();
  const lineFeedListeners = new Set<() => void>();
  let activeBufferType: "normal" | "alternate" = "normal";

  const term = {
    rows,
    buffer: {
      get active() {
        return { type: activeBufferType };
      },
    },
    refresh: (_a: number, _b: number) => {},
    onResize(fn: () => void) {
      resizeListeners.add(fn);
      return { dispose: () => resizeListeners.delete(fn) };
    },
    onCursorMove(fn: () => void) {
      cursorMoveListeners.add(fn);
      return { dispose: () => cursorMoveListeners.delete(fn) };
    },
    onLineFeed(fn: () => void) {
      lineFeedListeners.add(fn);
      return { dispose: () => lineFeedListeners.delete(fn) };
    },
    _core: { buffer },
  };

  return {
    term: term as unknown as Parameters<typeof createFoldStore>[0],
    buffer,
    lineContents: () => lineArray.map((l) => l.content),
    lineRefs: () => [...lineArray],
    markers,
    makeMarker,
    lineFeed: () => {
      y = Math.min(rows - 1, y + 1);
      lineFeedListeners.forEach((fn) => fn());
    },
    moveCursorTo: (nextY: number) => {
      y = nextY;
      cursorMoveListeners.forEach((fn) => fn());
    },
    fireCursorMove: () => cursorMoveListeners.forEach((fn) => fn()),
    setActiveBuffer: (type: "normal" | "alternate") => {
      activeBufferType = type;
    },
    triggerResize: () => resizeListeners.forEach((fn) => fn()),
    snapshot: () => ({
      length: lineArray.length,
      ybase,
      ydisp,
      y,
      cursorAbs: ybase + y,
    }),
  };
}

/** 极简 tracker 替身：blocks 数组直接暴露 + onChange 手动触发。 */
function fakeTracker(blocks: CommandBlock[]): CommandBlockTracker & { fire: () => void } {
  const listeners = new Set<() => void>();
  return {
    get blocks() {
      return blocks;
    },
    onChange(fn: () => void) {
      listeners.add(fn);
      return { dispose: () => listeners.delete(fn) };
    },
    dispose() {
      listeners.clear();
    },
    fire() {
      listeners.forEach((fn) => fn());
    },
  };
}

function makeBlock(
  id: number,
  start: FakeMarker,
  end: FakeMarker | null,
): CommandBlock {
  return { id, color: "hsl(0,0%,50%)", start: start as unknown as CommandBlock["start"], end: end as unknown as CommandBlock["end"] };
}

/* ─────────────────────────────────────────────────────────────
 * Tests
 * ───────────────────────────────────────────────────────────── */

describe("FoldStore — fold() validation", () => {
  it("refuses unknown blockId", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const tracker = fakeTracker([]);
    const store = createFoldStore(f.term, tracker);
    expect(store.fold(99)).toBe(false);
    store.dispose();
  });

  it("refuses block without end (open block)", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const start = f.makeMarker(0);
    const tracker = fakeTracker([makeBlock(1, start, null)]);
    const store = createFoldStore(f.term, tracker);
    expect(store.fold(1)).toBe(false);
    store.dispose();
  });

  it("refuses block with empty body (start+1 > end)", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(5);
    const e = f.makeMarker(5);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    expect(store.fold(1)).toBe(false);
    store.dispose();
  });

  it("refuses if fold range overlaps cursor", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 5 }); // cursor at abs 5
    const s = f.makeMarker(0);
    const e = f.makeMarker(10); // body=[1..10] includes 5
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    expect(store.fold(1)).toBe(false);
    store.dispose();
  });

  it("refuses double-fold of the same block", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    expect(store.fold(1)).toBe(true);
    expect(store.fold(1)).toBe(false);
    store.dispose();
  });
});

describe("FoldStore — fold() effects", () => {
  it("fold preserves lines.length (push blanks compensates splice)", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    const before = f.snapshot();
    store.fold(1);
    const after = f.snapshot();
    expect(after.length).toBe(before.length); // 不变量 1
    store.dispose();
  });

  it("fold preserves cursor's content position (cursorAbs -= count)", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    const before = f.snapshot();
    store.fold(1);
    const after = f.snapshot();
    // body=[1..12] = 12 行; cursor 绝对位置应 -12
    expect(after.cursorAbs).toBe(before.cursorAbs - 12);
    store.dispose();
  });

  it("fold disposes block.end (it's inside the splice range)", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    expect(e.isDisposed).toBe(true);
    expect(s.isDisposed).toBe(false); // start 在范围外存活
    store.dispose();
  });

  it("fold auto-migrates markers AFTER the splice range", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 20 });
    const s1 = f.makeMarker(0);
    const e1 = f.makeMarker(12);
    const s2 = f.makeMarker(13); // 紧邻 block 1 之后
    const e2 = f.makeMarker(15);
    const tracker = fakeTracker([
      makeBlock(1, s1, e1),
      makeBlock(2, s2, e2),
    ]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    expect(s2.line).toBe(1); // 13 - 12
    expect(e2.line).toBe(3); // 15 - 12
    store.dispose();
  });

  it("isFolded reflects state", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    expect(store.isFolded(1)).toBe(false);
    store.fold(1);
    expect(store.isFolded(1)).toBe(true);
    store.dispose();
  });
});

describe("FoldStore — unfold() effects", () => {
  it("unfold pops pushed blanks → buffer length restored to pre-fold (safe path)", () => {
    // 折叠期间无 xterm 写末尾、cursor 也没爬过去 → safe pop
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    const before = f.snapshot();
    store.fold(1);
    store.unfold(1);
    const after = f.snapshot();
    expect(after.length).toBe(before.length); // 复原
  });

  it("unfold with scrollback (ybase>0) restores length precisely (pushCount<count)", () => {
    // 关键场景：fold 时 ybase 有 scrollback，让 pushCount < count。
    // unfold 应严格还原长度——这是滚动条与内容同步的根本前提
    const f = fakeTerm({ rows: 24, initialLines: 38, cursorY: 23, ybase: 14 });
    const s = f.makeMarker(20);
    const e = f.makeMarker(30); // body=[21..30] = 10 行
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    const before = f.snapshot();
    store.fold(1);
    // ybaseDrain = min(14, 10) = 10。pushCount = 0。ybase: 14→4
    expect(f.snapshot().ybase).toBe(4);
    store.unfold(1);
    const after = f.snapshot();
    // 完全还原：linesLen, ybase, cursor 三个全回到 fold 前
    expect(after.length).toBe(before.length);
    expect(after.ybase).toBe(before.ybase);
    expect(after.cursorAbs).toBe(before.cursorAbs);
  });

  it("unfold removes still-blank compensation lines when later output is appended", () => {
    // ybase=0 → pushCount=count。后续输出追加在补偿空行之后时，
    // 仍可按引用删除那些还保持空白的补偿行，保留用户输出。
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1); // pushCount=12
    f.buffer.lines.push({ content: "<user-output>" } as never);
    const afterFold = f.snapshot();
    store.unfold(1);
    const afterUnfold = f.snapshot();
    expect(afterUnfold.length).toBe(afterFold.length);
    expect(f.lineContents()).toContain("<user-output>");
  });

  it("unfold keeps blank compensation lines the cursor has consumed", () => {
    const f = fakeTerm({ rows: 10, initialLines: 10, cursorY: 9 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(8);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    const consumed = store.getFold(1)!.pushedBlankRefs[0] as FakeLine;
    // The cursor has moved onto the first compensation blank. It still renders
    // blank, but it now represents real terminal output and must be preserved.
    f.buffer.y = 2;
    const afterFold = f.snapshot();
    store.unfold(1);
    const afterUnfold = f.snapshot();
    expect(afterUnfold.length).toBe(afterFold.length + 1);
    expect(f.lineRefs()).toContain(consumed);
  });

  it("unfold keeps consumed blank lines after the cursor moves back", () => {
    const f = fakeTerm({ rows: 10, initialLines: 10, cursorY: 9 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(8);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    const consumedBlank = store.getFold(1)!.pushedBlankRefs[0] as FakeLine;
    const outputLine = store.getFold(1)!.pushedBlankRefs[1] as FakeLine;

    // A legal output sequence can cross an empty line, write below it, then
    // move the cursor back up. The empty line is real output history now.
    f.lineFeed();
    f.lineFeed();
    outputLine.content = "<user-output>";
    f.moveCursorTo(1);

    const afterFold = f.snapshot();
    store.unfold(1);
    const refs = f.lineRefs();
    expect(f.snapshot().length).toBe(afterFold.length + 2);
    expect(refs.indexOf(outputLine)).toBe(refs.indexOf(consumedBlank) + 1);
  });

  it("unfold preserves the blank prefix before output written in one cursor batch", () => {
    const f = fakeTerm({ rows: 10, initialLines: 10, cursorY: 9 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(8);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    const first = store.getFold(1)!.pushedBlankRefs[0] as FakeLine;
    const second = store.getFold(1)!.pushedBlankRefs[1] as FakeLine;

    // xterm can parse "cursor down; write; cursor up" as one batch and emit no
    // intermediate cursor event. The modified second ref proves the first ref
    // was crossed and must remain even though it is still visually blank.
    second.content = "<batched-output>";
    f.moveCursorTo(1);

    store.unfold(1);

    const refs = f.lineRefs();
    expect(refs.indexOf(second)).toBe(refs.indexOf(first) + 1);
  });

  it("unfold keeps a compensation line that was replaced by user output", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(4);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1); // count=4, push 4 blanks
    // 把最末尾的 blank 替换成用户内容（模拟 xterm 在折叠期间往那写了东西）
    f.buffer.lines.pop();
    f.buffer.lines.push({ content: "<user-output>" } as never);
    const afterFold = f.snapshot();
    store.unfold(1);
    const afterUnfold = f.snapshot();
    expect(afterUnfold.length).toBe(afterFold.length + 1);
    expect(f.lineContents()).toContain("<user-output>");
  });

  it("unfold keeps the compensation prefix before reused user output", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(4);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1); // count=4, push 4 blanks
    const reused = store.getFold(1)!.pushedBlankRefs[3] as FakeLine;
    reused.content = "<user-output>";
    const afterFold = f.snapshot();
    store.unfold(1);
    const afterUnfold = f.snapshot();
    // Reaching the fourth compensation row makes the three rows above it part
    // of the output layout too. Removing them would move the output upward.
    expect(afterUnfold.length).toBe(afterFold.length + 4);
    expect(f.lineContents()).toContain("<user-output>");
  });

  it("unfold preserves cursor content position", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    const before = f.snapshot();
    store.fold(1);
    store.unfold(1);
    const after = f.snapshot();
    expect(after.cursorAbs).toBe(before.cursorAbs);
  });

  it("unfold re-registers block.end at correct line", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const block = makeBlock(1, s, e);
    const tracker = fakeTracker([block]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    expect(block.end?.isDisposed).toBe(true); // 折叠时被吞
    store.unfold(1);
    expect(block.end).not.toBeNull();
    expect(block.end?.isDisposed).toBe(false); // 新 marker
    expect(block.end?.line).toBe(12); // start.line(0) + count(12)
  });

  it("unfold returns false for unknown blockId", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const tracker = fakeTracker([]);
    const store = createFoldStore(f.term, tracker);
    expect(store.unfold(99)).toBe(false);
    store.dispose();
  });

  it("unfold drops fold record if block.start was disposed (scrollback trim)", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const block = makeBlock(1, s, e);
    const tracker = fakeTracker([block]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    // 模拟 scrollback 修剪：start marker 被 dispose
    s.dispose();
    expect(store.unfold(1)).toBe(false);
    expect(store.isFolded(1)).toBe(false);
  });

  it("unfold commits buffer coordinates if insert trimming disposes block.start", () => {
    const f = fakeTerm({ rows: 5, initialLines: 6, cursorY: 4, ybase: 1, maxLength: 6 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(2);
    const block = makeBlock(1, s, e);
    const tracker = fakeTracker([block]);
    const store = createFoldStore(f.term, tracker);

    store.fold(1);
    const compensation = [...store.getFold(1)!.pushedBlankRefs];
    const markerCountAfterFold = f.markers.length;

    expect(store.unfold(1)).toBe(false);
    expect(s.isDisposed).toBe(true);
    expect(store.isFolded(1)).toBe(false);
    expect(f.markers.length).toBe(markerCountAfterFold);
    expect(block.end?.isDisposed).toBe(true);
    expect(compensation.some((line) => f.lineRefs().includes(line as FakeLine))).toBe(false);
    expect(f.snapshot()).toEqual({
      length: 5,
      ybase: 0,
      ydisp: 0,
      y: 4,
      cursorAbs: 4,
    });
  });

  it("unfold pads the viewport after head trim removes more history than padding", () => {
    const f = fakeTerm({ rows: 5, initialLines: 6, cursorY: 4, ybase: 1, maxLength: 6 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(4);
    const block = makeBlock(1, s, e);
    const tracker = fakeTracker([block]);
    const store = createFoldStore(f.term, tracker);

    store.fold(1); // count=4, ybaseDrain=1, pushCount=3
    const compensation = [...store.getFold(1)!.pushedBlankRefs];

    expect(store.unfold(1)).toBe(false);
    expect(s.isDisposed).toBe(true);
    expect(compensation.some((line) => f.lineRefs().includes(line as FakeLine))).toBe(false);
    expect(f.lineContents()).toEqual(["L3", "L4", "L5", "<blank>", "<blank>"]);
    expect(f.snapshot()).toEqual({
      length: 5,
      ybase: 0,
      ydisp: 0,
      y: 2,
      cursorAbs: 2,
    });
  });

  it("keeps multi-chunk insertion ordered when CircularList trims after each chunk", () => {
    const f = fakeTerm({
      rows: 40_010,
      initialLines: 40_010,
      cursorY: 40_009,
      maxLength: 40_010,
    });
    const s = f.makeMarker(0);
    const e = f.makeMarker(33_000);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);

    expect(store.fold(1)).toBe(true);
    store.unfold(1);

    const survivingLineNumbers = f.lineContents()
      .filter((line) => /^L\d+$/.test(line))
      .map((line) => Number(line.slice(1)));
    expect(survivingLineNumbers).toEqual([...survivingLineNumbers].sort((a, b) => a - b));
  });
});

describe("FoldStore — multiple folds", () => {
  it("fold two distinct blocks, both tracked independently", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 19 });
    // Block 1: start=0, end=5 (5-line body)
    // Block 2: start=6, end=12 (6-line body)
    const s1 = f.makeMarker(0);
    const e1 = f.makeMarker(5);
    const s2 = f.makeMarker(6);
    const e2 = f.makeMarker(12);
    const tracker = fakeTracker([
      makeBlock(1, s1, e1),
      makeBlock(2, s2, e2),
    ]);
    const store = createFoldStore(f.term, tracker);
    expect(store.fold(1)).toBe(true);
    expect(store.fold(2)).toBe(true);
    expect(store.isFolded(1)).toBe(true);
    expect(store.isFolded(2)).toBe(true);
    expect(store.folds.length).toBe(2);
  });

  it("unfold preserves the OTHER fold's state", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 19 });
    const s1 = f.makeMarker(0);
    const e1 = f.makeMarker(5);
    const s2 = f.makeMarker(6);
    const e2 = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s1, e1), makeBlock(2, s2, e2)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    store.fold(2);
    store.unfold(2);
    expect(store.isFolded(1)).toBe(true);
    expect(store.isFolded(2)).toBe(false);
  });

  it("unfolds multiple folds in non-LIFO order without leaking compensation blanks", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 19 });
    const s1 = f.makeMarker(0);
    const e1 = f.makeMarker(5);
    const s2 = f.makeMarker(6);
    const e2 = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s1, e1), makeBlock(2, s2, e2)]);
    const store = createFoldStore(f.term, tracker);
    const before = f.snapshot();
    const beforeLines = f.lineContents();

    store.fold(1);
    store.fold(2);
    store.unfold(1);
    store.unfold(2);

    expect(f.snapshot()).toEqual(before);
    expect(f.lineContents()).toEqual(beforeLines);
  });
});

describe("FoldStore — auto-cleanup", () => {
  it("unfoldAll expands active folds before resize", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    expect(store.isFolded(1)).toBe(true);
    store.unfoldAll();
    expect(store.isFolded(1)).toBe(false);
  });

  it("binds folds to normal history while alternate buffer is active", () => {
    const normal = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const alternate = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = normal.makeMarker(0);
    const e = normal.makeMarker(4);
    const bodyRef = normal.lineRefs()[1];
    const alternateBefore = alternate.lineRefs();
    const core = (normal.term as unknown as {
      _core: { buffer: typeof normal.buffer; buffers?: { normal: typeof normal.buffer } };
    })._core;
    core.buffer = alternate.buffer;
    core.buffers = { normal: normal.buffer };

    const store = createFoldStore(normal.term, fakeTracker([makeBlock(1, s, e)]));
    expect(store.fold(1)).toBe(true);

    expect(normal.lineRefs()).not.toContain(bodyRef);
    expect(alternate.lineRefs()).toEqual(alternateBefore);
  });

  it("ignores alternate-buffer cursor events when tracking normal compensation rows", () => {
    const f = fakeTerm({ rows: 10, initialLines: 10, cursorY: 9 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(8);
    const store = createFoldStore(f.term, fakeTracker([makeBlock(1, s, e)]));
    const before = f.snapshot();

    expect(store.fold(1)).toBe(true);
    // Leave the normal cursor over a compensation blank, then switch to an
    // alternate-screen application. Its cursor events must not consume normal
    // history that the application never touched.
    f.buffer.y = 2;
    f.setActiveBuffer("alternate");
    f.fireCursorMove();
    // The normal cursor is still dormant at its original content line when
    // we restore; only the alternate event above could have consumed a blank.
    f.buffer.y = 1;
    store.unfold(1);

    expect(f.snapshot().length).toBe(before.length);
  });

  it("tracker drops a folded block → fold record auto-dropped", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const blocks = [makeBlock(1, s, e)];
    const tracker = fakeTracker(blocks);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    // 模拟 tracker 在 scrollback 修剪时移除 block（command-blocks.ts 真实行为）
    blocks.length = 0;
    tracker.fire();
    expect(store.isFolded(1)).toBe(false);
    expect(store.folds.length).toBe(0);
  });

  it("dispose() clears state and unsubscribes listeners", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    let onChangeCalls = 0;
    store.onChange(() => onChangeCalls++);
    store.dispose();
    expect(store.folds.length).toBe(0);
    // 之后即使 fold 被尝试，监听也不应再被叫
    f.triggerResize();
    expect(onChangeCalls).toBe(0);
  });
});

describe("FoldStore — onChange notifications", () => {
  it("fires onChange on fold/unfold", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    let calls = 0;
    store.onChange(() => calls++);
    store.fold(1);
    expect(calls).toBe(1);
    store.unfold(1);
    expect(calls).toBe(2);
  });

  it("does NOT fire onChange on failed fold", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const tracker = fakeTracker([]);
    const store = createFoldStore(f.term, tracker);
    let calls = 0;
    store.onChange(() => calls++);
    store.fold(99); // 不存在
    expect(calls).toBe(0);
    store.dispose();
  });
});
