import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn(async () => null);
const unlistenMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => unlistenMock),
}));

beforeEach(() => {
  invokeMock.mockClear();
  unlistenMock.mockClear();
  vi.stubGlobal("localStorage", {
    getItem: () => null,
    setItem: vi.fn(),
  });
  vi.stubGlobal("window", globalThis);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("executeCommand", () => {
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
