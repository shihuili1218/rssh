import { describe, it, expect } from "vitest";
import { readLineCells } from "./highlight-decorations.ts";

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
