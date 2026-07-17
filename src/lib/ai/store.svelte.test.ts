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
  vi.useRealTimers();
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

  it("returns a closed panel to its first-open conversation state", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    ai.setPanelWidth("tab-a", 520);
    ai.prefillInput("tab-a", "stale draft");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });

    await ai.startSession(args);
    const userListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:user_message:tab-a",
    )?.[1];
    const assistantStartListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_message_start:tab-a",
    )?.[1];
    const assistantEndListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_message_end:tab-a",
    )?.[1];
    expect(userListener).toBeDefined();
    userListener?.({ payload: { text: "old conversation" } });
    assistantStartListener?.({ payload: { id: "reply-a" } });
    assistantEndListener?.({
      payload: { id: "reply-a", text: "old answer", tokens_in: 12, tokens_out: 8 },
    });
    expect(ai.chatItems("tab-a")).toHaveLength(2);
    expect(ai.tokenUsage("tab-a")).toEqual({ tokens_in: 12, tokens_out: 8 });

    const closed = ai.closePanel("tab-a");

    expect(ai.isOpen("tab-a")).toBe(false);
    expect(ai.sessionForTab("tab-a")).toBeUndefined();
    expect(ai.chatItems("tab-a")).toEqual([]);
    expect(ai.tokenUsage("tab-a")).toEqual({ tokens_in: 0, tokens_out: 0 });
    expect(ai.pendingPrefill("tab-a")).toBeNull();
    expect(ai.panelWidth("tab-a")).toBe(520);
    await closed;
    const saveArgs = invokeMock.mock.calls.find(
      ([command]) => command === "ai_conversation_save_timeline",
    )?.[1] as { id: string; timeline: string } | undefined;
    expect(saveArgs?.id).toBe("conversation-a");
    expect(JSON.parse(saveArgs?.timeline ?? "[]")).toEqual([
      { kind: "user", text: "old conversation", at: expect.any(Number) },
      {
        kind: "assistant",
        id: "reply-a",
        text: "old answer",
        at: expect.any(Number),
        streaming: false,
        cancelled: false,
      },
    ]);
    expect(invokeMock).toHaveBeenCalledWith("ai_session_stop", { tabId: "tab-a" });

    ai.openPanel("tab-a");
    expect(ai.sessionForTab("tab-a")).toBeUndefined();
    expect(ai.chatItems("tab-a")).toEqual([]);
  });

  it("resets only the tab whose panel was explicitly closed", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.activateTab("tab-b");
    ai.openPanel("tab-a");
    ai.openPanel("tab-b");
    const infoB = {
      ...info,
      tab_id: "tab-b",
      target_id: "pty-b",
      conversation_id: "conversation-b",
    };
    invokeMock.mockImplementation(async (command: string, callArgs?: unknown) => {
      if (command !== "ai_session_start") return null;
      return (callArgs as { tabId: string }).tabId === "tab-a" ? info : infoB;
    });

    await ai.startSession(args);
    await ai.startSession({ ...args, tabId: "tab-b", targetId: "pty-b" });
    const userListenerB = listenMock.mock.calls.find(
      ([event]) => event === "ai:user_message:tab-b",
    )?.[1];
    userListenerB?.({ payload: { text: "keep B" } });

    await ai.closePanel("tab-a");

    expect(ai.sessionForTab("tab-a")).toBeUndefined();
    expect(ai.isOpen("tab-a")).toBe(false);
    expect(ai.sessionForTab("tab-b")).toEqual(infoB);
    expect(ai.isOpen("tab-b")).toBe(true);
    expect(ai.chatItems("tab-b")).toEqual([
      { kind: "user", text: "keep B", at: expect.any(Number) },
    ]);
    expect(invokeMock).not.toHaveBeenCalledWith("ai_session_stop", { tabId: "tab-b" });
  });

  it("waits for the old actor to stop before starting after a rapid reopen", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");

    const nextInfo = {
      ...info,
      target_id: "pty-new",
      conversation_id: "conversation-b",
    };
    let startCount = 0;
    let resolveStop!: () => void;
    const pendingStop = new Promise<void>((resolve) => { resolveStop = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") {
        startCount += 1;
        return startCount === 1 ? info : nextInfo;
      }
      if (command === "ai_session_stop") return pendingStop;
      return null;
    });

    await ai.startSession(args);
    const closing = ai.closePanel("tab-a");
    ai.openPanel("tab-a");
    const restarting = ai.startSession({ ...args, targetId: "pty-new" });

    await Promise.resolve();
    expect(startCount).toBe(1);

    resolveStop();
    await closing;
    await expect(restarting).resolves.toEqual(nextInfo);
    expect(startCount).toBe(2);
  });

  it("waits for an in-flight timeline save before finishing panel close", async () => {
    vi.useFakeTimers();
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");

    let resolveSave!: () => void;
    const pendingSave = new Promise<void>((resolve) => { resolveSave = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_conversation_save_timeline") return pendingSave;
      return null;
    });

    await ai.startSession(args);
    const userListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:user_message:tab-a",
    )?.[1];
    userListener?.({ payload: { text: "persist me" } });
    await vi.advanceTimersByTimeAsync(300);
    expect(invokeMock).toHaveBeenCalledWith(
      "ai_conversation_save_timeline",
      expect.objectContaining({ id: "conversation-a" }),
    );

    let closeFinished = false;
    const closing = ai.closePanel("tab-a").then(() => { closeFinished = true; });
    await vi.advanceTimersByTimeAsync(0);
    expect(closeFinished).toBe(false);

    resolveSave();
    await closing;
    expect(closeFinished).toBe(true);
  });

  it("flushes the latest streamed text even after the debounce save already finished", async () => {
    vi.useFakeTimers();
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });

    await ai.startSession(args);
    const assistantStartListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_message_start:tab-a",
    )?.[1];
    const assistantDeltaListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_delta:tab-a",
    )?.[1];
    assistantStartListener?.({ payload: { id: "reply-a" } });
    await vi.advanceTimersByTimeAsync(300);
    assistantDeltaListener?.({ payload: { id: "reply-a", text: "latest partial reply" } });

    await ai.closePanel("tab-a");

    const saves = invokeMock.mock.calls.filter(
      ([command]) => command === "ai_conversation_save_timeline",
    );
    expect(saves).toHaveLength(2);
    const finalArgs = saves[saves.length - 1]?.[1] as { id: string; timeline: string };
    expect(JSON.parse(finalArgs.timeline)).toEqual([
      {
        kind: "assistant",
        id: "reply-a",
        text: "latest partial reply",
        at: expect.any(Number),
        streaming: true,
      },
    ]);
  });

  it("waits for an abandoned launch to clean up before restarting", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");

    const nextInfo = {
      ...info,
      target_id: "pty-new",
      conversation_id: "conversation-b",
    };
    let startCount = 0;
    let resolveFirstStart!: (value: typeof info) => void;
    const pendingFirstStart = new Promise<typeof info>((resolve) => {
      resolveFirstStart = resolve;
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") {
        startCount += 1;
        return startCount === 1 ? pendingFirstStart : nextInfo;
      }
      return null;
    });

    const firstLaunch = ai.startSession(args);
    await vi.waitFor(() => expect(startCount).toBe(1));
    const closing = ai.closePanel("tab-a");
    ai.openPanel("tab-a");
    const restarting = ai.startSession({ ...args, targetId: "pty-new" });

    await Promise.resolve();
    expect(startCount).toBe(1);

    resolveFirstStart(info);
    await expect(firstLaunch).rejects.toThrow(/closed/i);
    await closing;
    await expect(restarting).resolves.toEqual(nextInfo);
    const lifecycleCalls = invokeMock.mock.calls.map(([command]) => command).filter(
      (command) => command === "ai_session_start" || command === "ai_session_stop",
    );
    expect(lifecycleCalls[0]).toBe("ai_session_start");
    expect(lifecycleCalls[lifecycleCalls.length - 1]).toBe("ai_session_start");
    expect(lifecycleCalls.slice(1, -1).every((command) => command === "ai_session_stop"))
      .toBe(true);
  });

  it("waits for every concurrent abandoned launch before restarting", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");

    const nextInfo = {
      ...info,
      target_id: "pty-new",
      conversation_id: "conversation-c",
    };
    let startCount = 0;
    let resolveFirst!: (value: typeof info) => void;
    let resolveSecond!: (value: typeof info) => void;
    const firstPending = new Promise<typeof info>((resolve) => { resolveFirst = resolve; });
    const secondPending = new Promise<typeof info>((resolve) => { resolveSecond = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command !== "ai_session_start") return null;
      startCount += 1;
      if (startCount === 1) return firstPending;
      if (startCount === 2) return secondPending;
      return nextInfo;
    });

    const firstLaunch = ai.startSession(args);
    const secondLaunch = ai.startSession({ ...args, targetId: "pty-second" });
    await vi.waitFor(() => expect(startCount).toBe(2));

    let closeFinished = false;
    const closing = ai.closePanel("tab-a").then(() => { closeFinished = true; });
    ai.openPanel("tab-a");
    const restarting = ai.startSession({ ...args, targetId: "pty-new" });

    resolveSecond({ ...info, target_id: "pty-second", conversation_id: "conversation-b" });
    await expect(secondLaunch).rejects.toThrow(/closed/i);
    await Promise.resolve();
    await Promise.resolve();
    const closeFinishedBeforeFirstLaunch = closeFinished;
    const startCountBeforeFirstLaunch = startCount;

    resolveFirst(info);
    await expect(firstLaunch).rejects.toThrow(/closed/i);
    await closing;
    await expect(restarting).resolves.toEqual(nextInfo);

    expect(closeFinishedBeforeFirstLaunch).toBe(false);
    expect(startCountBeforeFirstLaunch).toBe(2);
    expect(startCount).toBe(3);
  });

  it("rejects a user action captured before close even after the panel reopens", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    const lease = ai.captureSessionLease("tab-a");

    await ai.closePanel("tab-a");
    ai.openPanel("tab-a");

    await expect(ai.startSession({ ...args, lease })).rejects.toThrow(/closed/i);
    expect(invokeMock).not.toHaveBeenCalledWith(
      "ai_session_start",
      expect.objectContaining({ tabId: "tab-a" }),
    );
  });

  it("does not let stale UI actions mutate the replacement session", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    const staleLease = ai.captureSessionLease("tab-a");

    await ai.closePanel("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });
    await ai.startSession(args);

    await expect(ai.sendMessage("tab-a", "stale message", staleLease)).rejects.toThrow(/closed/i);
    await expect(ai.cancelStream("tab-a", staleLease)).rejects.toThrow(/closed/i);
    await expect(ai.clearContext("tab-a", staleLease)).rejects.toThrow(/closed/i);
    await expect(ai.rebindTarget("tab-a", "local", "pty-stale", staleLease))
      .rejects.toThrow(/closed/i);

    const staleCommands = new Set([
      "ai_user_message",
      "ai_cancel_stream",
      "ai_session_clear_context",
      "ai_session_rebind_target",
    ]);
    expect(invokeMock.mock.calls.some(([command]) => staleCommands.has(command))).toBe(false);
    expect(ai.sessionForTab("tab-a")).toEqual(info);
  });

  it("does not let a late rebind overwrite the replacement session cache", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");

    const replacement = {
      ...info,
      target_id: "pty-new",
      conversation_id: "conversation-b",
    };
    let startCount = 0;
    let resolveRebind!: () => void;
    const pendingRebind = new Promise<void>((resolve) => { resolveRebind = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") {
        startCount += 1;
        return startCount === 1 ? info : replacement;
      }
      if (command === "ai_session_rebind_target") return pendingRebind;
      return null;
    });

    await ai.startSession(args);
    const staleLease = ai.captureSessionLease("tab-a");
    const staleRebind = ai.rebindTarget("tab-a", "local", "pty-stale", staleLease);
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_rebind_target",
      expect.objectContaining({ tabId: "tab-a", conversationId: "conversation-a" }),
    ));

    await ai.closePanel("tab-a");
    ai.openPanel("tab-a");
    await ai.startSession({ ...args, targetId: "pty-new" });
    resolveRebind();

    await expect(staleRebind).rejects.toThrow(/closed/i);
    expect(ai.sessionForTab("tab-a")).toEqual(replacement);
  });

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

    const disposing = ai.disposeTab("tab-a");
    resolveStart(info);

    await disposing;
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

    const disposing = ai.disposeTab("tab-a");
    resolveListen(lateUnlisten);

    await disposing;
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
