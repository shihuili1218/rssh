import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The real-Tauri branch lazily imports these plugins; mock them so the branch
// is exercisable in node. vi.hoisted lets the factories reference the spies.
const { saveMock, writeTextFileMock } = vi.hoisted(() => ({
    saveMock: vi.fn(),
    writeTextFileMock: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: saveMock }));
vi.mock("@tauri-apps/plugin-fs", () => ({ writeTextFile: writeTextFileMock }));

import { saveTextFile } from "./save-file.ts";

let win: any;
let anchor: any;

beforeEach(() => {
    saveMock.mockReset();
    writeTextFileMock.mockReset();
    win = {};
    anchor = { href: "", download: "", style: {} as Record<string, string>, click: vi.fn(), remove: vi.fn() };
    vi.stubGlobal("window", win);
    vi.stubGlobal("document", { createElement: () => anchor, body: { appendChild() {} } });
    vi.stubGlobal("URL", { createObjectURL: () => "blob:x", revokeObjectURL: vi.fn() });
});

afterEach(() => vi.unstubAllGlobals());

describe("saveTextFile", () => {
    it("browser: triggers a Blob download and returns the default name", async () => {
        const r = await saveTextFile("hello", { defaultName: "cfg.json" });
        expect(anchor.download).toBe("cfg.json");
        expect(anchor.click).toHaveBeenCalled();
        expect(r).toBe("cfg.json");
    });

    it("JCEF plugin host: rejects with a localizable error (downloads are dropped there)", async () => {
        win.__RSSH_PICK__ = vi.fn();
        await expect(saveTextFile("x", { defaultName: "cfg.json" })).rejects.toMatch(
            /file_save_unsupported_in_plugin/,
        );
    });

    it("real Tauri: save dialog + fs writeTextFile, returns the chosen path", async () => {
        win.__TAURI_INTERNALS__ = {};
        saveMock.mockResolvedValue("/home/u/cfg.json");
        const r = await saveTextFile("data", { defaultName: "cfg.json" });
        expect(saveMock).toHaveBeenCalled();
        expect(writeTextFileMock).toHaveBeenCalledWith("/home/u/cfg.json", "data");
        expect(r).toBe("/home/u/cfg.json");
    });

    it("real Tauri: user cancels → returns null, no write", async () => {
        win.__TAURI_INTERNALS__ = {};
        saveMock.mockResolvedValue(null);
        const r = await saveTextFile("data", { defaultName: "cfg.json" });
        expect(writeTextFileMock).not.toHaveBeenCalled();
        expect(r).toBeNull();
    });
});
