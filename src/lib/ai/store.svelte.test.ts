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

  it("uses the legacy saved width as each new tab's independent initial width", async () => {
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => key === "ai-panel-width" ? "515" : null,
      setItem: vi.fn(),
    });
    vi.resetModules();
    const ai = await import("./store.svelte.ts");

    ai.activateTab("tab-a");
    ai.activateTab("tab-b");
    expect(ai.panelWidth("tab-a")).toBe(515);
    expect(ai.panelWidth("tab-b")).toBe(515);

    ai.setPanelWidth("tab-a", 640);
    expect(ai.panelWidth("tab-a")).toBe(640);
    expect(ai.panelWidth("tab-b")).toBe(515);

    ai.discardPanelState("tab-a");
    expect(ai.panelWidth("tab-a")).toBeNull();
    expect(ai.commitPanelWidth("tab-a")).toBe(false);
    expect(ai.panelWidth("tab-b")).toBe(515);
  });

  it("commits a tab width as the future default without reviving a disposed tab", async () => {
    const setItem = vi.fn();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => key === "ai-panel-width" ? "515" : null,
      setItem,
    });
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.setPanelWidth("tab-a", 640);

    expect(ai.commitPanelWidth("tab-a")).toBe(true);
    expect(setItem).toHaveBeenCalledWith("ai-panel-width", "640");
    ai.activateTab("tab-b");
    expect(ai.panelWidth("tab-b")).toBe(640);

    await ai.disposeTab("tab-a");
    setItem.mockClear();
    ai.setPanelWidth("tab-a", 700);
    expect(ai.panelWidth("tab-a")).toBeNull();
    expect(ai.commitPanelWidth("tab-a")).toBe(false);
    expect(ai.panelWidth("tab-a")).toBeNull();
    expect(setItem).not.toHaveBeenCalled();
  });

  it("commits an explicit width reset and does not revive the legacy default", async () => {
    const removeItem = vi.fn();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => key === "ai-panel-width" ? "515" : null,
      setItem: vi.fn(),
      removeItem,
    });
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");

    ai.setPanelWidth("tab-a", null);
    expect(ai.commitPanelWidth("tab-a")).toBe(true);
    expect(removeItem).toHaveBeenCalledWith("ai-panel-width");

    ai.activateTab("tab-b");
    expect(ai.panelWidth("tab-a")).toBeNull();
    expect(ai.panelWidth("tab-b")).toBeNull();
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
    instance_id: "instance-a",
    target_id: "pty-old",
    skill: "general",
    model: "gpt-test",
    provider: "openai" as const,
    conversation_id: "conversation-a",
  };

  it("scopes user mutations to the current backend session instance", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    const lease = ai.captureSessionLease("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });

    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "hello", lease);

    expect(invokeMock).toHaveBeenCalledWith("ai_user_message", {
      tabId: "tab-a",
      instanceId: "instance-a",
      text: "hello",
    });
  });

  it("flushes an enqueued user message when close wins the backend event race", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let resolveSend!: () => void;
    const sendGate = new Promise<void>((resolve) => { resolveSend = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_user_message") return sendGate;
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });

    const sending = ai.sendMessage("tab-a", "last question", lease);
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_user_message",
      expect.objectContaining({ text: "last question" }),
    ));
    const closing = ai.closePanel("tab-a");
    await Promise.resolve();
    expect(invokeMock.mock.calls.some(
      ([command]) => command === "ai_conversation_save_timeline",
    )).toBe(false);

    resolveSend();
    await Promise.all([sending, closing]);
    const saves = invokeMock.mock.calls.filter(
      ([command]) => command === "ai_conversation_save_timeline",
    );
    const final = saves[saves.length - 1]?.[1] as { timeline: string };
    expect(JSON.parse(final.timeline)).toEqual([{
      kind: "user",
      text: "last question",
      at: expect.any(Number),
    }]);
  });

  it("filters a failed in-flight send from the close-time timeline", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let rejectSend!: (error: Error) => void;
    const sendGate = new Promise<void>((_, reject) => { rejectSend = reject; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_user_message") return sendGate;
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });

    const sending = ai.sendMessage("tab-a", "never accepted", lease);
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_user_message",
      expect.objectContaining({ text: "never accepted" }),
    ));
    const closing = ai.closePanel("tab-a");
    const sendFailure = expect(sending).rejects.toThrow("queue closed");
    const closeFailure = expect(closing).rejects.toThrow("queue closed");
    rejectSend(new Error("queue closed"));

    await Promise.all([sendFailure, closeFailure]);
    const saves = invokeMock.mock.calls.filter(
      ([command]) => command === "ai_conversation_save_timeline",
    );
    const final = saves[saves.length - 1]?.[1] as { timeline: string };
    expect(JSON.parse(final.timeline)).toEqual([]);
  });

  it("persists an acknowledged pending context clear before full stop", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let resolveClear!: () => void;
    const clearGate = new Promise<void>((resolve) => { resolveClear = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_clear_context") return clearGate;
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "remove me", lease);

    const clearing = ai.clearContext("tab-a", lease);
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_clear_context",
      { tabId: "tab-a", instanceId: "instance-a" },
    ));
    const closing = ai.closePanel("tab-a");
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_prepare_stop",
      { tabId: "tab-a", instanceId: "instance-a" },
    ));
    expect(invokeMock.mock.calls.some(
      ([command]) => command === "ai_conversation_save_timeline",
    )).toBe(false);

    resolveClear();
    await Promise.all([clearing, closing]);
    const saveIndex = invokeMock.mock.calls.findIndex(
      ([command]) => command === "ai_conversation_save_timeline",
    );
    const prepareIndex = invokeMock.mock.calls.findIndex(
      ([command]) => command === "ai_session_prepare_stop",
    );
    const stopIndex = invokeMock.mock.calls.findIndex(
      ([command]) => command === "ai_session_stop",
    );
    expect(prepareIndex).toBeLessThan(saveIndex);
    expect(saveIndex).toBeLessThan(stopIndex);
    const save = invokeMock.mock.calls[saveIndex]?.[1] as { timeline: string };
    expect(JSON.parse(save.timeline)).toEqual([]);
  });

  it("makes the acknowledged clear operation the single owner of UI reset", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "old context", lease);

    await ai.clearContext("tab-a", lease);

    expect(ai.chatItems("tab-a")).toEqual([]);
    expect(listenMock.mock.calls.some(
      ([event]) => event === "ai:context_cleared:tab-a",
    )).toBe(false);
  });

  it("drops terminal events from the context that was cleared before their callbacks ran", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });

    const listener = (event: string) => listenMock.mock.calls.find(
      ([name]) => name === `${event}:tab-a`,
    )?.[1];
    const assistantStart = listener("ai:assistant_message_start");
    const assistantEnd = listener("ai:assistant_message_end");
    const commandProposed = listener("ai:command_proposed");
    const commandCompleted = listener("ai:command_completed");

    await ai.clearContext("tab-a", lease);

    // These callbacks were already queued before the clear processing ack, but
    // execute afterwards. They belong to epoch 0 and must not rebuild cleared UI.
    assistantStart?.({ payload: {
      id: "old-reply",
      context_epoch: 0,
    } });
    assistantEnd?.({ payload: {
      id: "old-reply",
      text: "stale answer",
      tokens_in: 11,
      tokens_out: 7,
      context_epoch: 0,
    } });
    commandProposed?.({ payload: {
      id: "old-command",
      tool_call_id: "old-command",
      cmd: "echo stale",
      full_cmd: "echo stale",
      sentinel: "old-sentinel",
      explain: "",
      side_effect: "",
      timeout_s: 30,
      kind: "run_command",
      context_epoch: 0,
    } });
    commandCompleted?.({ payload: {
      id: "old-command",
      exit_code: 0,
      timed_out: false,
      duration_ms: 1,
      output: "stale",
      original_bytes: 5,
      truncated_bytes: 0,
      lock_keyboard: true,
      context_epoch: 0,
    } });

    expect(ai.chatItems("tab-a")).toEqual([]);
    expect(ai.pendingCommand("tab-a")).toBeNull();
    expect(ai.isKeyboardLocked("tab-a")).toBe(false);
    // Usage is actor-lifetime billing, not conversation UI. The stale bubble is
    // fenced out, but its already-billed tokens must still be accounted once.
    expect(ai.tokenUsage("tab-a")).toEqual({ tokens_in: 11, tokens_out: 7 });

    assistantStart?.({ payload: {
      id: "new-reply",
      context_epoch: 1,
    } });
    assistantEnd?.({ payload: {
      id: "new-reply",
      text: "fresh answer",
      tokens_in: 2,
      tokens_out: 3,
      context_epoch: 1,
    } });
    expect(ai.chatItems("tab-a")).toEqual([
      expect.objectContaining({
        kind: "assistant",
        id: "new-reply",
        text: "fresh answer",
        streaming: false,
      }),
    ]);
    expect(ai.tokenUsage("tab-a")).toEqual({ tokens_in: 13, tokens_out: 10 });
  });

  it("preserves next-epoch events that outrun the clear command response", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let resolveClear!: () => void;
    const clearGate = new Promise<void>((resolve) => { resolveClear = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_clear_context") return clearGate;
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "old context", lease);
    const assistantStart = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_message_start:tab-a",
    )?.[1];
    const assistantEnd = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_message_end:tab-a",
    )?.[1];

    const clearing = ai.clearContext("tab-a", lease);
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_clear_context",
      { tabId: "tab-a", instanceId: "instance-a" },
    ));

    // Event and command responses have independent delivery channels. Epoch 1
    // can therefore arrive after Rust processed clear but before invoke resolves.
    assistantStart?.({ payload: { id: "new-reply", context_epoch: 1 } });
    assistantEnd?.({ payload: {
      id: "new-reply",
      text: "new context",
      tokens_in: 1,
      tokens_out: 2,
      context_epoch: 1,
    } });
    expect(ai.chatItems("tab-a")).toEqual([
      expect.objectContaining({ kind: "user", text: "old context" }),
    ]);

    resolveClear();
    await clearing;

    expect(ai.chatItems("tab-a")).toEqual([
      expect.objectContaining({
        kind: "assistant",
        id: "new-reply",
        text: "new context",
        streaming: false,
      }),
    ]);
  });

  it("serializes a send after clear so post-clear events cannot precede the UI reset", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let resolveClear!: () => void;
    const clearGate = new Promise<void>((resolve) => { resolveClear = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_clear_context") return clearGate;
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "old context", lease);

    const clearing = ai.clearContext("tab-a", lease);
    const sending = ai.sendMessage("tab-a", "after clear", lease);
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_clear_context",
      { tabId: "tab-a", instanceId: "instance-a" },
    ));
    expect(invokeMock.mock.calls.some(
      ([command, call]) => command === "ai_user_message"
        && (call as { text?: string })?.text === "after clear",
    )).toBe(false);

    resolveClear();
    await Promise.all([clearing, sending]);

    const clearIndex = invokeMock.mock.calls.findIndex(
      ([command]) => command === "ai_session_clear_context",
    );
    const sendIndex = invokeMock.mock.calls.findIndex(
      ([command, call]) => command === "ai_user_message"
        && (call as { text?: string })?.text === "after clear",
    );
    expect(clearIndex).toBeLessThan(sendIndex);
    expect(ai.chatItems("tab-a")).toEqual([{
      kind: "user",
      client_id: expect.any(String),
      client_seq: expect.any(Number),
      text: "after clear",
      at: expect.any(Number),
    }]);
  });

  it("clears a resumed legacy timeline whose mutation sequence came from an older runtime", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_conversation_timeline") {
        return JSON.stringify([{
          kind: "user",
          client_id: "old-instance:100",
          client_seq: 100,
          text: "legacy context",
          at: 1,
        }]);
      }
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.resumeSession({ ...args, lease }, "conversation-a");

    expect(ai.chatItems("tab-a")).toEqual([
      { kind: "user", text: "legacy context", at: 1 },
    ]);

    await ai.clearContext("tab-a", lease);

    expect(ai.chatItems("tab-a")).toEqual([]);
  });

  it("keeps the old timeline when a pending context clear is rejected", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let rejectClear!: (error: Error) => void;
    const clearGate = new Promise<void>((_, reject) => { rejectClear = reject; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_clear_context") return clearGate;
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "keep me", lease);

    const clearing = ai.clearContext("tab-a", lease);
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_clear_context",
      expect.anything(),
    ));
    const closing = ai.closePanel("tab-a");
    const clearFailure = expect(clearing).rejects.toThrow("clear refused");
    const closeFailure = expect(closing).rejects.toThrow("clear refused");
    rejectClear(new Error("clear refused"));

    await Promise.all([clearFailure, closeFailure]);
    const saves = invokeMock.mock.calls.filter(
      ([command]) => command === "ai_conversation_save_timeline",
    );
    const final = saves[saves.length - 1]?.[1] as { timeline: string };
    expect(JSON.parse(final.timeline)).toEqual([{
      kind: "user",
      text: "keep me",
      at: expect.any(Number),
    }]);
  });

  it("rolls back the selected user message and every later timeline item", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    const assistantStart = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_message_start:tab-a",
    )?.[1];
    const assistantEnd = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_message_end:tab-a",
    )?.[1];

    await ai.sendMessage("tab-a", "keep", lease);
    assistantStart?.({ payload: { id: "reply-1", context_epoch: 0 } });
    assistantEnd?.({ payload: { id: "reply-1", text: "kept", context_epoch: 0 } });
    await ai.sendMessage("tab-a", "remove", lease);
    assistantStart?.({ payload: { id: "reply-2", context_epoch: 0 } });
    assistantEnd?.({ payload: { id: "reply-2", text: "removed", context_epoch: 0 } });

    await ai.rollbackContext("tab-a", 1, "remove", lease);

    expect(invokeMock).toHaveBeenCalledWith("ai_session_rollback_context", {
      tabId: "tab-a",
      instanceId: "instance-a",
      userMessageIndex: 1,
      expectedUserMessages: ["keep", "remove"],
    });
    expect(ai.chatItems("tab-a")).toEqual([
      expect.objectContaining({ kind: "user", text: "keep" }),
      expect.objectContaining({ kind: "assistant", text: "kept" }),
    ]);
  });

  it("can roll back an earlier user message when later turns exist", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_rollback_context") return [];
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "first", lease);
    await ai.sendMessage("tab-a", "second", lease);
    await ai.sendMessage("tab-a", "third", lease);

    await ai.rollbackContext("tab-a", 0, "first", lease);

    expect(invokeMock).toHaveBeenCalledWith("ai_session_rollback_context", {
      tabId: "tab-a",
      instanceId: "instance-a",
      userMessageIndex: 0,
      expectedUserMessages: ["first", "second", "third"],
    });
    expect(ai.chatItems("tab-a")).toEqual([]);
  });

  it("keeps the timeline when context rollback is rejected", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_rollback_context") throw new Error("message mismatch");
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "keep me", lease);

    await expect(ai.rollbackContext("tab-a", 0, "keep me", lease))
      .rejects.toThrow("message mismatch");

    expect(ai.chatItems("tab-a")).toEqual([
      expect.objectContaining({ kind: "user", text: "keep me" }),
    ]);
  });

  it("applies preserved command results before truncating a later user turn", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_rollback_context") return [{
        kind: "command_completed",
        payload: {
          id: "command-1",
          exit_code: 0,
          timed_out: false,
          duration_ms: 4,
          output: "done",
          original_bytes: 4,
          truncated_bytes: 0,
        },
      }];
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    const commandProposed = listenMock.mock.calls.find(
      ([event]) => event === "ai:command_proposed:tab-a",
    )?.[1];
    await ai.sendMessage("tab-a", "keep", lease);
    commandProposed?.({ payload: {
      id: "command-1",
      tool_call_id: "command-1",
      cmd: "echo done",
      full_cmd: "echo done",
      sentinel: "sentinel",
      explain: "",
      side_effect: "",
      timeout_s: 30,
      kind: "run_command",
      context_epoch: 0,
    } });
    await ai.sendMessage("tab-a", "remove", lease);

    await ai.rollbackContext("tab-a", 1, "remove", lease);

    expect(ai.chatItems("tab-a")).toEqual([
      expect.objectContaining({ kind: "user", text: "keep" }),
      expect.objectContaining({
        kind: "command",
        result: expect.objectContaining({ output: "done" }),
      }),
    ]);
  });

  it("persists an acknowledged pending rollback before full stop", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let resolveRollback!: () => void;
    const rollbackGate = new Promise<void>((resolve) => { resolveRollback = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_rollback_context") {
        await rollbackGate;
        return [];
      }
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "keep", lease);
    await ai.sendMessage("tab-a", "remove", lease);

    const rollingBack = ai.rollbackContext("tab-a", 1, "remove", lease);
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_rollback_context",
      expect.objectContaining({
        userMessageIndex: 1,
        expectedUserMessages: ["keep", "remove"],
      }),
    ));
    const closing = ai.closePanel("tab-a");
    resolveRollback();
    await Promise.all([rollingBack, closing]);

    const saves = invokeMock.mock.calls.filter(
      ([command]) => command === "ai_conversation_save_timeline",
    );
    const final = saves[saves.length - 1]?.[1] as { timeline: string };
    expect(JSON.parse(final.timeline)).toEqual([{
      kind: "user",
      text: "keep",
      at: expect.any(Number),
    }]);
  });

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

    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    const assistantStartListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_message_start:tab-a",
    )?.[1];
    const assistantEndListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_message_end:tab-a",
    )?.[1];
    await ai.sendMessage("tab-a", "old conversation", lease);
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
      {
        kind: "user",
        text: "old conversation",
        at: expect.any(Number),
      },
      {
        kind: "assistant",
        id: "reply-a",
        text: "old answer",
        at: expect.any(Number),
        streaming: false,
        cancelled: false,
      },
    ]);
    expect(invokeMock).toHaveBeenCalledWith("ai_session_stop", {
      tabId: "tab-a",
      instanceId: "instance-a",
    });

    ai.openPanel("tab-a");
    expect(ai.sessionForTab("tab-a")).toBeUndefined();
    expect(ai.chatItems("tab-a")).toEqual([]);
  });

  it("persists drained assistant and command terminal mutations before releasing the lease", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_prepare_stop") {
        return [
          {
            kind: "assistant_message_end",
            payload: {
              id: "reply-close",
              text: "canonical partial",
              cancelled: true,
            },
          },
          {
            kind: "command_completed",
            payload: {
              id: "command-close",
              exit_code: 130,
              timed_out: false,
              early_terminated: true,
              duration_ms: 42,
              output: "[REDACTED]",
              original_bytes: 128,
              truncated_bytes: 96,
              lock_keyboard: false,
            },
          },
          {
            kind: "command_rejected",
            payload: { id: "command-reject", reason: "closed" },
          },
        ];
      }
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    const assistantStart = listenMock.mock.calls.find(
      ([event]) => event === "ai:assistant_message_start:tab-a",
    )?.[1];
    const proposed = listenMock.mock.calls.find(
      ([event]) => event === "ai:command_proposed:tab-a",
    )?.[1];
    assistantStart?.({ payload: { id: "reply-close" } });
    const command = (id: string, tool_call_id: string) => ({
      id,
      tool_call_id,
      cmd: "show secret",
      full_cmd: "show secret",
      sentinel: "sentinel",
      explain: "",
      side_effect: "",
      timeout_s: 30,
      kind: "run_command" as const,
    });
    proposed?.({ payload: command("command-close", "tool-close") });
    proposed?.({ payload: command("command-reject", "tool-reject") });

    await ai.closePanel("tab-a");

    const saves = invokeMock.mock.calls.filter(
      ([commandName]) => commandName === "ai_conversation_save_timeline",
    );
    const final = saves[saves.length - 1]?.[1] as { timeline: string };
    expect(JSON.parse(final.timeline)).toEqual([
      {
        kind: "assistant",
        id: "reply-close",
        text: "canonical partial",
        at: expect.any(Number),
        streaming: false,
        cancelled: true,
      },
      expect.objectContaining({
        kind: "command",
        result: {
          id: "command-close",
          exit_code: 130,
          timed_out: false,
          early_terminated: true,
          duration_ms: 42,
          output: "[REDACTED]",
          original_bytes: 128,
          truncated_bytes: 96,
        },
      }),
      expect.objectContaining({
        kind: "command",
        rejected: { reason: "closed" },
      }),
    ]);
    const saveIndex = invokeMock.mock.calls.findIndex(
      ([commandName]) => commandName === "ai_conversation_save_timeline",
    );
    const stopIndex = invokeMock.mock.calls.findIndex(
      ([commandName]) => commandName === "ai_session_stop",
    );
    expect(saveIndex).toBeLessThan(stopIndex);
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
      instance_id: "instance-b",
      target_id: "pty-b",
      conversation_id: "conversation-b",
    };
    invokeMock.mockImplementation(async (command: string, callArgs?: unknown) => {
      if (command !== "ai_session_start") return null;
      return (callArgs as { tabId: string }).tabId === "tab-a" ? info : infoB;
    });

    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
    const leaseB = ai.captureSessionLease("tab-b");
    await ai.startSession({
      ...args,
      tabId: "tab-b",
      targetId: "pty-b",
      lease: leaseB,
    });
    await ai.sendMessage("tab-b", "keep B", leaseB);

    await ai.closePanel("tab-a");

    expect(ai.sessionForTab("tab-a")).toBeUndefined();
    expect(ai.isOpen("tab-a")).toBe(false);
    expect(ai.sessionForTab("tab-b")).toEqual(infoB);
    expect(ai.isOpen("tab-b")).toBe(true);
    expect(ai.chatItems("tab-b")).toEqual([
      {
        kind: "user",
        client_id: expect.any(String),
        client_seq: expect.any(Number),
        text: "keep B",
        at: expect.any(Number),
      },
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
      instance_id: "instance-b",
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

    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
    const closing = ai.closePanel("tab-a");
    ai.openPanel("tab-a");
    const restarting = ai.startSession({
      ...args,
      targetId: "pty-new",
      lease: ai.captureSessionLease("tab-a"),
    });

    await Promise.resolve();
    expect(startCount).toBe(1);

    resolveStop();
    await closing;
    await expect(restarting).resolves.toEqual(nextInfo);
    expect(startCount).toBe(2);
  });

  it("waits for an existing panel-close teardown when the tab is disposed", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let resolveStop!: () => void;
    const pendingStop = new Promise<void>((resolve) => { resolveStop = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_stop") return pendingStop;
      return null;
    });
    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });

    const closing = ai.closePanel("tab-a");
    let disposed = false;
    const disposing = ai.disposeTab("tab-a").then(() => { disposed = true; });
    await Promise.resolve();

    expect(disposed).toBe(false);
    resolveStop();
    await Promise.all([closing, disposing]);
    expect(disposed).toBe(true);
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

    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "persist me", lease);
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

  it("keeps the backend conversation lease until the final timeline save settles", async () => {
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
    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });

    const closing = ai.closePanel("tab-a");
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_conversation_save_timeline",
      expect.objectContaining({ id: "conversation-a" }),
    ));
    expect(invokeMock.mock.calls.some(([command]) => command === "ai_session_stop")).toBe(false);

    resolveSave();
    await closing;
    expect(invokeMock).toHaveBeenCalledWith("ai_session_stop", {
      tabId: "tab-a",
      instanceId: "instance-a",
    });
  });

  it("reads a resumed timeline only after the previous owner flushes and releases it", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.activateTab("tab-b");
    ai.openPanel("tab-a");
    ai.openPanel("tab-b");
    const resumedInfo = {
      ...info,
      tab_id: "tab-b",
      instance_id: "instance-b",
      target_id: "pty-b",
    };
    let pendingTimeline = "[]";
    let persistedTimeline = "[]";
    let resolveSave!: () => void;
    const saveGate = new Promise<void>((resolve) => { resolveSave = resolve; });
    let resolveResumeStart!: (value: typeof resumedInfo) => void;
    const resumeStart = new Promise<typeof resumedInfo>((resolve) => {
      resolveResumeStart = resolve;
    });
    invokeMock.mockImplementation(async (command: string, callArgs?: unknown) => {
      const call = callArgs as { tabId?: string; timeline?: string } | undefined;
      if (command === "ai_session_start") {
        return call?.tabId === "tab-a" ? info : resumeStart;
      }
      if (command === "ai_conversation_save_timeline") {
        pendingTimeline = call?.timeline ?? "[]";
        await saveGate;
        persistedTimeline = pendingTimeline;
        return null;
      }
      if (command === "ai_session_stop" && call?.tabId === "tab-a") {
        resolveResumeStart(resumedInfo);
        return null;
      }
      if (command === "ai_conversation_timeline") return persistedTimeline;
      return null;
    });
    const leaseA = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease: leaseA });
    await ai.sendMessage("tab-a", "latest from A", leaseA);

    const closing = ai.closePanel("tab-a");
    const resuming = ai.resumeSession({
      ...args,
      tabId: "tab-b",
      targetId: "pty-b",
      lease: ai.captureSessionLease("tab-b"),
    }, "conversation-a");
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_start",
      expect.objectContaining({ tabId: "tab-b", resume: "conversation-a" }),
    ));

    const resumeStartIndex = invokeMock.mock.calls.findIndex(
      ([command, call]) => command === "ai_session_start"
        && (call as { tabId?: string })?.tabId === "tab-b",
    );
    const timelineReadIndex = invokeMock.mock.calls.findIndex(
      ([command]) => command === "ai_conversation_timeline",
    );
    expect(timelineReadIndex === -1 || resumeStartIndex < timelineReadIndex).toBe(true);
    expect(invokeMock.mock.calls.some(
      ([command, call]) => command === "ai_session_stop"
        && (call as { tabId?: string })?.tabId === "tab-a",
    )).toBe(false);

    resolveSave();
    await closing;
    await resuming;
    expect(ai.chatItems("tab-b")).toEqual([
      {
        kind: "user",
        text: "latest from A",
        at: expect.any(Number),
      },
    ]);
  });

  it("stops an actor when its post-claim timeline read fails", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_conversation_timeline") throw new Error("timeline corrupt");
      return null;
    });

    await expect(ai.resumeSession(
      { ...args, lease: ai.captureSessionLease("tab-a") },
      "conversation-a",
    )).rejects.toThrow("timeline corrupt");

    expect(invokeMock).toHaveBeenCalledWith("ai_session_stop", {
      tabId: "tab-a",
      instanceId: "instance-a",
    });
    expect(ai.sessionForTab("tab-a")).toBeUndefined();
    expect(ai.chatItems("tab-a")).toEqual([]);
  });

  it("waits for teardown before reporting a failed final timeline save", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let resolveStop!: () => void;
    const pendingStop = new Promise<void>((resolve) => { resolveStop = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_stop") return pendingStop;
      if (command === "ai_conversation_save_timeline") throw new Error("timeline disk full");
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({ ...args, lease });
    await ai.sendMessage("tab-a", "persist me", lease);

    let settled = false;
    const closing = ai.closePanel("tab-a").finally(() => { settled = true; });
    await Promise.resolve();
    await Promise.resolve();
    expect(settled).toBe(false);

    resolveStop();
    await expect(closing).rejects.toThrow("timeline disk full");
    expect(settled).toBe(true);
  });

  it("does not leak a failed close outcome through the rapid-reopen barrier", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    const replacement = {
      ...info,
      instance_id: "instance-b",
      target_id: "pty-new",
      conversation_id: "conversation-b",
    };
    let startCount = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") {
        startCount += 1;
        return startCount === 1 ? info : replacement;
      }
      if (command === "ai_conversation_save_timeline") {
        throw new Error("timeline disk full");
      }
      return null;
    });
    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });

    const closing = ai.closePanel("tab-a");
    const closeFailure = expect(closing).rejects.toThrow("timeline disk full");
    ai.openPanel("tab-a");
    const restarting = ai.startSession({
      ...args,
      targetId: "pty-new",
      lease: ai.captureSessionLease("tab-a"),
    });

    await closeFailure;
    await expect(restarting).resolves.toEqual(replacement);
    expect(startCount).toBe(2);
  });

  it("surfaces a non-not-found error from an instance-less stop", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_stop") throw new Error("session registry poisoned");
      return null;
    });

    await expect(ai.stopSession("tab-with-missing-frontend-state"))
      .rejects.toThrow("session registry poisoned");
  });

  it("still performs full stop when prepare-stop fails", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_session_prepare_stop") throw new Error("prepare failed");
      return null;
    });
    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });

    await expect(ai.closePanel("tab-a")).rejects.toThrow("prepare failed");

    expect(invokeMock).toHaveBeenCalledWith("ai_session_stop", {
      tabId: "tab-a",
      instanceId: "instance-a",
    });
  });

  it("flushes and settles the latest streamed text after the debounce save finished", async () => {
    vi.useFakeTimers();
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });

    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
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
    expect(saves.length).toBeGreaterThanOrEqual(2);
    const finalCall = [...saves].reverse().find(([, callArgs]) => {
      const timeline = (callArgs as { timeline?: string } | undefined)?.timeline;
      return timeline?.includes("latest partial reply");
    });
    const finalArgs = finalCall?.[1] as { id: string; timeline: string };
    expect(JSON.parse(finalArgs.timeline)).toEqual([
      {
        kind: "assistant",
        id: "reply-a",
        text: "latest partial reply",
        at: expect.any(Number),
        streaming: false,
        cancelled: true,
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
      instance_id: "instance-b",
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

    const firstLaunch = ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
    await vi.waitFor(() => expect(startCount).toBe(1));
    const closing = ai.closePanel("tab-a");
    ai.openPanel("tab-a");
    const restarting = ai.startSession({
      ...args,
      targetId: "pty-new",
      lease: ai.captureSessionLease("tab-a"),
    });

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
      instance_id: "instance-c",
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

    const lease = ai.captureSessionLease("tab-a");
    const firstLaunch = ai.startSession({ ...args, lease });
    const secondLaunch = ai.startSession({ ...args, targetId: "pty-second", lease });
    await vi.waitFor(() => expect(startCount).toBe(2));

    let closeFinished = false;
    const closing = ai.closePanel("tab-a").then(() => { closeFinished = true; });
    ai.openPanel("tab-a");
    const restarting = ai.startSession({
      ...args,
      targetId: "pty-new",
      lease: ai.captureSessionLease("tab-a"),
    });

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

  it("sweeps an actor that appears after the first close stop misses it", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let resolveStart!: (value: typeof info) => void;
    const pendingStart = new Promise<typeof info>((resolve) => { resolveStart = resolve; });
    let backendActorLive = false;
    let stopCount = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") {
        const started = await pendingStart;
        backendActorLive = true;
        return started;
      }
      if (command === "ai_session_stop") {
        stopCount += 1;
        if (!backendActorLive) throw new Error("ai_session_not_found");
        backendActorLive = false;
      }
      return null;
    });

    const launching = ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_start",
      expect.objectContaining({ tabId: "tab-a" }),
    ));
    const closing = ai.closePanel("tab-a");
    await vi.waitFor(() => expect(stopCount).toBe(1));

    resolveStart(info);
    await expect(launching).rejects.toThrow(/closed/i);
    await closing;

    expect(stopCount).toBe(2);
    expect(backendActorLive).toBe(false);
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
    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });

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
      instance_id: "instance-b",
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

    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
    const staleLease = ai.captureSessionLease("tab-a");
    const staleRebind = ai.rebindTarget("tab-a", "local", "pty-stale", staleLease);
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_session_rebind_target",
      expect.objectContaining({ tabId: "tab-a", conversationId: "conversation-a" }),
    ));

    await ai.closePanel("tab-a");
    ai.openPanel("tab-a");
    await ai.startSession({
      ...args,
      targetId: "pty-new",
      lease: ai.captureSessionLease("tab-a"),
    });
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

    const launch = ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
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

    const launch = ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
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

  it("snapshots auto-approval when a proposal arrives before its dialog mounts", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    const { commandApprovals } = await import("./command-approval.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });
    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
    const proposedListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:command_proposed:tab-a",
    )?.[1];

    proposedListener?.({
      payload: {
        id: "command-delayed",
        tool_call_id: "call-delayed",
        cmd: "rm -rf /tmp/example",
        full_cmd: "rm -rf /tmp/example",
        sentinel: "sentinel-delayed",
        explain: "cleanup",
        side_effect: "deletes files",
        timeout_s: 30,
        kind: "run_command",
      },
    });

    // Models auditOpen: no CommandConfirmDialog mounted at arrival. Enabling
    // auto-approval later must not let that delayed first mount grant itself.
    expect(commandApprovals.snapshotEligibility(
      { tabId: "tab-a", instanceId: "instance-a" },
      "command-delayed",
      true,
    )).toBe(false);
  });

  it("revokes an unmounted proposal when auto-approval is disabled", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    const { commandApprovals } = await import("./command-approval.ts");
    const enabledSettings = {
      provider: "openai" as const,
      model: "gpt-test",
      endpoint: null,
      has_api_key: true,
      danger_mode: true,
      auto_run_command: true,
      auto_match_file: false,
      auto_download_file: false,
      auto_analyze_locally: false,
      auto_patch_cp: false,
      auto_patch_modify: false,
      auto_patch_diff: false,
      auto_patch_mv: false,
      auto_detect_remote_shell: false,
    };
    let currentSettings = enabledSettings;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_settings_get") return currentSettings;
      return null;
    });
    await ai.loadSettings();
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
    const proposedListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:command_proposed:tab-a",
    )?.[1];
    proposedListener?.({
      payload: {
        id: "command-hidden",
        tool_call_id: "call-hidden",
        cmd: "echo hidden",
        full_cmd: "echo hidden",
        sentinel: "sentinel-hidden",
        explain: "",
        side_effect: "",
        timeout_s: 30,
        kind: "run_command",
      },
    });
    const session = { tabId: "tab-a", instanceId: "instance-a" };
    expect(commandApprovals.isEligible(session, "command-hidden")).toBe(true);

    // The command card remains unmounted in AuditPanel while settings change.
    // A disable must be sticky even if danger mode is enabled again before mount.
    currentSettings = { ...enabledSettings, danger_mode: false };
    await ai.loadSettings();
    currentSettings = enabledSettings;
    await ai.loadSettings();

    expect(commandApprovals.snapshotEligibility(session, "command-hidden", true)).toBe(false);
  });

  it("fails closed immediately while a danger-mode disable is still saving", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    const { commandApprovals } = await import("./command-approval.ts");
    const enabledSettings = {
      provider: "openai" as const,
      model: "gpt-test",
      endpoint: null,
      has_api_key: true,
      danger_mode: true,
      auto_run_command: true,
      auto_match_file: false,
      auto_download_file: false,
      auto_analyze_locally: false,
      auto_patch_cp: false,
      auto_patch_modify: false,
      auto_patch_diff: false,
      auto_patch_mv: false,
      auto_detect_remote_shell: false,
    };
    let backendSettings = enabledSettings;
    let resolveDisable!: () => void;
    const disableGate = new Promise<void>((resolve) => { resolveDisable = resolve; });
    invokeMock.mockImplementation(async (command: string, callArgs?: unknown) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_settings_get") return backendSettings;
      if (command === "ai_settings_set") {
        await disableGate;
        const patch = (callArgs as { patch?: { dangerMode?: boolean } })?.patch;
        if (patch?.dangerMode === false) {
          backendSettings = { ...enabledSettings, danger_mode: false };
        }
      }
      return null;
    });
    await ai.loadSettings();
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
    const proposedListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:command_proposed:tab-a",
    )?.[1];

    const disabling = ai.saveSettings({ dangerMode: false });
    proposedListener?.({
      payload: {
        id: "command-during-disable",
        tool_call_id: "call-during-disable",
        cmd: "rm -rf /tmp/example",
        full_cmd: "rm -rf /tmp/example",
        sentinel: "sentinel-disable",
        explain: "cleanup",
        side_effect: "deletes files",
        timeout_s: 30,
        kind: "run_command",
      },
    });
    const disabledLocallyDuringSave = ai.settings()?.danger_mode === false;
    const eligibleDuringSave = commandApprovals.isEligible(
      { tabId: "tab-a", instanceId: "instance-a" },
      "command-during-disable",
    );
    resolveDisable();
    await disabling;

    expect(disabledLocallyDuringSave).toBe(true);
    expect(eligibleDuringSave).toBe(false);
  });

  it("does not restore automatic approval when a disable save fails", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    const enabledSettings = {
      provider: "openai" as const,
      model: "gpt-test",
      endpoint: null,
      has_api_key: true,
      danger_mode: true,
      auto_run_command: true,
      auto_match_file: false,
      auto_download_file: false,
      auto_analyze_locally: false,
      auto_patch_cp: false,
      auto_patch_modify: false,
      auto_patch_diff: false,
      auto_patch_mv: false,
      auto_detect_remote_shell: false,
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_settings_get") return enabledSettings;
      if (command === "ai_settings_set") throw new Error("settings disk full");
      return null;
    });
    await ai.loadSettings();

    await expect(ai.saveSettings({ dangerMode: false })).rejects.toThrow("settings disk full");

    expect(ai.settings()?.danger_mode).toBe(false);
  });

  it("cleans approval guards from store events and session close without mounted dialogs", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    const { commandApprovals } = await import("./command-approval.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      return null;
    });
    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
    const proposedListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:command_proposed:tab-a",
    )?.[1];
    const completedListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:command_completed:tab-a",
    )?.[1];
    const rejectedListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:command_rejected:tab-a",
    )?.[1];
    const session = { tabId: "tab-a", instanceId: "instance-a" };
    const proposal = (id: string, tool_call_id: string) => ({
      id,
      tool_call_id,
      cmd: "echo test",
      full_cmd: "echo test",
      sentinel: `sentinel-${id}`,
      explain: "",
      side_effect: "",
      timeout_s: 30,
      kind: "run_command" as const,
    });

    proposedListener?.({ payload: proposal("command-complete", "call-complete") });
    commandApprovals.markAcknowledged(session, "command-complete");
    commandApprovals.markAttempted(session, "command-complete");
    completedListener?.({
      payload: {
        id: "command-complete",
        exit_code: 0,
        timed_out: false,
        duration_ms: 1,
        output: "ok",
        original_bytes: 2,
        truncated_bytes: 0,
        lock_keyboard: false,
      },
    });
    expect(commandApprovals.isAcknowledged(session, "command-complete")).toBe(false);
    expect(commandApprovals.wasAttempted(session, "command-complete")).toBe(false);

    proposedListener?.({ payload: proposal("command-reject", "call-reject") });
    commandApprovals.markAcknowledged(session, "command-reject");
    rejectedListener?.({ payload: { id: "command-reject", reason: "no" } });
    expect(commandApprovals.isAcknowledged(session, "command-reject")).toBe(false);

    proposedListener?.({ payload: proposal("command-close", "call-close") });
    commandApprovals.markAttempted(session, "command-close");
    await ai.closePanel("tab-a");
    expect(commandApprovals.wasAttempted(session, "command-close")).toBe(false);
  });

  it("does not resurrect execution status when completion precedes the processing ack", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    let resolveReport!: () => void;
    const reportGate = new Promise<void>((resolve) => { resolveReport = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return info;
      if (command === "ai_command_result") return reportGate;
      return null;
    });
    await ai.startSession({ ...args, lease: ai.captureSessionLease("tab-a") });
    const proposedListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:command_proposed:tab-a",
    )?.[1];
    const completedListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:command_completed:tab-a",
    )?.[1];
    const proposed = {
      id: "command-ack-race",
      tool_call_id: "tool-ack-race",
      cmd: "show version",
      full_cmd: "show version",
      sentinel: "unused-for-raw-device",
      explain: "",
      side_effect: "",
      timeout_s: 30,
      kind: "run_command" as const,
    };
    proposedListener?.({ payload: proposed });
    const session = { tabId: "tab-a", instanceId: "instance-a" };
    const executing = ai.executeCommand(session, proposed, "telnet", "raw-target");
    await vi.waitFor(() => expect(ai.isCommandRunning(session, "command-ack-race")).toBe(true));
    const submitting = ai.submitCommand(session, "command-ack-race");
    await vi.waitFor(() => expect(ai.commandExecutionStatus(session, "command-ack-race"))
      .toBe("reporting"));

    completedListener?.({
      payload: {
        id: "command-ack-race",
        exit_code: 0,
        timed_out: false,
        duration_ms: 1,
        output: "ok",
        original_bytes: 2,
        truncated_bytes: 0,
        lock_keyboard: false,
      },
    });
    expect(ai.commandExecutionStatus(session, "command-ack-race")).toBeNull();

    resolveReport();
    await Promise.all([executing, submitting]);
    expect(ai.commandExecutionStatus(session, "command-ack-race")).toBeNull();
  });

  it("redelivers an internal command's recorded result without synthesizing a second payload", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    const rawInfo = { ...info, target_id: "raw-target" };
    let reportAttempts = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return rawInfo;
      if (command === "ai_command_result") {
        reportAttempts += 1;
        if (reportAttempts === 1) throw new Error("result delivery unavailable");
      }
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({
      ...args,
      targetKind: "telnet",
      targetId: "raw-target",
      lease,
    });
    const internalListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:internal_command:tab-a",
    )?.[1];
    internalListener?.({
      payload: {
        id: "probe-command",
        tool_call_id: "probe-tool",
        cmd: "show capabilities",
        full_cmd: "show capabilities",
        sentinel: "unused-for-raw-device",
      },
    });
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "telnet_write_line",
      { sessionId: "raw-target", text: "show capabilities" },
    ));
    const session = { tabId: "tab-a", instanceId: "instance-a" };

    await expect(ai.submitCommand(session, "probe-command"))
      .rejects.toThrow("result delivery unavailable");
    await vi.waitFor(() => expect(invokeMock.mock.calls.filter(
      ([command]) => command === "ai_command_result",
    )).toHaveLength(2));
    await vi.waitFor(() => expect(ai.commandExecutionStatus(session, "probe-command"))
      .toBeNull());

    expect(invokeMock.mock.calls.filter(
      ([command]) => command === "telnet_write_line",
    )).toHaveLength(1);
    const reports = invokeMock.mock.calls
      .filter(([command]) => command === "ai_command_result")
      .map(([, call]) => call);
    expect(reports).toHaveLength(2);
    expect(reports[1]).toEqual(reports[0]);
    expect(reports[0]).toEqual(expect.objectContaining({
      tabId: "tab-a",
      instanceId: "instance-a",
      toolCallId: "probe-command",
      exitCode: 0,
      earlyTerminated: false,
    }));
  });

  it("synthesizes an internal failure only when command transport setup never started", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    const rawInfo = { ...info, target_id: "raw-target" };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return rawInfo;
      return null;
    });
    listenMock.mockImplementation(async (event: string) => {
      if (event === "telnet:data:raw-target") throw new Error("listen unavailable");
      return unlistenMock;
    });
    await ai.startSession({
      ...args,
      targetKind: "telnet",
      targetId: "raw-target",
      lease: ai.captureSessionLease("tab-a"),
    });
    const internalListener = listenMock.mock.calls.find(
      ([event]) => event === "ai:internal_command:tab-a",
    )?.[1];

    internalListener?.({
      payload: {
        id: "probe-setup",
        tool_call_id: "tool-setup",
        cmd: "show capabilities",
        full_cmd: "show capabilities",
        sentinel: "unused-for-raw-device",
      },
    });

    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "ai_command_result",
      expect.objectContaining({
        tabId: "tab-a",
        instanceId: "instance-a",
        toolCallId: "probe-setup",
        exitCode: -1,
        output: "listen unavailable",
      }),
    ));
    expect(invokeMock.mock.calls.some(
      ([command]) => command === "telnet_write_line",
    )).toBe(false);
  });
});

