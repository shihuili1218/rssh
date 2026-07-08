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
 * unfold 流程：splice 塞回 → 删除本 fold 当初补在 buffer 里的空行。
 *   补偿空行可能已被后续 fold 推到 buffer 中间；因此不能只看末尾，
 *   更不能用全局 pushedBlanks 去 pop 其他 fold 的空行。按对象引用找到
 *   本 fold 自己的空行，确认仍为空后删除，才能让非 LIFO 展开恢复不变量。
 *
 * Auto-unfold 触发：
 *   - 终端 resize（saved 是按旧列宽抓的，新列宽展开会错位）
 *   - block.start 死亡（scrollback 修剪到该 block 之前）— 通过监听
 *     tracker.onChange 检测 block 从 tracker 消失来代理
 *
 * ⚠️ Private-API warning: depends on _core.buffer's lines/ybase/ydisp/y/
 *    getBlankLine/addMarker, plus _core._viewport.queueSync (scrollbar resync).
 *    package.json pins "@xterm/xterm": "6.0.0". Any xterm bump must re-run
 *    folds.test.ts — but that test uses a FAKE terminal (it verifies this
 *    file's logic, not the real xterm internals), so a version bump also
 *    requires re-checking these private hooks against the new build by hand.
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
   *  unfold 时最多删除这些补偿空行（不是 count）才能精确还原长度。 */
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
  queueSync(yDisp?: number): void;
}

function getBuf(term: Terminal): PrivateBuffer {
  return (term as unknown as { _core: { buffer: PrivateBuffer } })._core.buffer;
}

/** The viewport derives its scroll height from buffer.lines.length and only
 *  resyncs on the core's scroll/resize events. We splice buffer.lines directly,
 *  bypassing those events, so the scrollbar would otherwise go stale ("can't
 *  scroll up after unfold"). queueSync() recomputes it on the next render frame,
 *  and folds always calls term.refresh() right after, which drives that frame.
 *  (xterm 6.0 renamed _core.viewport.syncScrollArea → _core._viewport.queueSync.) */
function syncViewport(term: Terminal): void {
  const vp = (term as unknown as { _core: { _viewport?: PrivateViewport } })._core._viewport;
  vp?.queueSync();
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, n));
}

function findLineIndex(lines: PrivateBuffer["lines"], needle: unknown): number {
  for (let i = 0; i < lines.length; i++) {
    if (lines.get(i) === needle) return i;
  }
  return -1;
}

function isStillBlankLine(line: unknown): boolean {
  const candidate = line as { getTrimmedLength?: () => number; isWrapped?: boolean } | null;
  if (candidate && typeof candidate.getTrimmedLength === "function") {
    return candidate.getTrimmedLength() === 0 && candidate.isWrapped !== true;
  }
  return true;
}

export function createFoldStore(term: Terminal, tracker: CommandBlockTracker): FoldStore {
  // 以 blockId 为键便于 O(1) 判断"该 block 是否折叠"。Fold 的 id 仅用于调试。
  const folds = new Map<number, Fold>();
  const listeners = new Set<() => void>();
  const disposables: IDisposable[] = [];
  let nextId = 1;
  // 所有 fold 期间 push 进 buffer 的 blank BufferLine 引用集合。
  // 每个 Fold 也保存自己的 pushedBlankRefs；unfold 只能删除当前 Fold
  // 自己的仍为空的引用，避免非 LIFO 展开误删其他 Fold 的补偿行。
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
    const cursorAbsBefore = buf.ybase + buf.y;
    let nextCursorAbs = insertAt <= cursorAbsBefore ? cursorAbsBefore + f.count : cursorAbsBefore;
    let nextYdisp = buf.ydisp;
    const wasLive = buf.ydisp === buf.ybase;

    // Compensation refs below the active cursor are still untouched screen
    // padding. Refs at/above the cursor may have been consumed by real output,
    // even if they still render as blank lines, so unfold must preserve them.
    const untouchedBlankRefs = new Set(
      f.pushedBlankRefs.filter((line) => {
        const index = findLineIndex(buf.lines, line);
        return index > cursorAbsBefore && isStillBlankLine(line);
      }),
    );

    // splice 塞回 → marker 反向迁移
    // 分块插：Array spread 在 V8 上有 ~65k 参数硬上限（large build log /
    // find / 输出轻易就超过）。一次性 splice(...savedLines) 会抛 RangeError。
    const lengthBeforeInsert = buf.lines.length;
    const SPLICE_CHUNK = 32768;
    for (let i = 0; i < f.savedLines.length; i += SPLICE_CHUNK) {
      const chunk = f.savedLines.slice(i, i + SPLICE_CHUNK);
      buf.lines.splice(insertAt + i, 0, ...chunk);
    }
    if (insertAt <= nextYdisp) nextYdisp += f.count;

    // CircularList.splice 会在 maxLength 满时从头 trim；我们直接碰私有
    // lines，必须自己把 cursor/viewport 的绝对行同步扣回来。
    const trimmedDuringInsert = Math.max(0, lengthBeforeInsert + f.count - buf.lines.length);
    if (trimmedDuringInsert > 0) {
      nextCursorAbs = Math.max(0, nextCursorAbs - trimmedDuringInsert);
      nextYdisp = Math.max(0, nextYdisp - trimmedDuringInsert);
    }
    if (block.start.isDisposed || block.start.line < 0) {
      discardFold(f);
      folds.delete(blockId);
      syncViewport(term);
      term.refresh(0, term.rows - 1);
      emit();
      return false;
    }

    // 删除本 fold 自己补的空行。后续 fold 可能把这些行从末尾推到中间，
    // 所以按当前 index 从下往上 splice，避免 index 级联偏移。
    const removable = Array.from(untouchedBlankRefs)
      .map((line) => ({ line, index: findLineIndex(buf.lines, line) }))
      .filter(({ line, index }) => index >= 0 && isStillBlankLine(line))
      .sort((a, b) => b.index - a.index);

    for (const { line, index } of removable) {
      buf.lines.splice(index, 1);
      pushedBlanks.delete(line);
      if (index < nextCursorAbs) nextCursorAbs--;
      if (index < nextYdisp) nextYdisp--;
    }

    buf.ybase = Math.max(0, buf.lines.length - term.rows);
    buf.y = clamp(nextCursorAbs - buf.ybase, 0, term.rows - 1);
    buf.ydisp = wasLive ? buf.ybase : clamp(nextYdisp, 0, buf.ybase);

    // 重装 block.end：splice 时它被 dispose，block-bar 渲染依赖它的位置
    try {
      const newEnd = buf.addMarker(block.start.line + f.count);
      // CommandBlock.end 字段未声明 readonly，可直接赋值；tracker 不知情
      // 但本 FoldStore 自己 emit 让消费者重绘。
      (block as { end: IMarker | null }).end = newEnd;
    } catch {
      // addMarker 异常则保持 end=disposed，block-bar 会回退到 cursor 位置 — 可接受
    }

    // 清剩余 refs：已删除的上面删过；未删除（被写入/被 trim）也要从
    // 全局集合里释放。discardFold 是幂等的，统一收口更清楚。
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
