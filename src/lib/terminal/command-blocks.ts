/**
 * Command block tracker.
 *
 * Every time the user presses Enter in the terminal, a new command block
 * starts. Each block has a color (cycled from a palette) and a pair of
 * xterm markers (start + end) that follow the scrollback automatically.
 *
 * Rules (deliberately simple — no heuristics, no "smart" filtering):
 *   1. On Enter in normal buffer → close previous block, open new one.
 *   2. On switching to alternate buffer (vim/top/less/tmux) → close current,
 *      do nothing until we're back in normal and user presses Enter again.
 *   3. When a start marker is disposed (scrollback trimmed), drop the block.
 *
 * This module owns no DOM. A renderer (e.g. the overlay bar) subscribes
 * via `onChange` and redraws.
 */
import type { Terminal, IMarker, IDisposable } from "@xterm/xterm";

export interface CommandBlock {
  id: number;
  color: string;
  start: IMarker;
  end: IMarker | null;
}

export interface CommandBlockTracker extends IDisposable {
  readonly blocks: ReadonlyArray<CommandBlock>;
  /** Called whenever blocks array changes (add / close / gc). */
  onChange(fn: () => void): IDisposable;
}

/** Golden-angle HSL cycling — infinite palette, no adjacent-hue collisions. */
function colorForIndex(i: number): string {
  const hue = (i * 137.508) % 360;
  return `hsl(${hue.toFixed(1)}, 65%, 58%)`;
}

export function createCommandBlockTracker(term: Terminal): CommandBlockTracker {
  const blocks: CommandBlock[] = [];
  const listeners = new Set<() => void>();
  let nextId = 1;
  const disposables: IDisposable[] = [];

  const emit = () => listeners.forEach(fn => fn());

  const closeCurrent = () => {
    const cur = blocks[blocks.length - 1];
    if (cur && cur.end === null) {
      // Mark one line above the new prompt. registerMarker(-1) = previous line.
      // If that fails (cursor at top), fall back to current line.
      cur.end = term.registerMarker(-1) ?? term.registerMarker(0);
    }
  };

  const openNew = () => {
    const start = term.registerMarker(0);
    if (!start) return; // can't mark — give up silently
    const id = nextId++;
    const block: CommandBlock = {
      id,
      color: colorForIndex(id),
      start,
      end: null,
    };
    // When start marker is trimmed out of scrollback, drop the block.
    start.onDispose(() => {
      const i = blocks.indexOf(block);
      if (i >= 0) {
        blocks.splice(i, 1);
        emit();
      }
    });
    blocks.push(block);
    emit();
  };

  // Rule 1: Enter in normal buffer. Each `\r` = one new block (includes
  // multiline pastes — pasted lines each get their own block by design).
  disposables.push(
    term.onData((data: string) => {
      if (term.buffer.active.type === "alternate") return;
      for (const ch of data) {
        if (ch === "\r") {
          closeCurrent();
          openNew();
        }
      }
    }),
  );

  // Rule 2: buffer switch. Close on entering alternate; ignore return.
  disposables.push(
    term.buffer.onBufferChange((buf) => {
      if (buf.type === "alternate") closeCurrent();
    }),
  );

  return {
    get blocks() { return blocks; },
    onChange(fn) {
      listeners.add(fn);
      return { dispose: () => listeners.delete(fn) };
    },
    dispose() {
      disposables.forEach(d => d.dispose());
      // Explicitly dispose any markers still alive — otherwise they linger
      // until the terminal itself is disposed.
      blocks.forEach(b => { b.start.dispose(); b.end?.dispose(); });
      listeners.clear();
      blocks.length = 0;
    },
  };
}
