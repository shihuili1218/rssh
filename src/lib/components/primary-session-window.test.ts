import { describe, expect, it, vi } from "vitest";

import { initializePrimarySessionWindow } from "./primary-session-window.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("initializePrimarySessionWindow", () => {
  it("releases resource panes before reading and applying auto-open", async () => {
    const reconcile = deferred<void>();
    const events: string[] = [];
    const initializing = initializePrimarySessionWindow({
      reconcile: () => {
        events.push("reconcile");
        return reconcile.promise;
      },
      allowResourcePanes: () => events.push("release"),
      loadAutoOpenLocal: async () => {
        events.push("load auto-open");
        return true;
      },
      openLocal: () => events.push("open local"),
    });

    await Promise.resolve();
    expect(events).toEqual(["reconcile"]);

    reconcile.resolve();
    await initializing;
    expect(events).toEqual([
      "reconcile",
      "release",
      "load auto-open",
      "open local",
    ]);
  });

  it("releases resource panes and continues when reconciliation fails", async () => {
    const events: string[] = [];

    await expect(initializePrimarySessionWindow({
      reconcile: async () => {
        events.push("reconcile");
        throw new Error("backend unavailable");
      },
      allowResourcePanes: () => events.push("release"),
      loadAutoOpenLocal: async () => {
        events.push("load auto-open");
        return false;
      },
      openLocal: () => events.push("open local"),
    })).resolves.toBeUndefined();

    expect(events).toEqual(["reconcile", "release", "load auto-open"]);
  });

  it("does nothing after its owner is cancelled", async () => {
    const reconcile = deferred<void>();
    const controller = new AbortController();
    const events: string[] = [];
    const initializing = initializePrimarySessionWindow({
      signal: controller.signal,
      reconcile: () => reconcile.promise,
      allowResourcePanes: () => events.push("release"),
      loadAutoOpenLocal: async () => true,
      openLocal: () => events.push("open local"),
    });

    controller.abort();
    reconcile.resolve();
    await initializing;

    expect(events).toEqual([]);
  });

  it("does not auto-open after cancellation during the settings read", async () => {
    const autoOpen = deferred<boolean>();
    const controller = new AbortController();
    const events: string[] = [];
    const initializing = initializePrimarySessionWindow({
      signal: controller.signal,
      reconcile: async () => {},
      allowResourcePanes: () => events.push("release"),
      loadAutoOpenLocal: () => autoOpen.promise,
      openLocal: () => events.push("open local"),
    });

    await vi.waitFor(() => expect(events).toEqual(["release"]));
    controller.abort();
    autoOpen.resolve(true);
    await initializing;

    expect(events).toEqual(["release"]);
  });
});
