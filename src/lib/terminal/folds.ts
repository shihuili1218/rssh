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
 * fold 流程：splice 抽出 → push 空行补齐 → drain ybase 再 y → 重排 ydisp
 *
 * unfold 流程（不 pop 末尾，避免吃掉用户在折叠期间产生的输出）：
 *   splice 把 saved 塞回 → ybase += count（多出来的"曾是空行"自然进入
 *   scrollback）→ 重新 registerMarker 给 block.end（splice 时被 dispose）
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
  /** splice 抽出的 BufferLine 实例（对我们透明） */
  savedLines: unknown[];
}

export interface FoldStore extends IDisposable {
  readonly folds: ReadonlyArray<Fold>;
  fold(blockId: number): boolean;
  unfold(blockId: number): boolean;
  isFolded(blockId: number): boolean;
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

    const saved: unknown[] = [];
    for (let i = 0; i < count; i++) saved.push(buf.lines.get(startLine + i));

    // splice 抽出 → marker 自动迁移 + 范围内 marker 自动 dispose（含 block.end）
    buf.lines.splice(startLine, count);

    // 不变量 (1)：lines.length === ybase + rows。补回 count 个空行。
    for (let i = 0; i < count; i++) buf.lines.push(buf.getBlankLine(BLANK_ATTR));

    // 不变量 (2)：cursor 跟随内容。绝对位置 -= count。先消 ybase，不够再消 y。
    const ybaseDrain = Math.min(buf.ybase, count);
    buf.ybase -= ybaseDrain;
    buf.y -= count - ybaseDrain;
    if (buf.y < 0) buf.y = 0;

    // 视口顶端：在 splice 后则减 count；在区间内塌到 startLine；最后夹到 [0, ybase]
    if (buf.ydisp >= startLine + count) buf.ydisp -= count;
    else if (buf.ydisp >= startLine) buf.ydisp = startLine;
    if (buf.ydisp > buf.ybase) buf.ydisp = buf.ybase;

    folds.set(blockId, { id: nextId++, blockId, count, savedLines: saved });
    syncViewport(term);
    term.refresh(0, term.rows - 1);
    emit();
    return true;
  }

  function unfold(blockId: number): boolean {
    const f = folds.get(blockId);
    if (!f) return false;
    const block = tracker.blocks.find((b) => b.id === blockId);
    if (!block || block.start.isDisposed) {
      // block 已被 scrollback 吞噬 — 丢弃 saved（用户也看不见原内容了）
      folds.delete(blockId);
      emit();
      return false;
    }
    const buf = getBuf(term);
    const insertAt = block.start.line + 1;
    const wasLive = buf.ydisp === buf.ybase;

    // splice 塞回 → marker 反向迁移
    buf.lines.splice(insertAt, 0, ...f.savedLines);

    // 不变量 (1)：lines.length 长了 count，让 ybase 跟着长（多出来的"曾是空行"
    // 自然进入 scrollback）。不 pop 末尾 — 避免吞掉用户在折叠期间产生的内容。
    buf.ybase += f.count;

    // 视口：
    //   live 状态：上滚 count 行 — 让刚展开的内容紧贴 cursor 上方显示。
    //     若直接用 ybase 做 live（ydisp=ybase'），新增的 count 行会被推入
    //     scrollback，用户看不到。这对"首块在 buffer 顶端"的场景尤其糟。
    //     用 ybase' - count 等价于"viewport 维持在原 live 行"，新展开的内容
    //     立刻进入可见区。后续输出会自动 snap 回 live（xterm 默认行为）。
    //   非 live 状态：用户手动滚到了某处，按 insertAt 平移保持内容相对位置不变。
    if (wasLive) buf.ydisp = Math.max(0, buf.ybase - f.count);
    else if (buf.ydisp >= insertAt) buf.ydisp += f.count;
    // y 不变 — cursor 绝对位置 = ybase + y，ybase += count，cursor 自动 +count

    // 重装 block.end：splice 时它被 dispose，block-bar 渲染依赖它的位置
    try {
      const newEnd = buf.addMarker(block.start.line + f.count);
      // CommandBlock.end 字段未声明 readonly，可直接赋值；tracker 不知情
      // 但本 FoldStore 自己 emit 让消费者重绘。
      (block as { end: IMarker | null }).end = newEnd;
    } catch {
      // addMarker 异常则保持 end=disposed，block-bar 会回退到 cursor 位置 — 可接受
    }

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
    onChange(fn) {
      listeners.add(fn);
      return { dispose: () => listeners.delete(fn) };
    },
    dispose() {
      disposables.forEach((d) => d.dispose());
      folds.clear();
      listeners.clear();
    },
  };
}
