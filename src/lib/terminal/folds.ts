/**
 * FoldStore — 命令块折叠/展开。
 *
 * 设计基础（spike 已验证）：
 *   1. xterm Buffer.addMarker 注册了 lines.onDelete/onInsert/onTrim：
 *      splice 时 marker 行号自动迁移、范围内的 marker 自动 dispose
 *   2. 隐藏不变量 lines.length === ybase + rows：splice 后必须用
 *      Buffer.getBlankLine 在末尾补齐
 *   3. 不变量 cursor 内容跟随：splice 在 cursor 上方时 cursor 绝对行
 *      要相应减少（fold）或增加（unfold）
 *
 * fold 流程：splice 抽出 → push 空行补齐（记下引用）→ drain ybase 再 y → 重排 ydisp
 *
 * unfold 流程：splice 塞回 → 部分 pop 还原 buffer 长度。
 *   能 pop 多少 pop 多少（不是全有全无）：
 *     k_max = min(count, rows - 1 - y)   ← cursor 在屏幕上下移的最大量
 *     遍历 pop k_max 次；遇到末尾不是我们 push 的 blank（用户/xterm 写过）
 *     就把它推回，提前停。
 *   实际 popped 次数 → cursor 下移 popped 行
 *   未 pop 的 (count - popped) → ybase 增长这么多（多余行进 scrollback）
 *
 *   设计核心：cursor-overflow 和 end-not-blank 是物理约束，不是全或无。
 *   尽力 pop 让 buffer 不必要的膨胀降到最小。
 *
 * Auto-unfold 触发：
 *   - 终端 resize（saved 是按旧列宽抓的，新列宽展开会错位）
 *   - block.start 死亡（scrollback 修剪到该 block 之前）— 通过监听
 *     tracker.onChange 检测 block 从 tracker 消失来代理
 *
 * ⚠️ 私有 API 警告：依赖 _core.buffer 的 lines/ybase/ydisp/y/getBlankLine/
 *    addMarker。package.json 已锁 "@xterm/xterm": "5.5.0"。升级 xterm 必须
 *    重跑 folds.test.ts 全套验证。
 */
import type { Terminal, IDisposable, IMarker } from "@xterm/xterm";
import type { CommandBlockTracker } from "./command-blocks";

export interface Fold {
  /** 自增 id（仅用于调试）；外界以 blockId 索引 */
  id: number;
  blockId: number;
  /** body 行数 */
  count: number;
  /** fold 时实际 push 到末尾的空行数 = count - ybaseDrain。
   *  当 ybase 有足够 scrollback 可以让出空间时，pushCount < count。
   *  unfold 时必须 pop 这个数（不是 count）才能精确还原长度。 */
  pushCount: number;
  /** splice 抽出的 BufferLine 实例（对我们透明） */
  savedLines: unknown[];
  /** 这次 fold push 进 buffer 末尾的空行 refs。
   *  fold 记录被丢弃时（unfold-bail / tracker GC），这些 refs 要从全局
   *  pushedBlanks 里删掉，否则 Set 会无限增长——即便 xterm 已经 trim
   *  了那些 BufferLine，我们的 Set 还持有引用，构成内存泄漏。 */
  pushedBlankRefs: unknown[];
}

export interface FoldStore extends IDisposable {
  readonly folds: ReadonlyArray<Fold>;
  fold(blockId: number): boolean;
  unfold(blockId: number): boolean;
  isFolded(blockId: number): boolean;
  /** O(1) 取 fold 记录。derive blockRects 等高频路径必走这条，
   *  避免每帧都对 folds 数组做线性 find。 */
  getFold(blockId: number): Fold | undefined;
  /** 折叠状态变化时通知（fold/unfold/scrollback 失效）。 */
  onChange(fn: () => void): IDisposable;
}

/** xterm 默认 attr（fg=0,bg=0），与 DEFAULT_ATTR_DATA 等价。getBlankLine 必填。 */
const BLANK_ATTR = { fg: 0, bg: 0, extended: { ext: 0, urlId: 0, underlineStyle: 0 } };

interface PrivateBuffer {
  lines: {
    length: number;
    get(i: number): unknown;
    splice(start: number, deleteCount: number, ...items: unknown[]): void;
    push(item: unknown): void;
    pop(): unknown;
  };
  ybase: number;
  ydisp: number;
  y: number;
  getBlankLine(attr: unknown): unknown;
  addMarker(line: number): IMarker;
}

interface PrivateViewport {
  syncScrollArea(immediate?: boolean): void;
}

function getBuf(term: Terminal): PrivateBuffer {
  return (term as unknown as { _core: { buffer: PrivateBuffer } })._core.buffer;
}

