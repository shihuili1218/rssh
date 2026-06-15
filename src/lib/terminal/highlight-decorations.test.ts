import { describe, it, expect, vi, afterEach } from "vitest";
import { HighlightDecorator, readLineCells, reconcile } from "./highlight-decorations.ts";
import { compileHighlightRules } from "./highlight.ts";
import type { HighlightRule } from "../stores/app.svelte.ts";

/**
 * Build a fake IBufferLine from [chars, width] pairs. This is a data-only
 * double of xterm's line interface — we feed cell contents exactly as xterm
 * would, then assert our own column-mapping logic.
 */
function line(cells: Array<[string, number]>): any {
    return {
        length: cells.length,
        getCell: (x: number) => {
            const c = cells[x];
            if (!c) return undefined;
            return { getChars: () => c[0], getWidth: () => c[1] };
        },
    };
}

const CJK = "你"; // 你, one UTF-16 unit, width 2
const EMOJI = "\u{1F600}"; // 😀, two UTF-16 units, width 2
const ACUTE = "́"; // combining acute accent

describe("readLineCells", () => {
    it("maps ASCII cells one-to-one", () => {
        expect(readLineCells(line([["a", 1], ["b", 1]]))).toEqual({
            text: "ab",
            cellAt: [0, 1, 2],
        });
    });

    it("skips the spacer cell after a wide glyph", () => {
        // CJK occupies columns 0-1 (col 1 is a width-0 spacer); b is at column 2.
        expect(readLineCells(line([[CJK, 2], ["", 0], ["b", 1]]))).toEqual({
            text: CJK + "b",
            cellAt: [0, 2, 3],
        });
    });

    it("gives a trailing wide glyph the right end column", () => {
        expect(readLineCells(line([["a", 1], [CJK, 2], ["", 0]]))).toEqual({
            text: "a" + CJK,
            cellAt: [0, 1, 3],
        });
    });

    it("trims trailing unwritten cells", () => {
        expect(readLineCells(line([["h", 1], ["i", 1], ["", 1], ["", 1]]))).toEqual({
            text: "hi",
            cellAt: [0, 1, 2],
        });
    });

    it("returns an empty line as empty text with a zero end column", () => {
        expect(readLineCells(line([["", 1], ["", 1]]))).toEqual({
            text: "",
            cellAt: [0],
        });
    });

    it("maps a 2-unit emoji glyph without shifting later columns", () => {
        // The emoji is two UTF-16 units in one width-2 cell. cellAt must have one
        // entry per UTF-16 unit so string offsets stay aligned with cell columns.
        expect(readLineCells(line([[EMOJI, 2], ["", 0], ["x", 1]]))).toEqual({
            text: EMOJI + "x",
            cellAt: [0, 0, 2, 3],
        });
    });

    it("maps a combining sequence stored in one cell", () => {
        // "e" + combining acute is two UTF-16 units in one width-1 cell.
        expect(readLineCells(line([["e" + ACUTE, 1], ["x", 1]]))).toEqual({
            text: "e" + ACUTE + "x",
            cellAt: [0, 0, 1, 2],
        });
    });
});

describe("reconcile", () => {
    const plan = (color = "#fff") => [{ x: 0, width: 5, color }];

    it("keeps a line whose signature is unchanged (no churn)", () => {
        expect(reconcile(
            [{ line: 10, sig: "a", plan: plan() }],
            new Map([[10, "a"]]),
        )).toEqual({ disposeLines: [], createLines: [] });
    });

    it("recreates a line whose signature changed", () => {
        const v = { line: 10, sig: "b", plan: plan() };
        expect(reconcile([v], new Map([[10, "a"]]))).toEqual({
            disposeLines: [10],
            createLines: [v],
        });
    });

    it("disposes a line that lost all its matches", () => {
        expect(reconcile(
            [{ line: 10, sig: "", plan: [] }],
            new Map([[10, "a"]]),
        )).toEqual({ disposeLines: [10], createLines: [] });
    });

    it("creates a newly matched line", () => {
        const v = { line: 12, sig: "a", plan: plan() };
        expect(reconcile([v], new Map())).toEqual({
            disposeLines: [],
            createLines: [v],
        });
    });

    it("ignores a new line that has no matches", () => {
        expect(reconcile(
            [{ line: 12, sig: "", plan: [] }],
            new Map(),
        )).toEqual({ disposeLines: [], createLines: [] });
    });
});

/* ─────────────────────────────────────────────────────────────
 * Fake xterm.js Terminal — the minimal subset HighlightDecorator
 * uses, with a controllable rAF so repaints are deterministic.
 * Data-only double: we feed buffer/marker/decoration data exactly
 * as xterm would and assert our own create/dispose bookkeeping.
 * ───────────────────────────────────────────────────────────── */
function asciiLine(s: string) {
    return {
        length: s.length,
        getCell: (x: number) =>
            x < s.length ? { getChars: () => s[x], getWidth: () => 1 } : undefined,
    };
}

