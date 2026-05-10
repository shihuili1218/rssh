import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

async function loadToastModule() {
  vi.resetModules();
  return import("./toast.svelte.ts");
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("toast store", () => {
  it("adds toast items with incremental ids and expected kinds", async () => {
    const { toast, toasts } = await loadToastModule();

    toast.error("boom");
    toast.success("done");
    toast.info("heads up");

    expect(toasts()).toEqual([
      { id: 0, kind: "error", message: "boom" },
      { id: 1, kind: "success", message: "done" },
      { id: 2, kind: "info", message: "heads up" },
    ]);
  });

  it("dismisses only the requested toast", async () => {
    const { dismiss, toast, toasts } = await loadToastModule();

    toast.error("boom");
    toast.success("done");
    dismiss(0);

    expect(toasts()).toEqual([{ id: 1, kind: "success", message: "done" }]);
  });

  it("auto-removes toast items after the default TTL", async () => {
    const { toast, toasts } = await loadToastModule();

    toast.info("soon gone");
    vi.advanceTimersByTime(3999);
    expect(toasts()).toHaveLength(1);

    vi.advanceTimersByTime(1);
    expect(toasts()).toEqual([]);
  });

  it("ignores dismiss calls for unknown ids", async () => {
    const { dismiss, toast, toasts } = await loadToastModule();

    toast.info("keep me");
    dismiss(999);

    expect(toasts()).toEqual([{ id: 0, kind: "info", message: "keep me" }]);
  });
});
