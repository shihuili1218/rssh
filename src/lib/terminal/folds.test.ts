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

function fakeTerm(opts: { rows: number; initialLines: number; cursorY: number; ybase?: number }) {
  const rows = opts.rows;
  let ybase = opts.ybase ?? 0;
  let ydisp = ybase;
  let y = opts.cursorY;
  const lineArray: { content: string }[] = [];
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

  // CircularList 的子集 + xterm Buffer.addMarker 内嵌的迁移逻辑
  const lines = {
    get length() {
      return lineArray.length;
    },
    get(i: number) {
      return lineArray[i];
    },
    splice(start: number, deleteCount: number, ...items: { content: string }[]) {
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
      }
    },
    push(item: { content: string }) {
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
    getBlankLine: (_attr: unknown) => ({ content: "<blank>" }),
    addMarker: (line: number) => makeMarker(line),
  };

  const resizeListeners = new Set<() => void>();

  const term = {
    rows,
    refresh: (_a: number, _b: number) => {},
    onResize(fn: () => void) {
      resizeListeners.add(fn);
      return { dispose: () => resizeListeners.delete(fn) };
    },
    _core: { buffer },
  };

  return {
    term: term as unknown as Parameters<typeof createFoldStore>[0],
    buffer,
    lineContents: () => lineArray.map((l) => l.content),
    markers,
    makeMarker,
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

  it("unfold falls back to no-pop when end-of-buffer was overwritten (no scrollback case)", () => {
    // ybase=0 → pushCount=count。模拟 xterm 在折叠期间往末尾追加了用户内容
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
    expect(afterUnfold.length).toBe(afterFold.length + 12);
  });

  it("unfold partial-pops when cursor can't drop full pushCount rows", () => {
    // rows=10, ybase=0 → pushCount=count=8。手动推大 y 模拟用户敲 Enter
    const f = fakeTerm({ rows: 10, initialLines: 10, cursorY: 9 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(8);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    f.buffer.y = 8;
    const afterFold = f.snapshot();
    store.unfold(1);
    const afterUnfold = f.snapshot();
    // pushCount=8, kMax=min(8, 10-1-8)=1 → popped=1, remaining=7
    expect(afterUnfold.length).toBe(afterFold.length + 7);
  });

  it("unfold bails out when buffer end is overwritten by user output", () => {
    // pop 是从末尾开始；末尾就是非 blank 时，第一次 pop 就 push-back + break，
    // 一个都不 pop。这是 unfold 的"安全 pop"契约：宁可让 lines 临时膨胀，
    // 也不能把用户写过的内容当 blank 给吞了。
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
    // splice 塞回 4 行 saved；pop 一次失败、popped=0；净 +4
    expect(afterUnfold.length).toBe(afterFold.length + 4);
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
});

describe("FoldStore — auto-cleanup", () => {
  it("onResize unfolds all active folds", () => {
    const f = fakeTerm({ rows: 24, initialLines: 24, cursorY: 14 });
    const s = f.makeMarker(0);
    const e = f.makeMarker(12);
    const tracker = fakeTracker([makeBlock(1, s, e)]);
    const store = createFoldStore(f.term, tracker);
    store.fold(1);
    expect(store.isFolded(1)).toBe(true);
    f.triggerResize();
    expect(store.isFolded(1)).toBe(false);
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
