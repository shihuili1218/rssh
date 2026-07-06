export interface PaintScheduler {
  schedule(): void;
  dispose(): void;
}

export interface PaintSchedulerOptions {
  shouldPaint(): boolean;
  paint(): void;
  requestFrame?: (callback: FrameRequestCallback) => number;
  cancelFrame?: (handle: number) => void;
}

/** Coalesce high-frequency terminal events into at most one paint per frame. */
export function createPaintScheduler(opts: PaintSchedulerOptions): PaintScheduler {
  const requestFrame = opts.requestFrame ?? ((cb) => requestAnimationFrame(cb));
  const cancelFrame = opts.cancelFrame ?? ((handle) => cancelAnimationFrame(handle));
  let frame: number | null = null;
  let disposed = false;

  return {
    schedule() {
      if (disposed || frame !== null || !opts.shouldPaint()) return;
      frame = requestFrame(() => {
        frame = null;
        if (!disposed && opts.shouldPaint()) opts.paint();
      });
    },
    dispose() {
      disposed = true;
      if (frame !== null) {
        cancelFrame(frame);
        frame = null;
      }
    },
  };
}
