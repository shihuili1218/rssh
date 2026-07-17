import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn(async (_command: string, _args?: unknown): Promise<unknown> => null);
const unlistenMock = vi.fn();
type ListenMock = (
  event: string,
  handler: (event: { payload: unknown }) => void,
) => Promise<() => void>;
const listenMock = vi.fn<ListenMock>(async () => unlistenMock);

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(null);
  unlistenMock.mockReset();
  listenMock.mockReset();
  listenMock.mockResolvedValue(unlistenMock);
  vi.stubGlobal("localStorage", {
    getItem: () => null,
    setItem: vi.fn(),
  });
  vi.stubGlobal("window", globalThis);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("panel visibility", () => {
  it("keeps each tab's open state independent", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");

    ai.openPanel("tab-a");
    expect(ai.isOpen("tab-a")).toBe(true);
    expect(ai.isOpen("tab-b")).toBe(false);

    ai.openPanel("tab-b");
    expect(ai.isOpen("tab-a")).toBe(true);
    expect(ai.isOpen("tab-b")).toBe(true);

    ai.closePanel("tab-a");
    expect(ai.isOpen("tab-a")).toBe(false);
    expect(ai.isOpen("tab-b")).toBe(true);
  });

  it("keeps each tab's width independent while the panel is hidden", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.openPanel("tab-a");
    ai.openPanel("tab-b");

    ai.setPanelWidth("tab-a", 520);
    ai.setPanelWidth("tab-b", 360);
    ai.closePanel("tab-a");

    expect(ai.panelWidth("tab-a")).toBe(520);
    expect(ai.panelWidth("tab-b")).toBe(360);

    ai.setPanelWidth("tab-a", null);
    expect(ai.panelWidth("tab-a")).toBeNull();
    expect(ai.panelWidth("tab-b")).toBe(360);
  });

  it("discards only the closed tab's panel state", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.openPanel("tab-a");
    ai.openPanel("tab-b");
    ai.setPanelWidth("tab-a", 520);
    ai.setPanelWidth("tab-b", 360);
    ai.prefillInput("tab-a", "from A");
    ai.prefillInput("tab-b", "from B");

    ai.discardPanelState("tab-a");

    expect(ai.isOpen("tab-a")).toBe(false);
    expect(ai.panelWidth("tab-a")).toBeNull();
    expect(ai.pendingPrefill("tab-a")).toBeNull();
    expect(ai.isOpen("tab-b")).toBe(true);
    expect(ai.panelWidth("tab-b")).toBe(360);
    expect(ai.pendingPrefill("tab-b")?.text).toBe("from B");
  });
});

describe("tab lifecycle", () => {
  const args = {
    tabId: "tab-a",
    targetKind: "local" as const,
    targetId: "pty-old",
    skill: "general",
    provider: "openai" as const,
    model: "gpt-test",
  };
  const info = {
    tab_id: "tab-a",
    target_id: "pty-old",
    skill: "general",
    model: "gpt-test",
    provider: "openai" as const,
    conversation_id: "conversation-a",
  };

  it("cancels a backend session that finishes starting after its tab closes", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");

    let resolveStart!: (value: typeof info) => void;
    const pendingStart = new Promise<typeof info>((resolve) => { resolveStart = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return pendingStart;
      return null;
    });

    const launch = ai.startSession(args);
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_start",
      expect.objectContaining({ tabId: "tab-a" }),
    ));

    await ai.disposeTab("tab-a");
    resolveStart(info);

    await expect(launch).rejects.toThrow(/closed/i);
    expect(ai.sessionForTab("tab-a")).toBeUndefined();
    expect(invokeMock).toHaveBeenCalledWith("ai_session_stop", { tabId: "tab-a" });
  });

  it("tears down listeners that resolve after tab disposal", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });

    const lateUnlisten = vi.fn();
    let lateHandler: ((event: { payload: { text: string } }) => void) | undefined;
    let resolveListen!: (value: () => void) => void;
    const pendingListen = new Promise<() => void>((resolve) => { resolveListen = resolve; });
    listenMock.mockImplementationOnce((_event, handler) => {
      lateHandler = handler as typeof lateHandler;
      return pendingListen;
    });
    listenMock.mockImplementation(async () => () => {});

    const launch = ai.startSession(args);
    await vi.waitFor(() => expect(listenMock).toHaveBeenCalledTimes(1));

    await ai.disposeTab("tab-a");
    resolveListen(lateUnlisten);

    await expect(launch).rejects.toThrow(/closed/i);
    lateHandler?.({ payload: { text: "late message" } });
    expect(lateUnlisten).toHaveBeenCalledOnce();
    expect(ai.sessionForTab("tab-a")).toBeUndefined();
    expect(ai.chatItems("tab-a")).toEqual([]);
  });
});

describe("executeCommand", () => {
  it("does not write a command when termination wins the listener-registration race", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    const proposed = {
      id: "cmd-race",
      tool_call_id: "tool-race",
      cmd: "echo unsafe",
      full_cmd: "echo unsafe; printf sentinel",
      sentinel: "sentinel",
      explain: "",
      side_effect: "none",
      timeout_s: 30,
    };

    const lateUnlisten = vi.fn();
    let resolveListen!: (value: () => void) => void;
    listenMock.mockImplementationOnce(() => new Promise<() => void>((resolve) => {
      resolveListen = resolve;
    }));

    const running = ai.executeCommand("tab-1", proposed, "local", "session-1");
    await vi.waitFor(() => expect(ai.isCommandRunning("tool-race")).toBe(true));
    await ai.terminateCommand("tool-race");
    resolveListen(lateUnlisten);
    await running;

    const fullCommandData = Array.from(new TextEncoder().encode(`${proposed.full_cmd}\r`));
    expect(lateUnlisten).toHaveBeenCalledOnce();
    expect(invokeMock).not.toHaveBeenCalledWith("pty_write", {
      sessionId: "session-1",
      data: fullCommandData,
    });
    expect(ai.isCommandRunning("tool-race")).toBe(false);
  });

  it("lets the Telnet backend append the profile's configured newline", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    const proposed = {
      id: "cmd-1",
      tool_call_id: "tool-1",
      cmd: "show version",
      full_cmd: "show version",
      sentinel: "unused-for-telnet",
      explain: "",
      side_effect: "none",
      timeout_s: 30,
    };

    const running = ai.executeCommand("tab-1", proposed, "telnet", "session-1");
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("telnet_write_line", {
        sessionId: "session-1",
        text: "show version",
      });
    });
    await ai.submitCommand("tool-1");
    await running;
  });
});