function fakeTerm() {
    const buf = { type: "normal" as "normal" | "alternate", baseY: 0, cursorY: 0, viewportY: 0 };
    let rows = 3;
    const lines = new Map<number, string>();
    const ev = { write: new Set<() => void>(), scroll: new Set<() => void>(), resize: new Set<() => void>(), bufc: new Set<(b: any) => void>() };
    const sub = (set: Set<any>) => (fn: any) => { set.add(fn); return { dispose: () => set.delete(fn) }; };
    const markers: any[] = [];
    const decos: any[] = [];

    const term = {
        get rows() { return rows; },
        onWriteParsed: sub(ev.write),
        onScroll: sub(ev.scroll),
        onResize: sub(ev.resize),
        buffer: {
            get active() {
                return {
                    type: buf.type, baseY: buf.baseY, cursorY: buf.cursorY, viewportY: buf.viewportY,
                    getLine: (y: number) => (lines.has(y) ? asciiLine(lines.get(y)!) : undefined),
                };
            },
            onBufferChange: sub(ev.bufc),
        },
        registerMarker(offset: number) {
            const od: Array<() => void> = [];
            const m = {
                id: markers.length + 1, line: buf.baseY + buf.cursorY + offset, isDisposed: false,
                onDispose(fn: () => void) { od.push(fn); return { dispose() {} }; },
                dispose() { if (this.isDisposed) return; this.isDisposed = true; od.forEach((f) => f()); },
            };
            markers.push(m);
            return m;
        },
        registerDecoration({ marker }: any) {
            const d = { marker, isDisposed: false, dispose() { this.isDisposed = true; } };
            decos.push(d);
            return d;
        },
    };

    const rafq: Array<() => void> = [];
    vi.stubGlobal("requestAnimationFrame", (cb: () => void) => { rafq.push(cb); return rafq.length; });

    return {
        term: term as any,
        setLine(absLine: number, text: string) { lines.set(absLine, text); },
        clearLine(absLine: number) { lines.delete(absLine); },
        fireWrite() { ev.write.forEach((f) => f()); },
        fireResize() { ev.resize.forEach((f) => f()); },
        setBuffer(t: "normal" | "alternate") { buf.type = t; ev.bufc.forEach((f) => f({ type: t })); },
        flush() { const q = rafq.splice(0); q.forEach((cb) => cb()); },
        markers,
        activeDecos: () => decos.filter((d) => !d.isDisposed).length,
        createdDecos: () => decos.length,
    };
}

function rule(keyword: string): HighlightRule {
    return { keyword, name: "", color: "#FF6B6B", enabled: true, is_regex: false, is_case_sensitive: false };
}

describe("HighlightDecorator lifecycle", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("creates one decoration for a matched visible line", () => {
        const f = fakeTerm();
        const d = new HighlightDecorator(f.term);
        f.setLine(0, "ERROR here");
        d.setRules(compileHighlightRules([rule("ERROR")]));
        f.flush();
        expect(f.activeDecos()).toBe(1);
        expect(f.createdDecos()).toBe(1);
    });

    it("keeps decorations across an unchanged repaint (no churn)", () => {
        const f = fakeTerm();
        const d = new HighlightDecorator(f.term);
        f.setLine(0, "ERROR here");
        d.setRules(compileHighlightRules([rule("ERROR")]));
        f.flush();
        f.fireWrite(); // content unchanged
        f.flush();
        expect(f.createdDecos()).toBe(1); // not recreated
        expect(f.activeDecos()).toBe(1);
    });

    it("recreates when a line's matches change", () => {
        const f = fakeTerm();
        const d = new HighlightDecorator(f.term);
        f.setLine(0, "ERROR here");
        d.setRules(compileHighlightRules([rule("ERROR")]));
        f.flush();
        f.setLine(0, "all clear now"); // ERROR gone
        f.fireWrite();
        f.flush();
        expect(f.activeDecos()).toBe(0); // old disposed, nothing to recreate
    });

    it("disposes all decorations when switching to the alternate buffer", () => {
        const f = fakeTerm();
        const d = new HighlightDecorator(f.term);
        f.setLine(0, "ERROR here");
        d.setRules(compileHighlightRules([rule("ERROR")]));
        f.flush();
        expect(f.activeDecos()).toBe(1);
        f.setBuffer("alternate");
        f.flush();
        expect(f.activeDecos()).toBe(0);
    });

    it("prunes an entry whose marker was trimmed from scrollback", () => {
        const f = fakeTerm();
        const d = new HighlightDecorator(f.term);
        f.setLine(0, "ERROR here");
        d.setRules(compileHighlightRules([rule("ERROR")]));
        f.flush();
        // Simulate the line scrolling out of scrollback: xterm disposes the
        // marker and the line is gone from the buffer.
        f.markers[0].dispose();
        f.clearLine(0);
        f.fireWrite();
        f.flush();
        expect(f.activeDecos()).toBe(0);
    });
});
