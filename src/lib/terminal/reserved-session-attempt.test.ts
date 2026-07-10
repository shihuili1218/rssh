import { describe, expect, it, vi } from "vitest";

import { createReservedSessionAttempt } from "./reserved-session-attempt.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("ReservedSessionAttempt", () => {
  it("wires events before opening the reserved session", async () => {
    const wired = deferred<() => void>();
    const openBackend = vi.fn(async (id: string) => id);
    const attempt = createReservedSessionAttempt({
      makeId: () => "session-1",
      wireEvents: () => wired.promise,
      close: vi.fn(),
    });

    const pending = attempt.open(openBackend);
    await Promise.resolve();
    expect(openBackend).not.toHaveBeenCalled();

    wired.resolve(() => {});
    await expect(pending).resolves.toEqual({ kind: "ready", sessionId: "session-1" });
    expect(openBackend).toHaveBeenCalledWith("session-1");
  });

  it("closes both the reservation and a handle that returns after cancellation", async () => {
    const opened = deferred<string>();
    const close = vi.fn();
    const attempt = createReservedSessionAttempt({
      makeId: () => "reserved-1",
      wireEvents: async () => () => {},
      close,
    });

    const pending = attempt.open(() => opened.promise);
    await vi.waitFor(() => expect(attempt.isPending()).toBe(true));

    attempt.cancel();
    expect(close).toHaveBeenCalledWith("reserved-1");

    opened.resolve("opened-1");
    await expect(pending).resolves.toEqual({ kind: "cancelled" });
    expect(close).toHaveBeenCalledWith("opened-1");
  });

  it("destroys a wiring attempt and permanently rejects later opens", async () => {
    const wired = deferred<() => void>();
    const disposeEvents = vi.fn();
    const openBackend = vi.fn(async (id: string) => id);
    const close = vi.fn();
    const attempt = createReservedSessionAttempt({
      makeId: () => "reserved-1",
      wireEvents: () => wired.promise,
      close,
    });

    const pending = attempt.open(openBackend);
    await vi.waitFor(() => expect(attempt.isPending()).toBe(true));
    attempt.destroy();
    expect(close).toHaveBeenCalledWith("reserved-1");

    wired.resolve(disposeEvents);
    await expect(pending).resolves.toEqual({ kind: "cancelled" });
    expect(disposeEvents).toHaveBeenCalledOnce();
    expect(openBackend).not.toHaveBeenCalled();

    await expect(attempt.open(openBackend)).resolves.toEqual({ kind: "cancelled" });
    expect(openBackend).not.toHaveBeenCalled();
  });

  it("accepts events only for the current reservation", async () => {
    const opened = deferred<string>();
    const attempt = createReservedSessionAttempt({
      makeId: () => "reserved-1",
      wireEvents: async () => () => {},
      close: vi.fn(),
    });

    const pending = attempt.open(() => opened.promise);
    await vi.waitFor(() => expect(attempt.isPending()).toBe(true));
    expect(attempt.accepts("reserved-1")).toBe(true);
    expect(attempt.accepts("stale")).toBe(false);

    attempt.cancel();
    expect(attempt.accepts("reserved-1")).toBe(false);
    opened.resolve("opened-1");
    await pending;
  });

  it("disposes failed attempts and permits a clean retry", async () => {
    const disposeFirst = vi.fn();
    const disposeSecond = vi.fn();
    const disposers = [disposeFirst, disposeSecond];
    let nextId = 0;
    const attempt = createReservedSessionAttempt({
      makeId: () => `reserved-${++nextId}`,
      wireEvents: async () => disposers.shift()!,
      close: vi.fn(async () => {}),
    });
    const failure = new Error("open failed");

    await expect(attempt.open(async () => { throw failure; })).rejects.toBe(failure);
    expect(disposeFirst).toHaveBeenCalledOnce();
    expect(attempt.isPending()).toBe(false);
    expect(attempt.accepts("reserved-1")).toBe(false);

    await expect(attempt.open(async (id) => id)).resolves.toEqual({
      kind: "ready",
      sessionId: "reserved-2",
    });
    expect(disposeSecond).not.toHaveBeenCalled();
  });

  it("best-effort closes the reserved id when opening rejects", async () => {
    const close = vi.fn(async () => {});
    const attempt = createReservedSessionAttempt({
      makeId: () => "reserved-1",
      wireEvents: async () => () => {},
      close,
    });

    await expect(attempt.open(async () => {
      throw new Error("response lost");
    })).rejects.toThrow("response lost");

    expect(close).toHaveBeenCalledWith("reserved-1");
  });

  it("rejects a backend id that differs from the canonical reservation", async () => {
    const close = vi.fn();
    const disposeEvents = vi.fn();
    const attempt = createReservedSessionAttempt({
      makeId: () => "reserved-1",
      wireEvents: async () => disposeEvents,
      close,
    });

    await expect(attempt.open(async () => "opened-1")).rejects.toThrow(
      "backend returned a different session id",
    );
    expect(disposeEvents).toHaveBeenCalledOnce();
    expect(attempt.accepts("reserved-1")).toBe(false);
    expect(attempt.accepts("opened-1")).toBe(false);
    expect(close).toHaveBeenCalledWith("reserved-1");
    expect(close).toHaveBeenCalledWith("opened-1");
  });

  it("keeps a newer ready attempt when the superseded open returns late", async () => {
    const firstOpened = deferred<string>();
    const close = vi.fn();
    let nextId = 0;
    const attempt = createReservedSessionAttempt({
      makeId: () => `reserved-${++nextId}`,
      wireEvents: async () => () => {},
      close,
    });

    const first = attempt.open(() => firstOpened.promise);
    await vi.waitFor(() => expect(attempt.accepts("reserved-1")).toBe(true));

    await expect(attempt.open(async (id) => id)).resolves.toEqual({
      kind: "ready",
      sessionId: "reserved-2",
    });
    expect(attempt.accepts("reserved-2")).toBe(true);

    firstOpened.resolve("opened-1");
    await expect(first).resolves.toEqual({ kind: "cancelled" });
    expect(close).toHaveBeenCalledWith("opened-1");
    expect(close).not.toHaveBeenCalledWith("reserved-2");
    expect(attempt.accepts("reserved-2")).toBe(true);
  });

  it("does not hide an open error behind a close that never settles", async () => {
    const attempt = createReservedSessionAttempt({
      makeId: () => "reserved-1",
      wireEvents: async () => () => {},
      close: () => new Promise<void>(() => {}),
    });

    await expect(attempt.open(async () => {
      throw new Error("open failed");
    })).rejects.toThrow("open failed");
  });
});
