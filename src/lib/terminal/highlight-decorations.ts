import type { IBufferLine, IDecoration, IDisposable, Terminal } from "@xterm/xterm";
import { planLine, type CompiledHighlightRule, type LineCells } from "./highlight.ts";

/**
 * Read one buffer line into {@link LineCells}: the displayed text plus a map
 * from char offset to starting cell column.
 *
 * Two things make char offset ≠ cell column, and both are handled here:
 *   - wide (CJK) glyphs occupy 2 columns; xterm stores a width-0 spacer cell
 *     after them, which we skip — the next glyph's column already accounts for it
 *   - trailing cells that were never written ('' content) are padding, not output,
 *     so we trim them; matching them would let a `.*`/`\s` rule paint dead space
 */
export function readLineCells(line: Pick<IBufferLine, "getCell" | "length">): LineCells {
    let text = "";
    const cellAt: number[] = [];
    let writtenLen = 0; // text length up to and including the last written glyph
    let writtenEndCol = 0; // cell column just past that glyph
    for (let x = 0; x < line.length; x++) {
        const cell = line.getCell(x);
        if (!cell) break;
        const w = cell.getWidth();
        if (w === 0) continue; // spacer column trailing a wide glyph
        const chars = cell.getChars();
        // One glyph can be several UTF-16 units (emoji, astral, combining marks),
        // yet findMatches works in UTF-16 offsets — so push one cellAt entry per
        // unit, all pointing at this glyph's column, or text and cellAt drift apart.
        const glyph = chars === "" ? " " : chars; // unwritten interior cell → space
        for (let k = 0; k < glyph.length; k++) cellAt.push(x);
        text += glyph;
        if (chars !== "") {
            writtenLen = text.length;
            writtenEndCol = x + w;
        }
    }
    text = text.slice(0, writtenLen);
    cellAt.length = writtenLen;
    cellAt.push(writtenEndCol);
    return { text, cellAt };
}

/**
 * Live keyword highlighting as an xterm decoration layer.
 *
 * Why a decoration layer instead of rewriting the byte stream (issue #114):
 * the old approach injected ANSI color codes into PTY output before write(),
 * which reset the program's own SGR state, tore OSC sequences split across
 * write chunks, and leaned on a regex as a stand-in terminal parser. xterm has
 * already parsed the stream into a styled cell grid; highlighting belongs on
 * top of that grid, not back in the raw bytes.
 *
 * Scope: only the normal screen is highlighted. TUI apps (vim, htop, claude's
 * UI) run in the alternate buffer and redraw constantly — decorating them is
 * both expensive and exactly where the old approach corrupted output.
 *
 * Triggers are onWriteParsed/onScroll/onResize/onBufferChange (not onRender):
 * registering a decoration schedules a render, so triggering off onRender would
 * feed back on itself. Those four fire only on real content/position changes.
 */
export class HighlightDecorator {
    private compiled: CompiledHighlightRule[] = [];
    private items: IDecoration[] = [];
    private disposables: IDisposable[] = [];
    private scheduled = false;
    private disposed = false;

    constructor(private term: Terminal) {
        const schedule = () => this.schedule();
        this.disposables.push(
            term.onWriteParsed(schedule),
            term.onScroll(schedule),
            term.onResize(schedule),
            term.buffer.onBufferChange(schedule),
        );
    }

    /** Replace the active rule set and repaint. */
    setRules(compiled: CompiledHighlightRule[]): void {
        this.compiled = compiled;
        this.schedule();
    }

    /** Coalesce bursts of triggers into one repaint per animation frame. */
    private schedule(): void {
        if (this.scheduled || this.disposed) return;
        this.scheduled = true;
        requestAnimationFrame(() => {
            this.scheduled = false;
            if (!this.disposed) this.repaint(); // tab closed before the frame fired
        });
    }

    private clear(): void {
        for (const d of this.items) {
            const marker = d.marker;
            d.dispose();
            marker.dispose();
        }
        this.items.length = 0;
    }

    private repaint(): void {
        this.clear();
        const buf = this.term.buffer.active;
        if (buf.type === "alternate" || !this.compiled.length) return;

        const cursorAbs = buf.baseY + buf.cursorY;
        for (let row = 0; row < this.term.rows; row++) {
            const absLine = buf.viewportY + row;
            const line = buf.getLine(absLine);
            if (!line) continue;
            const plan = planLine(readLineCells(line), this.compiled);
            for (const d of plan) this.decorate(absLine - cursorAbs, d.x, d.width, d.color);
        }
    }

    private decorate(offset: number, x: number, width: number, color: string): void {
        const marker = this.term.registerMarker(offset);
        if (!marker || marker.line === -1) return;
        const dec = this.term.registerDecoration({
            marker,
            x,
            width,
            foregroundColor: color,
            layer: "top",
        });
        if (dec) this.items.push(dec);
        else marker.dispose();
    }

    dispose(): void {
        this.disposed = true;
        this.clear();
        for (const d of this.disposables) d.dispose();
        this.disposables.length = 0;
    }
}
