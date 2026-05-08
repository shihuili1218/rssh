import { describe, it, expect } from "vitest";
import { createCommandBlockTracker } from "./command-blocks.ts";

/* ─────────────────────────────────────────────────────────────
 * Fake xterm.js Terminal — 实现 createCommandBlockTracker 用到的
 * 最小子集。够测，不模拟真实滚动行为。
 * ───────────────────────────────────────────────────────────── */

type BufType = "normal" | "alternate";

interface FakeMarker {
  id: number;
  disposed: boolean;
  onDispose(fn: () => void): { dispose: () => void };
  dispose(): void;
}

function fakeTerm() {
  let bufferType: BufType = "normal";
  // Set 而非 Array：disposer 能 O(1) 拆订阅，且能在测试里观察"还剩几个 listener"。
  const dataListeners = new Set<(s: string) => void>();
  const bufferChangeListeners = new Set<(b: { type: BufType }) => void>();
  let markerCounter = 0;
  const allMarkers: FakeMarker[] = [];

  const makeMarker = (): FakeMarker => {
    const onDisposeFns: Array<() => void> = [];
    const m: FakeMarker = {
      id: ++markerCounter,
      disposed: false,
      onDispose(fn) {
        onDisposeFns.push(fn);
        return { dispose: () => {} };
      },
      dispose() {
        if (this.disposed) return;
        this.disposed = true;
        onDisposeFns.forEach((f) => f());
      },
    };
    allMarkers.push(m);
    return m;
  };

  const term = {
    onData(fn: (s: string) => void) {
      dataListeners.add(fn);
      return { dispose: () => dataListeners.delete(fn) };
    },
    buffer: {
      get active() {
        return { type: bufferType };
      },
      onBufferChange(fn: (b: { type: BufType }) => void) {
        bufferChangeListeners.add(fn);
        return { dispose: () => bufferChangeListeners.delete(fn) };
      },
    },
    registerMarker(_line: number): FakeMarker | undefined {
      return makeMarker();
    },
  };

  return {
    term: term as unknown as Parameters<typeof createCommandBlockTracker>[0],
    pushData(s: string) {
      dataListeners.forEach((f) => f(s));
    },
    setBuffer(t: BufType) {
      bufferType = t;
      bufferChangeListeners.forEach((f) => f({ type: t }));
    },
    markers: allMarkers,
    listenerCount: () => dataListeners.size + bufferChangeListeners.size,
  };
}

describe("createCommandBlockTracker — Enter opens blocks", () => {
  it("starts with no blocks", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    expect(t.blocks.length).toBe(0);
    t.dispose();
  });

  it("Enter in normal buffer opens a new block", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("\r");
    expect(t.blocks.length).toBe(1);
    expect(t.blocks[0].end).toBeNull();
    expect(typeof t.blocks[0].color).toBe("string");
    expect(t.blocks[0].color).toMatch(/^hsl\(/);
    t.dispose();
  });

  it("multiple Enters open multiple blocks with distinct colors", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("\r");
    f.pushData("\r");
    f.pushData("\r");
    expect(t.blocks.length).toBe(3);
    const colors = t.blocks.map((b) => b.color);
    expect(new Set(colors).size).toBe(3);
    // 第一块被 close（end 不为空），第三块还开着
    expect(t.blocks[0].end).not.toBeNull();
    expect(t.blocks[1].end).not.toBeNull();
    expect(t.blocks[2].end).toBeNull();
    t.dispose();
  });

  it("ids increment monotonically", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("\r\r");
    expect(t.blocks[1].id).toBeGreaterThan(t.blocks[0].id);
    t.dispose();
  });

  it("non-Enter chars do not open blocks", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("ls -la");
    expect(t.blocks.length).toBe(0);
    t.dispose();
  });

  it("Enter in alternate buffer is ignored", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.setBuffer("alternate");
    f.pushData("\r");
    f.pushData("\r");
    expect(t.blocks.length).toBe(0);
    t.dispose();
  });

  it("paste of multi-line input opens one block per \\r", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("first\rsecond\rthird\r");
    expect(t.blocks.length).toBe(3);
    t.dispose();
  });
});

describe("createCommandBlockTracker — buffer switch closes current", () => {
  it("entering alternate buffer closes the current open block", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("\r");
    expect(t.blocks[0].end).toBeNull();
    f.setBuffer("alternate");
    expect(t.blocks[0].end).not.toBeNull();
    t.dispose();
  });

  it("returning to normal buffer does NOT auto-open a block", () => {
    // 规则2：alternate→normal 不动作，必须用户再敲 Enter
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("\r");
    f.setBuffer("alternate");
    f.setBuffer("normal");
    expect(t.blocks.length).toBe(1);
    expect(t.blocks[0].end).not.toBeNull();
    t.dispose();
  });
});

describe("createCommandBlockTracker — onChange notifications", () => {
  it("fires onChange on each block open", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    let calls = 0;
    t.onChange(() => calls++);
    f.pushData("\r");
    f.pushData("\r");
    // 每次 Enter: closeCurrent (无 emit) + openNew (emit)
    expect(calls).toBe(2);
    t.dispose();
  });

  it("onChange unsubscribe stops further calls", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    let calls = 0;
    const sub = t.onChange(() => calls++);
    f.pushData("\r");
    sub.dispose();
    f.pushData("\r");
    expect(calls).toBe(1);
    t.dispose();
  });
});

describe("createCommandBlockTracker — marker disposal", () => {
  it("disposing a start marker drops the block from the list", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("\r");
    f.pushData("\r");
    expect(t.blocks.length).toBe(2);
    // 第一块的 start marker 触发 dispose（模拟 scrollback 修剪）
    t.blocks[0].start.dispose();
    expect(t.blocks.length).toBe(1);
    t.dispose();
  });

  it("dispose() empties the blocks array", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("\r");
    f.pushData("\r");
    expect(t.blocks.length).toBe(2);
    t.dispose();
    expect(t.blocks.length).toBe(0);
  });

  /// 回归 net：dispose() 必须 dispose 所有 block 的 marker，不能因为
  /// onDispose 回调 splice blocks 而漏掉后续 block。
  it("dispose() disposes markers of all blocks, not just the first", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("\r");
    f.pushData("\r");
    const totalBefore = f.markers.length;
    expect(totalBefore).toBeGreaterThan(0);
    t.dispose();
    for (const m of f.markers) {
      expect(m.disposed).toBe(true);
    }
  });

  it("dispose() unsubscribes onData / onBufferChange listeners", () => {
    // 不止断言"看不到副作用"——直接验证 listener 集合被清掉。
    // 这是真 leak 探针：tracker 漏不取消订阅会让 fake term 的 Set size 不归零。
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    expect(f.listenerCount()).toBeGreaterThan(0);
    t.dispose();
    expect(f.listenerCount()).toBe(0);
  });

  it("after dispose() further pushData / setBuffer cause no state change", () => {
    const f = fakeTerm();
    const t = createCommandBlockTracker(f.term);
    f.pushData("\r");
    t.dispose();
    let onChangeCalls = 0;
    t.onChange(() => onChangeCalls++);
    f.pushData("\r"); // 不应再开 block
    f.setBuffer("alternate");
    f.setBuffer("normal");
    expect(t.blocks.length).toBe(0);
    expect(onChangeCalls).toBe(0);
  });
});