describe("executeCommand", () => {
  it.each([
    ["serial" as const, "serial_write"],
    ["telnet" as const, "telnet_write"],
  ])("does not inject Ctrl+C into a %s device during panel close", async (kind, writeCommand) => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    ai.activateTab("tab-a");
    ai.openPanel("tab-a");
    const sessionInfo = {
      tab_id: "tab-a",
      instance_id: "instance-a",
      target_id: "raw-target",
      skill: "general",
      model: "gpt-test",
      provider: "openai" as const,
      conversation_id: "conversation-a",
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_session_start") return sessionInfo;
      return null;
    });
    const lease = ai.captureSessionLease("tab-a");
    await ai.startSession({
      tabId: "tab-a",
      targetKind: kind,
      targetId: "raw-target",
      skill: "general",
      provider: "openai",
      model: "gpt-test",
      lease,
    });
    const session = { tabId: "tab-a", instanceId: "instance-a" };
    const proposed = {
      id: "cmd-close",
      tool_call_id: "tool-close",
      cmd: "show version",
      full_cmd: "show version",
      sentinel: "unused-for-raw-device",
      explain: "",
      side_effect: "none",
      timeout_s: 30,
    };
    const running = ai.executeCommand(session, proposed, kind, "raw-target");
    await vi.waitFor(() => expect(ai.isCommandRunning(session, "cmd-close")).toBe(true));

    await ai.closePanel("tab-a");
    await running;

    expect(invokeMock).not.toHaveBeenCalledWith(writeCommand, {
      sessionId: "raw-target",
      data: [3],
    });
    expect(invokeMock).toHaveBeenCalledWith("ai_command_result", expect.objectContaining({
      tabId: "tab-a",
      instanceId: "instance-a",
      toolCallId: "cmd-close",
      earlyTerminated: true,
    }));
    const resultIndex = invokeMock.mock.calls.findIndex(
      ([command]) => command === "ai_command_result",
    );
    const prepareIndex = invokeMock.mock.calls.findIndex(
      ([command]) => command === "ai_session_prepare_stop",
    );
    expect(resultIndex).toBeLessThan(prepareIndex);
  });

  it("retries failed result delivery without replaying the transport", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    let reportAttempts = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "ai_command_result") {
        reportAttempts += 1;
        if (reportAttempts === 1) throw new Error("result store unavailable");
      }
      return null;
    });
    const session = { tabId: "tab-1", instanceId: "instance-1" };
    const proposed = {
      id: "cmd-once",
      tool_call_id: "tool-once",
      cmd: "dangerous command",
      full_cmd: "dangerous command",
      sentinel: "unused-for-telnet",
      explain: "",
      side_effect: "destructive",
      timeout_s: 30,
    };

    const running = ai.executeCommand(session, proposed, "telnet", "target-1");
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "telnet_write_line",
      { sessionId: "target-1", text: "dangerous command" },
    ));
    const submitting = ai.submitCommand(session, "cmd-once");
    await expect(running).rejects.toThrow("result store unavailable");
    await expect(submitting).rejects.toThrow("result store unavailable");
    expect(ai.commandExecutionStatus(session, "cmd-once")).toBe("delivery_failed");

    await ai.executeCommand(session, proposed, "telnet", "target-1");

    expect(invokeMock.mock.calls.filter(
      ([command]) => command === "telnet_write_line",
    )).toHaveLength(1);
    expect(invokeMock.mock.calls.filter(
      ([command]) => command === "ai_command_result",
    )).toHaveLength(2);
    expect(ai.commandExecutionStatus(session, "cmd-once")).toBe("delivered");
  });

  it("uses each sequential card id as its sole backend correlation", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    const session = { tabId: "tab-1", instanceId: "instance-1" };
    const proposed = (id: string) => ({
      id,
      tool_call_id: "patch-tool",
      cmd: id,
      full_cmd: id,
      sentinel: "unused-for-telnet",
      explain: "",
      side_effect: "",
      timeout_s: 30,
    });

    const first = ai.executeCommand(session, proposed("patch-cp"), "telnet", "target-1");
    await vi.waitFor(() => expect(ai.isCommandRunning(session, "patch-cp")).toBe(true));
    await ai.submitCommand(session, "patch-cp");
    await first;
    expect(ai.commandExecutionStatus(session, "patch-cp")).toBe("delivered");

    // The first card deliberately remains in the delivered registry: its
    // command_completed callback has not run yet. A later card from the same
    // patch_file tool call must still own a fresh transport slot.
    const second = ai.executeCommand(session, proposed("patch-modify"), "telnet", "target-1");
    await vi.waitFor(() => expect(ai.isCommandRunning(session, "patch-modify")).toBe(true));
    await ai.submitCommand(session, "patch-modify");
    await second;

    expect(invokeMock.mock.calls.filter(
      ([command]) => command === "telnet_write_line",
    )).toHaveLength(2);
    const reports = invokeMock.mock.calls
      .filter(([command]) => command === "ai_command_result")
      .map(([, args]) => args);
    expect(reports).toEqual([
      expect.objectContaining({
        toolCallId: "patch-cp",
      }),
      expect.objectContaining({
        toolCallId: "patch-modify",
      }),
    ]);
    expect(reports.every((report) => !("commandId" in (report as object)))).toBe(true);
  });

  it("keeps identical tool call ids isolated by session instance", async () => {
    vi.resetModules();
    const ai = await import("./store.svelte.ts");
    const proposed = {
      id: "cmd-shared",
      tool_call_id: "call-0",
      cmd: "show version",
      full_cmd: "show version",
      sentinel: "unused-for-raw-devices",
      explain: "",
      side_effect: "none",
      timeout_s: 30,
    };
    const sessionA = { tabId: "tab-a", instanceId: "instance-a" };
    const sessionB = { tabId: "tab-b", instanceId: "instance-b" };

    const runningA = ai.executeCommand(sessionA, proposed, "telnet", "target-a");
    await vi.waitFor(() => expect(ai.isCommandRunning(sessionA, "cmd-shared")).toBe(true));

    expect(ai.isCommandRunning(sessionB, "cmd-shared")).toBe(false);
    const runningB = ai.executeCommand(sessionB, proposed, "telnet", "target-b");
    await vi.waitFor(() => expect(ai.isCommandRunning(sessionB, "cmd-shared")).toBe(true));

    await ai.submitCommand(sessionB, "cmd-shared");
    await runningB;
    expect(ai.isCommandRunning(sessionA, "cmd-shared")).toBe(true);
    expect(ai.isCommandRunning(sessionB, "cmd-shared")).toBe(false);

    await ai.submitCommand(sessionA, "cmd-shared");
    await runningA;
  });

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

    const session = { tabId: "tab-1", instanceId: "instance-1" };
    const running = ai.executeCommand(session, proposed, "local", "session-1");
    await vi.waitFor(() => expect(ai.isCommandRunning(session, "cmd-race")).toBe(true));
    await ai.terminateCommand(session, "cmd-race");
    resolveListen(lateUnlisten);
    await running;

    const fullCommandData = Array.from(new TextEncoder().encode(`${proposed.full_cmd}\r`));
    expect(lateUnlisten).toHaveBeenCalledOnce();
    expect(invokeMock).not.toHaveBeenCalledWith("pty_write", {
      sessionId: "session-1",
      data: fullCommandData,
    });
    expect(ai.isCommandRunning(session, "cmd-race")).toBe(false);
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

    const session = { tabId: "tab-1", instanceId: "instance-1" };
    const running = ai.executeCommand(session, proposed, "telnet", "session-1");
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("telnet_write_line", {
        sessionId: "session-1",
        text: "show version",
      });
    });
    await ai.submitCommand(session, "cmd-1");
    await running;
  });
});
