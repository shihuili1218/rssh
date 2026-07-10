import { describe, expect, it } from "vitest";

import { createHomeRefresh } from "./home-refresh.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

describe("createHomeRefresh", () => {
  it("publishes static data before starting dynamic discovery", async () => {
    const events: string[] = [];
    let resolveStatic!: (value: string) => void;
    const staticData = new Promise<string>((resolve) => {
      resolveStatic = resolve;
    });
    const refresher = createHomeRefresh({
      loadStatic: () => {
        events.push("load static");
        return staticData;
      },
      loadDynamic: async () => {
        events.push("load dynamic");
        return "dynamic";
      },
      applyStatic: (value) => events.push(`apply ${value}`),
      applyDynamic: (value) => events.push(`apply ${value}`),
    });

    const pending = refresher.refresh();
    expect(events).toEqual(["load static"]);

    resolveStatic("static");
    await pending;

    expect(events).toEqual([
      "load static",
      "apply static",
      "load dynamic",
      "apply dynamic",
    ]);
  });

  it("does not let an older dynamic result overwrite a newer refresh", async () => {
    const firstDynamic = deferred<string>();
    const secondDynamic = deferred<string>();
    const pendingDynamic = [firstDynamic, secondDynamic];
    const applied: string[] = [];
    let dynamicIndex = 0;
    const refresher = createHomeRefresh({
      loadStatic: async () => "static",
      loadDynamic: () => pendingDynamic[dynamicIndex++].promise,
      applyStatic: () => {},
      applyDynamic: (value) => applied.push(value),
    });

    const first = refresher.refresh();
    await Promise.resolve();
    const second = refresher.refresh();
    await Promise.resolve();

    secondDynamic.resolve("new");
    await second;
    firstDynamic.resolve("old");
    await first;

    expect(applied).toEqual(["new"]);
  });

  it("does not publish a pending result after refresh is cancelled", async () => {
    const dynamic = deferred<string>();
    const applied: string[] = [];
    const refresher = createHomeRefresh({
      loadStatic: async () => "static",
      loadDynamic: () => dynamic.promise,
      applyStatic: () => {},
      applyDynamic: (value) => applied.push(value),
    });

    const pending = refresher.refresh();
    await Promise.resolve();
    refresher.cancel();
    dynamic.resolve("stale");
    await pending;

    expect(applied).toEqual([]);
  });

  it("reports a failure from the current refresh", async () => {
    const failure = new Error("profiles unavailable");
    const errors: unknown[] = [];
    const refresher = createHomeRefresh({
      loadStatic: async () => { throw failure; },
      loadDynamic: async () => "dynamic",
      applyStatic: () => {},
      applyDynamic: () => {},
      onError: (error) => errors.push(error),
    });

    await expect(refresher.refresh()).resolves.toBeUndefined();
    expect(errors).toEqual([failure]);
  });

  it("does not report a stale or cancelled failure", async () => {
    const first = deferred<string>();
    const second = deferred<string>();
    const cancelledLoad = deferred<string>();
    const loads = [first, second, cancelledLoad];
    const errors: unknown[] = [];
    let loadIndex = 0;
    const refresher = createHomeRefresh({
      loadStatic: () => loads[loadIndex++].promise,
      loadDynamic: async () => "dynamic",
      applyStatic: () => {},
      applyDynamic: () => {},
      onError: (error) => errors.push(error),
    });

    const stale = refresher.refresh();
    const current = refresher.refresh();
    first.reject(new Error("stale"));
    second.resolve("current");
    await Promise.all([stale, current]);

    const cancelled = refresher.refresh();
    refresher.cancel();
    cancelledLoad.reject(new Error("cancelled"));
    await cancelled;

    expect(errors).toEqual([]);
  });
});
