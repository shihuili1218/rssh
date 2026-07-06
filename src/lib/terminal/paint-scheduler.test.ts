import { describe, expect, it, vi } from "vitest";

import { createPaintScheduler } from "./paint-scheduler.ts";

function setup() {
  let callback: FrameRequestCallback | null = null;
  let shouldPaint = true;
  const paint = vi.fn();
  const cancelFrame = vi.fn();
  const requestFrame = vi.fn((cb: FrameRequestCallback) => {
    callback = cb;
    return 42;
  });
  const scheduler = createPaintScheduler({
    shouldPaint: () => shouldPaint,
    paint,
    requestFrame,
    cancelFrame,
  });

  return {
    scheduler,
    paint,
    requestFrame,
    cancelFrame,
    setShouldPaint(v: boolean) { shouldPaint = v; },
    flush() {
      const cb = callback;
      callback = null;
      cb?.(0);
    },
  };
}

describe("createPaintScheduler", () => {
  it("coalesces repeated schedule calls into one frame", () => {
    const f = setup();

    f.scheduler.schedule();
    f.scheduler.schedule();
    f.scheduler.schedule();

    expect(f.requestFrame).toHaveBeenCalledTimes(1);
    f.flush();
    expect(f.paint).toHaveBeenCalledTimes(1);
  });

  it("allows another paint after the pending frame runs", () => {
    const f = setup();

    f.scheduler.schedule();
    f.flush();
    f.scheduler.schedule();
    f.flush();

    expect(f.requestFrame).toHaveBeenCalledTimes(2);
    expect(f.paint).toHaveBeenCalledTimes(2);
  });

  it("does not request a frame when painting is disabled", () => {
    const f = setup();
    f.setShouldPaint(false);

    f.scheduler.schedule();

    expect(f.requestFrame).not.toHaveBeenCalled();
    expect(f.paint).not.toHaveBeenCalled();
  });

  it("rechecks whether painting is still needed when the frame runs", () => {
    const f = setup();

    f.scheduler.schedule();
    f.setShouldPaint(false);
    f.flush();

    expect(f.paint).not.toHaveBeenCalled();
  });

  it("cancels pending work on dispose", () => {
    const f = setup();

    f.scheduler.schedule();
    f.scheduler.dispose();
    f.flush();
    f.scheduler.schedule();

    expect(f.cancelFrame).toHaveBeenCalledWith(42);
    expect(f.paint).not.toHaveBeenCalled();
    expect(f.requestFrame).toHaveBeenCalledTimes(1);
  });
});
