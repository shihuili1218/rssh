import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { pickTextFile } from "./pick-file.ts";

// pickTextFile drives a hidden <input type=file>. vitest runs in `node`, so we
// stub document.createElement to hand back a controllable fake input whose
// `change` event the test fires by hand — same manual-stub style as ipc-shim.test.ts.

function fakeInput() {
    const listeners: Record<string, Array<() => void>> = {};
    return {
        type: "",
        accept: "",
        style: {} as Record<string, string>,
        files: null as Array<{ name: string; size: number; text: () => Promise<string> }> | null,
        addEventListener(ev: string, fn: () => void) {
            (listeners[ev] ??= []).push(fn);
        },
        remove() {},
        click() {},
        fire(ev: string) {
            (listeners[ev] ?? []).forEach((fn) => fn());
        },
    };
}

let input: ReturnType<typeof fakeInput>;
let win: any;

beforeEach(() => {
    input = fakeInput();
    win = { addEventListener: vi.fn() };
    vi.stubGlobal("window", win);
    vi.stubGlobal("document", { createElement: () => input, body: { appendChild() {} } });
});

afterEach(() => vi.unstubAllGlobals());

describe("pickTextFile", () => {
    it("resolves the picked file's name and text", async () => {
        const p = pickTextFile();
        input.files = [{ name: "id_rsa", size: 42, text: async () => "PRIVATE-KEY" }];
        input.fire("change");
        await expect(p).resolves.toEqual({ name: "id_rsa", text: "PRIVATE-KEY" });
    });

    it("resolves null when the user cancels (change with no file)", async () => {
        const p = pickTextFile();
        input.files = null;
        input.fire("change");
        await expect(p).resolves.toBeNull();
    });

    it("rejects oversized files with the shared key_file_too_large error shape", async () => {
        const p = pickTextFile({ maxBytes: 10 });
        input.files = [{ name: "big", size: 11, text: async () => "x" }];
        input.fire("change");
        await expect(p).rejects.toMatch(/key_file_too_large/);
    });

    it("rejects with a localizable error inside the JCEF plugin host", async () => {
        win.__RSSH_PICK__ = vi.fn(); // marks the IDEA-plugin JCEF host
        await expect(pickTextFile()).rejects.toMatch(/file_pick_unsupported_in_plugin/);
    });
});
