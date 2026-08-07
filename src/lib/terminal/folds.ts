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
 * fold 流程：splice 抽出 → push 空行补齐（记下引用）→ drain ybase 再 y → 重排 ydisp。
 * 手动折叠抽完整 body；自动折叠只抽最早的前缀，保留最新输出可见。
 *
 * unfold 流程：splice 塞回 → 删除本 fold 当初补在 buffer 里的空行。
 *   补偿空行可能已被后续 fold 推到 buffer 中间；因此不能只看末尾，
 *   更不能用全局集合去 pop 其他 fold 的空行。按对象引用找到
 *   本 fold 自己的空行，确认仍为空后删除，才能让非 LIFO 展开恢复不变量。
 *
 * Auto-unfold 触发：
 *   - 终端 resize 前由调用方执行 unfoldAll（saved 是按旧列宽抓的）
 *   - block.start 死亡（scrollback 修剪到该 block 之前）— 通过监听
 *     tracker.onChange 检测 block 从 tracker 消失来代理
 *
 * 缓存不变量：所有 fold.savedLines 的总数受 maxCachedLines 限制。预算
 * 满时先展开旧 fold，把历史交还 xterm；单个超长活动块只保留最新的
 * 可恢复前缀。调用方保证 visible + cached < xterm scrollback。
 *
 * ⚠️ Private-API warning: depends on _core.buffer's lines/ybase/ydisp/y/
 *    getBlankLine/addMarker, plus _core._viewport.queueSync (scrollbar resync).
 *    package.json pins "@xterm/xterm": "6.0.0". Any xterm bump must re-run
 *    folds.test.ts — but that test uses a FAKE terminal (it verifies this
 *    file's logic, not the real xterm internals), so a version bump also
 *    requires re-checking these private hooks against the new build by hand.
 */
import type { Terminal, IDisposable, IMarker } from "@xterm/xterm";
import type { CommandBlock, CommandBlockTracker } from "./command-blocks";

export interface Fold {
  /** 自增 id（仅用于调试）；外界以 blockId 索引 */
  id: number;
  blockId: number;
  /** full = user folded the complete closed block; prefix = automatic oldest rows. */
  kind: "full" | "prefix";
  /** body 行数 */
  count: number;
  /** splice 抽出的 BufferLine 实例（对我们透明） */
  savedLines: unknown[];
  /** 这次 fold push 进 buffer 末尾的空行 refs。
   *  refs 只由对应 Fold 持有；删除 fold 记录即可释放。 */
  pushedBlankRefs: unknown[];
}

interface FoldState extends Fold {
  /** Number of leading compensation lines the output cursor has reached.
   *  Monotonic: cursor-up must not make already-consumed blank lines removable. */
  consumedBlankCount: number;
}

export interface FoldStore extends IDisposable {
  readonly folds: ReadonlyArray<Fold>;
  fold(blockId: number): boolean;
  unfold(blockId: number): boolean;
  isFolded(blockId: number): boolean;
  /** O(1) 取 fold 记录。derive blockRects 等高频路径必走这条，
   *  避免每帧都对 folds 数组做线性 find。 */
  getFold(blockId: number): Fold | undefined;
  /** Expand every fold before xterm changes row/column geometry. */
  unfoldAll(): void;
  /** Reapply the configured automatic limit after xterm reflows its rows. */
  enforceAutoFold(): void;
  /** 折叠状态变化时通知（fold/unfold/scrollback 失效）。 */
  onChange(fn: () => void): IDisposable;
}

export interface FoldStoreOptions {
  /** Keep this many newest body rows visible in a growing command block. */
  maxVisibleLines?: number;
  /** Global saved-line budget across every fold in this terminal. */
  maxCachedLines?: number;
  /** Visual block controls must be available before automatic folding hides rows. */
  shouldAutoFold?: () => boolean;
}

/** xterm 默认 attr（fg=0,bg=0），与 DEFAULT_ATTR_DATA 等价。getBlankLine 必填。 */
const BLANK_ATTR = { fg: 0, bg: 0, extended: { ext: 0, urlId: 0, underlineStyle: 0 } };
// Bound work inside one xterm parse batch without splicing on every LF. The
// settings cap leaves at least 99 scrollback rows, so 32 is safely below the
// distance at which xterm could trim the current block's start marker.
const AUTO_FOLD_BATCH_LINES = 32;

interface PrivateBuffer {
  lines: {
    length: number;
    get(i: number): unknown;
    splice(start: number, deleteCount: number, ...items: unknown[]): void;
    push(item: unknown): void;
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
  const core = (term as unknown as {
    _core: { buffer: PrivateBuffer; buffers?: { normal: PrivateBuffer } };
  })._core;
  // Folds belong to command history in the normal buffer. The active buffer
  // may be the alternate screen when a resize is requested.
  return core.buffers?.normal ?? core.buffer;
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

function indexLineRefs(lines: PrivateBuffer["lines"], refs: Iterable<unknown>): Map<unknown, number> {
  const targets = new Set(refs);
  const indices = new Map<unknown, number>();
  if (targets.size === 0) return indices;

  for (let i = 0; i < lines.length; i++) {
    const line = lines.get(i);
    if (targets.has(line)) {
      indices.set(line, i);
      if (indices.size === targets.size) break;
    }
  }
  return indices;
}

function isStillBlankLine(line: unknown): boolean {
  const candidate = line as { getTrimmedLength?: () => number; isWrapped?: boolean } | null;
  if (candidate && typeof candidate.getTrimmedLength === "function") {
    return candidate.getTrimmedLength() === 0 && candidate.isWrapped !== true;
  }
  return true;
}

export function createFoldStore(
  term: Terminal,
  tracker: CommandBlockTracker,
  options: FoldStoreOptions = {},
): FoldStore {
  // 以 blockId 为键便于 O(1) 判断"该 block 是否折叠"。Fold 的 id 仅用于调试。
  const folds = new Map<number, FoldState>();
  // Compensation-line identity -> owning fold/index. Cursor events can update
  // consumption in O(1) instead of scanning the entire scrollback per LF.
  const blankOwners = new Map<unknown, { blockId: number; index: number }>();
  const listeners = new Set<() => void>();
  const disposables: IDisposable[] = [];
  let nextId = 1;

  const emit = () => listeners.forEach((fn) => fn());

  const maxVisibleLines = Number.isFinite(options.maxVisibleLines)
    ? Math.max(1, Math.trunc(options.maxVisibleLines!))
    : null;
  const maxCachedLines = Number.isFinite(options.maxCachedLines)
    ? Math.max(1, Math.trunc(options.maxCachedLines!))
    : Number.POSITIVE_INFINITY;

  function discardFold(f: FoldState): void {
    for (const line of f.pushedBlankRefs) {
      const owner = blankOwners.get(line);
      if (owner?.blockId === f.blockId) blankOwners.delete(line);
    }
  }

  function pruneConsumedBlankRefs(f: FoldState): void {
    const consumed = Math.min(f.consumedBlankCount, f.pushedBlankRefs.length);
    if (consumed === 0) return;
    for (const line of f.pushedBlankRefs.slice(0, consumed)) {
      const owner = blankOwners.get(line);
      if (owner?.blockId === f.blockId) blankOwners.delete(line);
    }
    f.pushedBlankRefs.splice(0, consumed);
    f.consumedBlankCount = 0;
    f.pushedBlankRefs.forEach((line, index) => {
      blankOwners.set(line, { blockId: f.blockId, index });
    });
  }

  /** Drop the oldest saved rows first. The xterm buffer has the same policy,
   *  and keeping a larger shadow history would only make unfold trim it again. */
  function enforceSavedLineBudget(currentBlockId: number): void {
    if (!Number.isFinite(maxCachedLines)) return;
    let overflow = Array.from(folds.values())
      .reduce((total, item) => total + item.savedLines.length, 0) - maxCachedLines;
    if (overflow <= 0) return;

    // Restore older folds into xterm before reusing their shadow-cache budget.
    // Silently deleting a full fold would leave only its command row with no
    // label and no way to recover the body.
    for (const blockId of Array.from(folds.keys())) {
      if (blockId === currentBlockId) continue;
      unfold(blockId);
      overflow = Array.from(folds.values())
        .reduce((total, item) => total + item.savedLines.length, 0) - maxCachedLines;
      if (overflow <= 0) return;
    }

    // One long current block can still exceed the whole budget. Match xterm's
    // head-trim policy and retain the newest restorable prefix rows.
    const current = folds.get(currentBlockId);
    if (!current || overflow <= 0) return;
    current.savedLines.splice(0, Math.min(overflow, current.savedLines.length));
    current.count = current.savedLines.length;
  }

  function removeLines(
    blockId: number,
    kind: Fold["kind"],
    startLine: number,
    count: number,
  ): boolean {
    if (count <= 0) return false;
    const existing = folds.get(blockId);
    if (existing && (existing.kind !== "prefix" || kind !== "prefix")) return false;
    if (existing) {
      recordCursorConsumption();
      for (let i = existing.consumedBlankCount; i < existing.pushedBlankRefs.length; i++) {
        if (!isStillBlankLine(existing.pushedBlankRefs[i])) {
          existing.consumedBlankCount = i + 1;
        }
      }
      pruneConsumedBlankRefs(existing);
    }

    const buf = getBuf(term);
    const cursorAbs = buf.ybase + buf.y;
    const endLine = startLine + count - 1;
    if (endLine >= cursorAbs) return false;
    const wasLive = buf.ydisp === buf.ybase;

    const saved: unknown[] = [];
    for (let i = 0; i < count; i++) saved.push(buf.lines.get(startLine + i));

    // Drain scrollback first; only deletions from the visible screen need
    // compensation rows to preserve lines.length === ybase + rows.
    const ybaseDrain = Math.min(buf.ybase, count);
    const pushCount = count - ybaseDrain;
    buf.lines.splice(startLine, count);

    const pushedRefs: unknown[] = [];
    for (let i = 0; i < pushCount; i++) {
      const blank = buf.getBlankLine(BLANK_ATTR);
      buf.lines.push(blank);
      pushedRefs.push(blank);
    }

    buf.ybase -= ybaseDrain;
    buf.y = Math.max(0, buf.y - pushCount);
    if (buf.ydisp >= startLine + count) buf.ydisp -= count;
    else if (buf.ydisp >= startLine) buf.ydisp = startLine;
    if (buf.ydisp > buf.ybase) buf.ydisp = buf.ybase;
    if (wasLive) buf.ydisp = buf.ybase;

    const foldState = existing ?? {
      id: nextId++,
      blockId,
      kind,
      count: 0,
      savedLines: [],
      pushedBlankRefs: [],
      consumedBlankCount: 0,
    };
    foldState.savedLines.push(...saved);
    foldState.count = foldState.savedLines.length;
    const blankOffset = foldState.pushedBlankRefs.length;
    foldState.pushedBlankRefs.push(...pushedRefs);
    folds.set(blockId, foldState);
    pushedRefs.forEach((line, index) => {
      blankOwners.set(line, { blockId, index: blankOffset + index });
    });
    enforceSavedLineBudget(blockId);
    syncViewport(term);
    term.refresh(0, term.rows - 1);
    emit();
    return true;
  }

  function fold(blockId: number): boolean {
    if (folds.has(blockId)) return false;
    const block = tracker.blocks.find((b) => b.id === blockId);
    if (!block || !block.end) return false;
    if (block.start.isDisposed || block.end.isDisposed) return false;
    const startLine = block.start.line + 1;
    const endLine = block.end.line;
    if (startLine > endLine) return false; // 空 body

    const count = endLine - startLine + 1;
    return removeLines(blockId, "full", startLine, count);
  }

  function canAutoFold(): boolean {
    return maxVisibleLines !== null
      && options.shouldAutoFold?.() !== false
      && term.buffer.active.type === "normal";
  }

  function foldBlockOverflow(block: CommandBlock, minimumExcess: number): void {
    if (maxVisibleLines === null || block.start.isDisposed) return;
    if (folds.get(block.id)?.kind === "full") return;
    const buf = getBuf(term);
    const endLine = block.end === null
      ? buf.ybase + buf.y
      : block.end.isDisposed ? null : block.end.line;
    if (endLine === null) return;
    const visibleBodyLines = endLine - block.start.line;
    const excess = visibleBodyLines - maxVisibleLines;
    if (excess >= minimumExcess) {
      removeLines(block.id, "prefix", block.start.line + 1, excess);
    }
  }

  function foldActiveOverflow(minimumExcess: number): void {
    if (!canAutoFold()) return;
    const block = tracker.blocks[tracker.blocks.length - 1];
    if (block?.end === null) foldBlockOverflow(block, minimumExcess);
  }

  function foldRecentOverflow(): void {
    if (!canAutoFold()) return;
    const blocks = tracker.blocks;
    // Prompt detection runs before this listener and may close the previous
    // block while opening a new prompt block in the same parse batch.
    for (let i = Math.max(0, blocks.length - 2); i < blocks.length; i++) {
      foldBlockOverflow(blocks[i], 1);
    }
  }

  function enforceAutoFold(): void {
    if (!canAutoFold()) return;
    // Markers migrate as older rows are removed, so a stable block snapshot is
    // enough even though removeLines mutates the xterm buffer underneath it.
    for (const block of Array.from(tracker.blocks)) foldBlockOverflow(block, 1);
  }

  function recordCursorConsumption(): void {
    if (folds.size === 0) return;
    // Cursor/line-feed events belong to the active buffer. Folds own normal
    // history, so an alternate-screen application must not consume normal
    // compensation rows merely because the dormant normal cursor happens to
    // point at one.
    if (term.buffer.active.type !== "normal") return;
    const buf = getBuf(term);
    const cursorAbs = buf.ybase + buf.y;
    const owner = blankOwners.get(buf.lines.get(cursorAbs));
    if (!owner) return;
    const fold = folds.get(owner.blockId);
    if (fold && owner.index >= fold.consumedBlankCount) {
      fold.consumedBlankCount = owner.index + 1;
    }
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

    // Cursor position is not history: output can move down through blank lines
    // and then cursor-up. Keep the event high-water mark, and also treat every
    // blank before the last modified compensation line as consumed. The latter
    // covers one parser batch that moves down, writes, then returns to its start.
    recordCursorConsumption();
    for (let i = f.consumedBlankCount; i < f.pushedBlankRefs.length; i++) {
      if (!isStillBlankLine(f.pushedBlankRefs[i])) f.consumedBlankCount = i + 1;
    }
    const blankRefIndicesBeforeInsert = indexLineRefs(buf.lines, f.pushedBlankRefs);
    const untouchedBlankRefs = new Set(
      f.pushedBlankRefs.slice(f.consumedBlankCount).filter((line) => {
        const index = blankRefIndicesBeforeInsert.get(line);
        return index !== undefined && index > cursorAbsBefore && isStillBlankLine(line);
      }),
    );

    // splice 塞回 → marker 反向迁移
    // 分块插：Array spread 在 V8 上有 ~65k 参数硬上限（large build log /
    // find / 输出轻易就超过）。一次性 splice(...savedLines) 会抛 RangeError。
    const SPLICE_CHUNK = 32768;
    let inserted = 0;
    let trimmedDuringInsert = 0;
    for (let i = 0; i < f.savedLines.length; i += SPLICE_CHUNK) {
      const chunk = f.savedLines.slice(i, i + SPLICE_CHUNK);
      // CircularList enforces maxLength after every splice, not after the
      // whole logical insertion. Each head trim shifts the next insertion
      // point left; using insertAt+i would append later chunks out of order.
      const chunkInsertAt = clamp(
        insertAt + inserted - trimmedDuringInsert,
        0,
        buf.lines.length,
      );
      const lengthBeforeChunk = buf.lines.length;
      buf.lines.splice(chunkInsertAt, 0, ...chunk);
      trimmedDuringInsert += Math.max(
        0,
        lengthBeforeChunk + chunk.length - buf.lines.length,
      );
      inserted += chunk.length;
    }
    if (insertAt <= nextYdisp) nextYdisp += f.count;

    // CircularList.splice 会在 maxLength 满时从头 trim；我们直接碰私有
    // lines，必须自己把 cursor/viewport 的绝对行同步扣回来。
    if (trimmedDuringInsert > 0) {
      nextCursorAbs = Math.max(0, nextCursorAbs - trimmedDuringInsert);
      nextYdisp = Math.max(0, nextYdisp - trimmedDuringInsert);
    }

    // Delete this fold's untouched compensation rows before either return
    // path. Head trimming may dispose block.start, but it does not make those
    // artificial rows real terminal history.
    const removableIndices = indexLineRefs(buf.lines, untouchedBlankRefs);
    const removable = Array.from(untouchedBlankRefs)
      .map((line) => ({ line, index: removableIndices.get(line) }))
      .filter((item): item is { line: unknown; index: number } => item.index !== undefined && isStillBlankLine(item.line))
      .sort((a, b) => b.index - a.index);

    for (const { index } of removable) {
      buf.lines.splice(index, 1);
      if (index < nextCursorAbs) nextCursorAbs--;
      if (index < nextYdisp) nextYdisp--;
    }

    // Head trimming can consume part of the restored history before we remove
    // this fold's compensation rows. In that case deletion may leave fewer
    // lines than the visible viewport. Refill only the missing screen padding;
    // appending at the tail does not change cursor or viewport coordinates.
    while (buf.lines.length < term.rows) {
      buf.lines.push(buf.getBlankLine(BLANK_ATTR));
    }

    if (block.start.isDisposed || block.start.line < 0) {
      // The insertion already mutated the CircularList. Even though the block
      // can no longer be reconstructed, ybase/ydisp/y must describe the new
      // buffer before we drop the fold record.
      buf.ybase = Math.max(0, buf.lines.length - term.rows);
      buf.y = clamp(nextCursorAbs - buf.ybase, 0, term.rows - 1);
      buf.ydisp = wasLive ? buf.ybase : clamp(nextYdisp, 0, buf.ybase);
      discardFold(f);
      folds.delete(blockId);
      syncViewport(term);
      term.refresh(0, term.rows - 1);
      emit();
      return false;
    }

    buf.ybase = Math.max(0, buf.lines.length - term.rows);
    buf.y = clamp(nextCursorAbs - buf.ybase, 0, term.rows - 1);
    buf.ydisp = wasLive ? buf.ybase : clamp(nextYdisp, 0, buf.ybase);

    // A full fold consumed block.end, so recreate it. Prefix folds leave the
    // newest rows in the buffer; a later end marker migrates with insertion.
    if (f.kind === "full") {
      try {
        const newEnd = buf.addMarker(block.start.line + f.count);
        (block as { end: IMarker | null }).end = newEnd;
      } catch {
        // Keep the disposed marker; renderers already fall back to the cursor.
      }
    }

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

  // onCursorMove only reports the final position of a parse batch. onLineFeed
  // also fires for intermediate downward movement, so together they retain the
  // high-water mark when output later moves the cursor back up.
  disposables.push(
    term.onCursorMove(recordCursorConsumption),
    term.onLineFeed(() => {
      recordCursorConsumption();
      foldActiveOverflow(AUTO_FOLD_BATCH_LINES);
    }),
  );
  if (maxVisibleLines !== null) {
    disposables.push(term.onWriteParsed(foldRecentOverflow));
  }

  // scrollback 修剪：tracker 监听 block.start.onDispose 后从 blocks 数组移除。
  // 这里通过 onChange 比对 tracker 现存 block — 折叠记录里若 block 不在了，丢弃。
  disposables.push(tracker.onChange(() => {
    const trackedIds = new Set(tracker.blocks.map((b) => b.id));
    let dropped = false;
    for (const blockId of Array.from(folds.keys())) {
      if (!trackedIds.has(blockId)) {
        const fold = folds.get(blockId);
        if (fold) discardFold(fold);
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
    unfoldAll,
    enforceAutoFold,
    onChange(fn) {
      listeners.add(fn);
      return { dispose: () => listeners.delete(fn) };
    },
    dispose() {
      disposables.forEach((d) => d.dispose());
      folds.clear();
      blankOwners.clear();
      listeners.clear();
    },
  };
}
