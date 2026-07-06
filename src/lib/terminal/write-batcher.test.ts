import { describe, expect, it, vi } from "vitest";

import { createTerminalWriteBatcher } from "./write-batcher.ts";

function bytes(values: number[]): Uint8Array {
  return new Uint8Array(values);
}

function setup(maxBytes = 1024) {
  let callback: (() => void) | null = null;
  const write = vi.fn();
  const clearTimer = vi.fn();
  const setTimer = vi.fn((cb: () => void) => {
    callback = cb;
    return 7 as ReturnType<typeof setTimeout>;
  });
  const batcher = createTerminalWriteBatcher({
    write,
    delayMs: 8,
    maxBytes,
    setTimer,
    clearTimer,
  });

  return {
    batcher,
    write,
    setTimer,
    clearTimer,
    flushTimer() {
      const cb = callback;
      callback = null;
      cb?.();
    },
  };
}

describe("createTerminalWriteBatcher", () => {
  it("coalesces chunks into one terminal write", () => {
    const f = setup();

    f.batcher.write(bytes([1, 2]));
    f.batcher.write(bytes([3]));
    f.batcher.write(bytes([4, 5]));

    expect(f.setTimer).toHaveBeenCalledTimes(1);
    expect(f.write).not.toHaveBeenCalled();

    f.flushTimer();

    expect(f.write).toHaveBeenCalledTimes(1);
    expect([...f.write.mock.calls[0][0]]).toEqual([1, 2, 3, 4, 5]);
  });

  it("flushes pending chunks immediately when requested", () => {
    const f = setup();

    f.batcher.write(bytes([1]));
    f.batcher.write(bytes([2]));
    f.batcher.flush();

    expect(f.clearTimer).toHaveBeenCalledWith(7);
    expect([...f.write.mock.calls[0][0]]).toEqual([1, 2]);
    f.flushTimer();
    expect(f.write).toHaveBeenCalledTimes(1);
  });

  it("flushes immediately when the byte threshold is reached", () => {
    const f = setup(4);

    f.batcher.write(bytes([1, 2]));
    f.batcher.write(bytes([3, 4]));

    expect(f.clearTimer).toHaveBeenCalledWith(7);
    expect(f.write).toHaveBeenCalledTimes(1);
    expect([...f.write.mock.calls[0][0]]).toEqual([1, 2, 3, 4]);
  });

  it("ignores empty chunks", () => {
    const f = setup();

    f.batcher.write(bytes([]));

    expect(f.setTimer).not.toHaveBeenCalled();
    expect(f.write).not.toHaveBeenCalled();
  });

  it("drops pending chunks on dispose", () => {
    const f = setup();

    f.batcher.write(bytes([1, 2, 3]));
    f.batcher.dispose();
    f.flushTimer();
    f.batcher.write(bytes([4]));

    expect(f.clearTimer).toHaveBeenCalledWith(7);
    expect(f.write).not.toHaveBeenCalled();
    expect(f.setTimer).toHaveBeenCalledTimes(1);
  });
});
