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

describe("recent Home connections", () => {
  it("records every saved or discovered GUI connection through addTab", async () => {
    const app = await loadAppModule();

    app.addTab({ id: "ssh-tab", type: "ssh", label: "SSH", meta: { profileId: "p1" } });
    app.addTab({ id: "fwd-tab", type: "forward", label: "Forward", meta: { forwardId: "f1" } });
    app.addTab({ id: "serial-tab", type: "serial", label: "Serial", meta: { serialProfileId: "s1" } });
    app.addTab({ id: "telnet-tab", type: "telnet", label: "Telnet", meta: { profileId: "t1" } });
    app.addTab({
      id: "docker-tab",
      type: "docker_exec",
      label: "Docker",
      meta: { sourceId: "src", dynamicTargetId: "docker_exec:ctx:container" },
    });
    app.addTab({
      id: "k8s-tab",
      type: "kubectl_exec",
      label: "Kubernetes",
      meta: { sourceId: "src", dynamicTargetId: "kubectl_exec:ctx:ns:pod:container" },
    });

    expect(app.recentHomeItemIds()).toEqual([
      "dynamic:src:kubectl_exec:ctx:ns:pod:container",
      "dynamic:src:docker_exec:ctx:container",
      "telnet:t1",
      "serial:s1",
      "forward:f1",
      "ssh:p1",
    ]);
  });

  it("moves repeated connections to the front and ignores non-Home tabs", async () => {
    const app = await loadAppModule();

    app.addTab({ id: "ssh-a", type: "ssh", label: "A", meta: { profileId: "a" } });
    app.addTab({ id: "ssh-b", type: "ssh", label: "B", meta: { profileId: "b" } });
    app.addTab(local("local"));
    app.addTab({ id: "edit", type: "edit", label: "Edit" });
    app.addTab({ id: "ssh-a-2", type: "ssh", label: "A", meta: { profileId: "a" } });

    expect(app.recentHomeItemIds()).toEqual(["ssh:a", "ssh:b"]);
    expect(JSON.parse(localStorage.getItem("home.recent_items.v1") ?? "[]")).toEqual([
      "ssh:a",
      "ssh:b",
    ]);
  });

  it("loads valid persisted ids and falls back safely from corrupt storage", async () => {
    localStorage.setItem("home.recent_items.v1", JSON.stringify(["ssh:p2", "ssh:p1", "ssh:p2"]));
    let app = await loadAppModule();
    expect(app.recentHomeItemIds()).toEqual(["ssh:p2", "ssh:p1"]);

    localStorage.setItem("home.recent_items.v1", "not-json");
    app = await loadAppModule();
    expect(app.recentHomeItemIds()).toEqual([]);
  });

  it("merges a newer record written by another window before persisting", async () => {
    const app = await loadAppModule();
    app.addTab({ id: "ssh-a", type: "ssh", label: "A", meta: { profileId: "a" } });

    localStorage.setItem("home.recent_items.v1", JSON.stringify(["ssh:b", "ssh:a"]));
    app.addTab({ id: "ssh-c", type: "ssh", label: "C", meta: { profileId: "c" } });

    expect(app.recentHomeItemIds()).toEqual(["ssh:c", "ssh:b", "ssh:a"]);
  });

  it("reacts to recent records written by another window", async () => {
    let onStorage: ((event: StorageEvent) => void) | undefined;
    vi.stubGlobal("window", {
      addEventListener: (type: string, listener: (event: StorageEvent) => void) => {
        if (type === "storage") onStorage = listener;
      },
    });
    const app = await loadAppModule();

    onStorage?.({
      key: "home.recent_items.v1",
      newValue: JSON.stringify(["ssh:remote", "forward:f1"]),
    } as StorageEvent);

    expect(app.recentHomeItemIds()).toEqual(["ssh:remote", "forward:f1"]);
  });

  it("bounds transient discovery history instead of growing forever", async () => {
    const app = await loadAppModule();
    for (let index = 0; index < 260; index += 1) {
      app.addTab({
        id: `docker-tab-${index}`,
        type: "docker_exec",
        label: `Container ${index}`,
        meta: { sourceId: "src", dynamicTargetId: `docker_exec:ctx:${index}` },
      });
    }

    expect(app.recentHomeItemIds()).toHaveLength(256);
    expect(app.recentHomeItemIds()[0]).toBe("dynamic:src:docker_exec:ctx:259");
    expect(app.recentHomeItemIds()[255]).toBe("dynamic:src:docker_exec:ctx:4");
  });
});

