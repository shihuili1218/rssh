import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Settings persist through the Tauri backend (invoke). Unit tests have no
// backend, so stub it to a no-op — we only assert in-memory tab ordering.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => null) }));

// The store transitively imports the AI store, which reads localStorage at
// module load (loadPos). Node has no localStorage, so give the import an
// in-memory stub. Fresh per test to avoid cross-test leakage.
beforeEach(() => {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

// Module-level $state — reset + re-import for a clean tab list per test
// (mirrors toast.test.ts).
async function loadAppModule() {
  vi.resetModules();
  return import("./app.svelte.ts");
}

const local = (id: string) => ({ id, type: "local" as const, label: id });

describe("tab MRU ordering", () => {
  it("seeds with the fixed home tab at the front", async () => {
    const app = await loadAppModule();
    expect(app.tabs().map((t) => t.id)).toEqual(["home"]);
    expect(app.activeTabId()).toBe("home");
  });

  it("inserts each new tab at the front of the session region (after home)", async () => {
    const app = await loadAppModule();
    app.addTab(local("a"));
    app.addTab(local("b"));
    app.addTab(local("c"));
    // Newest = most-recently-focused → front. Home stays pinned at index 0.
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "c", "b", "a"]);
    expect(app.activeTabId()).toBe("c");
  });

  it("brings the focused session tab to the front on activation", async () => {
    const app = await loadAppModule();
    app.addTab(local("a"));
    app.addTab(local("b"));
    app.addTab(local("c")); // [home, c, b, a]

    app.setActiveTab("a");
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "a", "c", "b"]);
    expect(app.activeTabId()).toBe("a");

    app.setActiveTab("c");
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "c", "a", "b"]);
  });

  it("activating the already-front tab is a no-op", async () => {
    const app = await loadAppModule();
    app.addTab(local("a"));
    app.addTab(local("b")); // [home, b, a]
    app.setActiveTab("b");
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "b", "a"]);
  });

  it("never reorders the fixed home tab", async () => {
    const app = await loadAppModule();
    app.addTab(local("a"));
    app.addTab(local("b")); // [home, b, a]
    app.setActiveTab("home");
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "b", "a"]);
    expect(app.activeTabId()).toBe("home");
  });
});

describe("tab drag reorder stays independent of MRU", () => {
  it("moveTab reorders without refocusing the dragged tab", async () => {
    const app = await loadAppModule();
    app.addTab(local("a"));
    app.addTab(local("b"));
    app.addTab(local("c")); // [home, c, b, a], active c

    // Drag the front tab (c, idx 1) to the end (idx 3).
    app.moveTab(1, 3);
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "b", "a", "c"]);
    // Dragging must NOT change the active tab — MRU only fires on focus.
    expect(app.activeTabId()).toBe("c");
  });
});

describe("closeTab keeps the most-recent tab active", () => {
  it("activates the next session tab after closing the active one", async () => {
    const app = await loadAppModule();
    app.addTab(local("a"));
    app.addTab(local("b"));
    app.addTab(local("c")); // [home, c, b, a], active c at front

    app.closeTab("c");
    // c was front (idx 1); the next most-recent (b) takes the front and focus.
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "b", "a"]);
    expect(app.activeTabId()).toBe("b");
  });
});

describe("MRU toggle disables reordering", () => {
  it("appends new tabs at the end and does not move on focus when off", async () => {
    const app = await loadAppModule();
    await app.setTabMru(false);

    app.addTab(local("a"));
    app.addTab(local("b"));
    app.addTab(local("c"));
    // Plain insertion order — the pre-MRU behavior.
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "a", "b", "c"]);

    app.setActiveTab("a");
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "a", "b", "c"]);
    expect(app.activeTabId()).toBe("a");
  });

  it("resumes move-to-front once re-enabled", async () => {
    const app = await loadAppModule();
    await app.setTabMru(false);
    app.addTab(local("a"));
    app.addTab(local("b")); // [home, a, b]

    await app.setTabMru(true);
    app.setActiveTab("b"); // b at idx 2 → front
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "b", "a"]);
  });

  it("defaults to enabled (move-to-front) with no setting loaded", async () => {
    const app = await loadAppModule();
    app.addTab(local("a"));
    app.addTab(local("b"));
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "b", "a"]);
  });
});
