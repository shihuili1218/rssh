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

// window / document share a tiny event-target registry so tests can fire the
// two cancel signals pickTextFile listens for: focus (desktop) and
// visibilitychange (Android, returning from the SAF picker activity).
function listenable(extra: object) {
    const listeners: Record<string, Array<() => void>> = {};
    return {
        addEventListener(ev: string, fn: () => void) {
            (listeners[ev] ??= []).push(fn);
        },
        removeEventListener(ev: string, fn: () => void) {
            listeners[ev] = (listeners[ev] ?? []).filter((f) => f !== fn);
        },
        fire(ev: string) {
            (listeners[ev] ?? []).slice().forEach((fn) => fn());
        },
        ...extra,
    };
}

let input: ReturnType<typeof fakeInput>;
let win: any;
let doc: any;

beforeEach(() => {
    input = fakeInput();
    win = listenable({});
    doc = listenable({
        createElement: () => input,
        body: { appendChild() {} },
        visibilityState: "visible",
    });
    vi.stubGlobal("window", win);
    vi.stubGlobal("document", doc);
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

    it("resolves null on the file input cancel event", async () => {
        const p = pickTextFile();
        input.fire("cancel");
        await expect(p).resolves.toBeNull();
    });

    it("rejects oversized files with the shared key_file_too_large error shape", async () => {
        const p = pickTextFile({ maxBytes: 10 });
        input.files = [{ name: "big", size: 11, text: async () => "x" }];
        input.fire("change");
        await expect(p).rejects.toMatch(/key_file_too_large/);
    });

    // A file can vanish or turn unreadable between pick and read; surface that
    // instead of hanging (or silently resolving null via the cancel fallback).
    it("rejects when reading the chosen file fails", async () => {
        const p = pickTextFile();
        const boom = new Error("unreadable");
        input.files = [{ name: "id_rsa", size: 4, text: async () => { throw boom; } }];
        input.fire("change");
        await expect(p).rejects.toBe(boom);
    });

    it("rejects with a localizable error inside the JCEF plugin host", async () => {
        win.__RSSH_PICK__ = vi.fn(); // marks the IDEA-plugin JCEF host
        await expect(pickTextFile()).rejects.toMatch(/file_pick_unsupported_in_plugin/);
    });

    // Desktop cancel: the file dialog closes, window refocuses, no `change`.
    it("resolves null when the window refocuses with no change", async () => {
        vi.useFakeTimers();
        try {
            const p = pickTextFile();
            win.fire("focus");
            await vi.advanceTimersByTimeAsync(500);
            await expect(p).resolves.toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });

    // Android cancel: backing out of the SAF picker fires no `change` and no
    // reliable `focus` — only visibilitychange→visible. The leading `hidden`
    // (picker opening) must be ignored, not mistaken for a cancel.
    it("resolves null when the page returns to the foreground with no change", async () => {
        vi.useFakeTimers();
        try {
            const p = pickTextFile();
            doc.visibilityState = "hidden";
            doc.fire("visibilitychange"); // picker opening — must NOT cancel
            await vi.advanceTimersByTimeAsync(500);
            doc.visibilityState = "visible";
            doc.fire("visibilitychange"); // returned from picker — cancel
            await vi.advanceTimersByTimeAsync(500);
            await expect(p).resolves.toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });
});
