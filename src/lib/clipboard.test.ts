import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

beforeEach(() => {
  invokeMock.mockReset();
  vi.unstubAllGlobals();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("text clipboard", () => {
  it("uses the desktop commands and preserves failures", async () => {
    vi.stubGlobal("navigator", { userAgent: "Macintosh" });
    invokeMock.mockRejectedValue(new Error("clipboard unavailable"));
    vi.resetModules();
    const clipboard = await import("./clipboard.ts");

    await expect(clipboard.readText()).rejects.toThrow("clipboard unavailable");
    await expect(clipboard.writeText("hello")).rejects.toThrow("clipboard unavailable");
    expect(invokeMock).toHaveBeenCalledWith("clipboard_read");
    expect(invokeMock).toHaveBeenCalledWith("clipboard_write", { text: "hello" });
  });

  it("uses the browser clipboard on mobile", async () => {
    const readText = vi.fn(async () => "mobile text");
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal("navigator", {
      userAgent: "Mozilla/5.0 (Linux; Android 15)",
      clipboard: { readText, writeText },
    });
    vi.resetModules();
    const clipboard = await import("./clipboard.ts");

    await expect(clipboard.readText()).resolves.toBe("mobile text");
    await expect(clipboard.writeText("hello")).resolves.toBeUndefined();
    expect(writeText).toHaveBeenCalledWith("hello");
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("turns synchronous browser clipboard failures into rejected promises", async () => {
    vi.stubGlobal("navigator", {
      userAgent: "Mozilla/5.0 (iPhone)",
      clipboard: {
        readText: () => { throw new Error("read denied"); },
        writeText: () => { throw new Error("write denied"); },
      },
    });
    vi.resetModules();
    const clipboard = await import("./clipboard.ts");

    await expect(clipboard.readText()).rejects.toThrow("read denied");
    await expect(clipboard.writeText("hello")).rejects.toThrow("write denied");
  });
});