describe("tab MRU ordering", () => {
  it("seeds with the fixed home tab at the front", async () => {
    const app = await loadAppModule();
    expect(app.tabs().map((t) => t.id)).toEqual(["home"]);
    expect(app.activeTabId()).toBe("home");
  });

  it("inserts each new tab at the front of the session region (after home)", async () => {
    const app = await loadAppModule();
    await app.setTabMru(true);
    app.addTab(local("a"));
    app.addTab(local("b"));
    app.addTab(local("c"));
    // Newest = most-recently-focused → front. Home stays pinned at index 0.
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "c", "b", "a"]);
    expect(app.activeTabId()).toBe("c");
  });

  it("brings the focused session tab to the front on activation", async () => {
    const app = await loadAppModule();
    await app.setTabMru(true);
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
    await app.setTabMru(true);
    app.addTab(local("a"));
    app.addTab(local("b")); // [home, b, a]
    app.setActiveTab("b");
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "b", "a"]);
  });

  it("never reorders the fixed home tab", async () => {
    const app = await loadAppModule();
    await app.setTabMru(true);
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
    await app.setTabMru(true);
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
    await app.setTabMru(true);
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

  it("defaults to disabled (insertion order) with no setting loaded", async () => {
    const app = await loadAppModule();
    app.addTab(local("a"));
    app.addTab(local("b"));
    expect(app.tabs().map((t) => t.id)).toEqual(["home", "a", "b"]);
  });
});

describe("connection editor navigation", () => {
  it("opens a new connection with a selectable SSH type by default", async () => {
    const app = await loadAppModule();

    app.openConnectionCreate();

    expect(app.settingsPage()).toBe("connection-edit");
    expect(app.connectionEditorIntent()).toEqual({ mode: "create", kind: "ssh", sourceId: null });
    expect(app.connectionTypeLocked()).toBe(false);
  });

  it("locks the source type while editing an existing connection", async () => {
    const app = await loadAppModule();

    app.openConnectionEdit("serial", "serial-1");

    expect(app.settingsPage()).toBe("connection-edit");
    expect(app.connectionEditorIntent()).toEqual({ mode: "edit", kind: "serial", sourceId: "serial-1" });
    expect(app.connectionTypeLocked()).toBe(true);
    expect(app.connectionUpdateId()).toBe("serial-1");
  });

  it("locks the source type while copying without turning the source into an edit target", async () => {
    const app = await loadAppModule();

    app.openConnectionCopy("telnet", "telnet-1");

    expect(app.settingsPage()).toBe("connection-edit");
    expect(app.connectionEditorIntent()).toEqual({ mode: "copy", kind: "telnet", sourceId: "telnet-1" });
    expect(app.connectionTypeLocked()).toBe(true);
    expect(app.connectionUpdateId()).toBeNull();
  });

  it("returns from the connection editor to the unified list", async () => {
    const app = await loadAppModule();
    app.openConnectionCreate("forward");

    app.settingsBack();

    expect(app.settingsPage()).toBe("connections");
  });
});

describe("connectTelnetProfile", () => {
  it("carries the explicit echo mode into the terminal tab", async () => {
    const app = await loadAppModule();

    app.connectTelnetProfile({
      id: "telnet-1",
      name: "switch",
      host: "192.0.2.10",
      port: 23,
      input_newline: "crlf",
      output_newline: "raw",
      local_echo: false,
      echo_mode: "off",
      backspace: "del",
      login_script: "send super-secret",
      group_id: null,
    });

    expect(app.tabs()[1].meta?.echo_mode).toBe("off");
    expect(app.tabs()[1].meta?.profileId).toBe("telnet-1");
    expect(app.tabs()[1].meta?.login_script).toBeUndefined();
  });

  it("maps the legacy local_echo flag when echo_mode is absent", async () => {
    const app = await loadAppModule();
    const legacy = {
      id: "telnet-legacy",
      name: "legacy switch",
      host: "192.0.2.11",
      port: 23,
      input_newline: "crlf",
      output_newline: "raw",
      local_echo: true,
      backspace: "del",
      login_script: "",
      group_id: null,
    };

    app.connectTelnetProfile(legacy);

    expect(app.tabs()[1].meta?.echo_mode).toBe("on");
  });
});
