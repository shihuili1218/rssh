export interface TerminalWriteBatcher {
  write(data: Uint8Array): void;
  flush(): void;
  dispose(): void;
}

export interface TerminalWriteBatcherOptions {
  write(data: Uint8Array): void;
  delayMs?: number;
  maxBytes?: number;
  setTimer?: (callback: () => void, delayMs: number) => ReturnType<typeof setTimeout>;
  clearTimer?: (handle: ReturnType<typeof setTimeout>) => void;
}

const DEFAULT_DELAY_MS = 8;
const DEFAULT_MAX_BYTES = 64 * 1024;

function concatChunks(chunks: Uint8Array[], totalBytes: number): Uint8Array {
  if (chunks.length === 1) return chunks[0];

  const out = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}

/** Batch bursts of transport chunks before writing them into xterm. */
export function createTerminalWriteBatcher(opts: TerminalWriteBatcherOptions): TerminalWriteBatcher {
  const delayMs = opts.delayMs ?? DEFAULT_DELAY_MS;
  const maxBytes = opts.maxBytes ?? DEFAULT_MAX_BYTES;
  const setTimer = opts.setTimer ?? ((cb, ms) => setTimeout(cb, ms));
  const clearTimer = opts.clearTimer ?? ((handle) => clearTimeout(handle));
  let timer: ReturnType<typeof setTimeout> | null = null;
  let chunks: Uint8Array[] = [];
  let totalBytes = 0;
  let disposed = false;

  function clearPendingTimer() {
    if (timer !== null) {
      clearTimer(timer);
      timer = null;
    }
  }

  function flushNow() {
    if (!totalBytes) return;

    const data = concatChunks(chunks, totalBytes);
    chunks = [];
    totalBytes = 0;
    opts.write(data);
  }

  function schedule() {
    if (timer !== null) return;
    timer = setTimer(() => {
      timer = null;
      if (!disposed) flushNow();
    }, delayMs);
  }

  return {
    write(data: Uint8Array) {
      if (disposed || data.length === 0) return;
      chunks.push(data);
      totalBytes += data.length;
      if (totalBytes >= maxBytes) {
        clearPendingTimer();
        flushNow();
        return;
      }
      schedule();
    },
    flush() {
      clearPendingTimer();
      if (!disposed) flushNow();
    },
    dispose() {
      disposed = true;
      clearPendingTimer();
      chunks = [];
      totalBytes = 0;
    },
  };
}