/** xterm.Viewport 缓存了 lines.length，不对外暴露失效信号。我们直接 splice
 *  绕过了 _onScroll 事件，Viewport 不知道 scrollback 变长 → 滚动条计算错误，
 *  用户表现为"unfold 后不能向上滚"。手动喊它重算。 */
function syncViewport(term: Terminal): void {
  const vp = (term as unknown as { _core: { viewport?: PrivateViewport } })._core.viewport;
  vp?.syncScrollArea(true);
}

export function createFoldStore(term: Terminal, tracker: CommandBlockTracker): FoldStore {
  // 以 blockId 为键便于 O(1) 判断"该 block 是否折叠"。Fold 的 id 仅用于调试。
  const folds = new Map<number, Fold>();
  const listeners = new Set<() => void>();
  const disposables: IDisposable[] = [];
  let nextId = 1;
  // 所有 fold 期间 push 进 buffer 末尾的 blank BufferLine 引用集合。
  // unfold 用它判断"末尾 count 行是否还是我们 push 的空行"——是 → 安全 pop
  // 让 buffer 长度恢复（无遗留空行、无 scrollbar 虚胀）；否则用户内容已经
  // 把空行挤走，pop 会吞数据 → 退到 no-pop 路径。
  const pushedBlanks = new Set<unknown>();

  const emit = () => listeners.forEach((fn) => fn());

  function fold(blockId: number): boolean {
    if (folds.has(blockId)) return false;
    const block = tracker.blocks.find((b) => b.id === blockId);
    if (!block || !block.end) return false;
    if (block.start.isDisposed || block.end.isDisposed) return false;
    const startLine = block.start.line + 1;
    const endLine = block.end.line;
    if (startLine > endLine) return false; // 空 body

    const buf = getBuf(term);
    const cursorAbs = buf.ybase + buf.y;
    if (endLine >= cursorAbs) return false; // 折叠区间含 cursor 或之后 — 拒绝
    const count = endLine - startLine + 1;
    // 抓 wasLive 在 mutation 之前——和 unfold 对称。用户在底部活线时折叠
    // 上方旧块，ydisp -= count 会把视口推上去脱离底部，体感像"自动滚动"。
    const wasLive = buf.ydisp === buf.ybase;

    const saved: unknown[] = [];
    for (let i = 0; i < count; i++) saved.push(buf.lines.get(startLine + i));

    // 抽 ybase 让出 scrollback 空间，剩下的部分用 push 空行补。
    // 关键：push 数量 = count - ybaseDrain（不是 count！），否则 lines.length
    // 会比实际需要的多 ybaseDrain 行，造成滚动条与内容不同步。
    const ybaseDrain = Math.min(buf.ybase, count);
    const pushCount = count - ybaseDrain;

    // splice 抽出 → marker 自动迁移 + 范围内 marker 自动 dispose（含 block.end）
    buf.lines.splice(startLine, count);

    // 不变量 (1)：lines.length === ybase + rows
    //   splice 后 lines.length 减了 count
    //   ybase 减 ybaseDrain，rows 不变
    //   缺口 = count - ybaseDrain = pushCount → 末尾补 pushCount 行
    const pushedRefs: unknown[] = [];
    for (let i = 0; i < pushCount; i++) {
      const blank = buf.getBlankLine(BLANK_ATTR);
      buf.lines.push(blank);
      pushedBlanks.add(blank);
      pushedRefs.push(blank);
    }

    // 不变量 (2)：cursor 跟随内容。绝对位置 -= count。
    //   ybase 让 ybaseDrain；y 让 pushCount。和 = count。
    buf.ybase -= ybaseDrain;
    buf.y -= pushCount;
    if (buf.y < 0) buf.y = 0;

    // 视口顶端：在 splice 后则减 count；在区间内塌到 startLine；最后夹到 [0, ybase]
    if (buf.ydisp >= startLine + count) buf.ydisp -= count;
    else if (buf.ydisp >= startLine) buf.ydisp = startLine;
    if (buf.ydisp > buf.ybase) buf.ydisp = buf.ybase;
    // wasLive 钉在底部：上面的位移逻辑会把活线模式打破，这里把它拉回来
    if (wasLive) buf.ydisp = buf.ybase;

    folds.set(blockId, {
      id: nextId++, blockId, count, pushCount,
      savedLines: saved, pushedBlankRefs: pushedRefs,
    });
    syncViewport(term);
    term.refresh(0, term.rows - 1);
    emit();
    return true;
  }

  /** 清理 fold 记录的 pushed blank refs。fold 记录被丢弃前必走这条
   *  （unfold-bail / 完成 unfold / tracker GC），否则全局 Set 会泄漏。 */
  function discardFold(f: Fold): void {
    for (const b of f.pushedBlankRefs) pushedBlanks.delete(b);
  }

  function unfold(blockId: number): boolean {
    const f = folds.get(blockId);
    if (!f) return false;
    const block = tracker.blocks.find((b) => b.id === blockId);
    if (!block || block.start.isDisposed) {
      // block 已被 scrollback 吞噬 — 丢弃 saved（用户也看不见原内容了）
      discardFold(f);
      folds.delete(blockId);
      emit();
      return false;
    }
    const buf = getBuf(term);
    const insertAt = block.start.line + 1;
    const wasLive = buf.ydisp === buf.ybase;

    // splice 塞回 → marker 反向迁移
    // 分块插：Array spread 在 V8 上有 ~65k 参数硬上限（large build log /
    // find / 输出轻易就超过）。一次性 splice(...savedLines) 会抛 RangeError。
    const SPLICE_CHUNK = 32768;
    for (let i = 0; i < f.savedLines.length; i += SPLICE_CHUNK) {
      const chunk = f.savedLines.slice(i, i + SPLICE_CHUNK);
      buf.lines.splice(insertAt + i, 0, ...chunk);
    }

    // 目标：pop 掉 fold 时 push 进末尾的 pushCount 行（只有这些是我们的）。
    // count = pushCount + ybaseDrain；ybaseDrain 那部分通过"还原 ybase"恢复，
    // pushCount 那部分通过"pop 末尾 + cursor 下移"恢复。
    const ybaseDrain = f.count - f.pushCount;
    const kMax = Math.max(0, Math.min(f.pushCount, term.rows - 1 - buf.y));
    let popped = 0;
    while (popped < kMax) {
      const item = buf.lines.pop();
      if (!pushedBlanks.has(item)) {
        buf.lines.push(item);
        break;
      }
      pushedBlanks.delete(item);
      popped++;
    }

    const remaining = f.pushCount - popped;
    // 完全 pop（popped == pushCount）：ybase 恢复 ybaseDrain；linesLen 与 fold 前一致
    // 部分 pop：未 pop 的 remaining 行只能让 ybase 多长，linesLen 暂时膨胀
    buf.ybase += ybaseDrain + remaining;
    buf.y += popped;

    if (wasLive) buf.ydisp = buf.ybase;
    else if (buf.ydisp >= insertAt) buf.ydisp += f.count;
    if (buf.ydisp > buf.ybase) buf.ydisp = buf.ybase;

    // 重装 block.end：splice 时它被 dispose，block-bar 渲染依赖它的位置
    try {
      const newEnd = buf.addMarker(block.start.line + f.count);
      // CommandBlock.end 字段未声明 readonly，可直接赋值；tracker 不知情
      // 但本 FoldStore 自己 emit 让消费者重绘。
      (block as { end: IMarker | null }).end = newEnd;
    } catch {
      // addMarker 异常则保持 end=disposed，block-bar 会回退到 cursor 位置 — 可接受
    }

    // 清剩余 refs：pop 循环已删过 popped 个，剩下 remaining 个还在 Set 里。
    // discardFold 是幂等的（重复 delete 同 key 是 no-op），统一收口更清楚。
    discardFold(f);
    folds.delete(blockId);
    syncViewport(term);
    term.refresh(0, term.rows - 1);
    emit();
    return true;
  }

  function unfoldAll(): void {
    for (const blockId of Array.from(folds.keys())) unfold(blockId);
  }

  // resize 自动展开：saved BufferLine 是旧列宽，新列宽下展开会错位
  disposables.push(term.onResize(() => {
    if (folds.size > 0) unfoldAll();
  }));

  // scrollback 修剪：tracker 监听 block.start.onDispose 后从 blocks 数组移除。
  // 这里通过 onChange 比对 tracker 现存 block — 折叠记录里若 block 不在了，丢弃。
  disposables.push(tracker.onChange(() => {
    const trackedIds = new Set(tracker.blocks.map((b) => b.id));
    let dropped = false;
    for (const blockId of Array.from(folds.keys())) {
      if (!trackedIds.has(blockId)) {
        const f = folds.get(blockId);
        if (f) discardFold(f);
        folds.delete(blockId);
        dropped = true;
      }
    }
    if (dropped) emit();
  }));

  return {
    get folds() {
      return Array.from(folds.values());
    },
    fold,
    unfold,
    isFolded(blockId) {
      return folds.has(blockId);
    },
    getFold(blockId) {
      return folds.get(blockId);
    },
    onChange(fn) {
      listeners.add(fn);
      return { dispose: () => listeners.delete(fn) };
    },
    dispose() {
      disposables.forEach((d) => d.dispose());
      folds.clear();
      pushedBlanks.clear();
      listeners.clear();
    },
  };
}
