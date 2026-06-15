import type { IBufferLine, IDecoration, IDisposable, IMarker, Terminal } from "@xterm/xterm";
import { planLine, type CompiledHighlightRule, type LineCells, type LineDecoration } from "./highlight.ts";

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

/** A visible line reduced to its highlight plan plus a signature of that plan. */
export interface VisibleLine {
    line: number;
    sig: string;
    plan: LineDecoration[];
}

export interface ReconcilePlan {
    disposeLines: number[];
    createLines: VisibleLine[];
}

/**
 * Diff the desired highlights for the visible lines against what is already
 * decorated (line → current signature). Pure so the keep/recreate/dispose/
 * create decision is unit-tested without xterm.
 *
 *   - same signature        → keep (no DOM churn — the whole point of this)
 *   - different signature    → dispose old, create new (if it still matches)
 *   - matches gone (empty)   → dispose old
 *   - new match              → create
 */
export function reconcile(visible: VisibleLine[], existing: Map<number, string>): ReconcilePlan {
    const disposeLines: number[] = [];
    const createLines: VisibleLine[] = [];
    for (const v of visible) {
        const cur = existing.get(v.line);
        if (cur !== undefined && cur === v.sig) continue;
        if (cur !== undefined) disposeLines.push(v.line);
        if (v.plan.length) createLines.push(v);
    }
    return { disposeLines, createLines };
}

interface LineEntry {
    marker: IMarker;
    sig: string;
    items: IDecoration[];
}

function sigOf(plan: LineDecoration[]): string {
    return plan.map((d) => `${d.x}:${d.width}:${d.color}`).join(",");
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
 * Decorations are PERSISTENT and anchored to markers: a decorated line keeps its
 * highlight as it scrolls (the marker tracks it) and we touch only the lines
 * whose content actually changed. Absolute line indices shift when scrollback is
 * trimmed, so the marker — not a line number — is the stable identity; we read
 * `marker.line` each pass and prune entries whose marker was disposed (trimmed).
 *
 * Scope: only the normal screen is highlighted. TUI apps (vim, htop, claude's
 * UI) run in the alternate buffer and redraw constantly — decorating them is
 * both expensive and exactly where the old approach corrupted output.
 *
 * Triggers are onWriteParsed/onScroll/onResize/onBufferChange (not onRender):
 * registering a decoration schedules a render, so triggering off onRender would
 * feed back on itself. Those four fire only on real content/position changes.
 * Resize and buffer switches reflow geometry, so they force a full rebuild.
 */
export class HighlightDecorator {
    private compiled: CompiledHighlightRule[] = [];
    private entries: LineEntry[] = []; // one per currently-decorated line
    private disposables: IDisposable[] = [];
    private scheduled = false;
    private fullRebuild = true;
    private disposed = false;

    constructor(private term: Terminal) {
        const onContent = () => this.schedule(false);
        const onReflow = () => this.schedule(true);
        this.disposables.push(
            term.onWriteParsed(onContent),
            term.onScroll(onContent),
            term.onResize(onReflow),
            term.buffer.onBufferChange(onReflow),
        );
    }

    /** Replace the active rule set and repaint everything. */
    setRules(compiled: CompiledHighlightRule[]): void {
        this.compiled = compiled;
        this.schedule(true);
    }

    /** Coalesce bursts of triggers into one repaint per animation frame. */
    private schedule(full: boolean): void {
        if (full) this.fullRebuild = true;
        if (this.scheduled || this.disposed) return;
        this.scheduled = true;
        requestAnimationFrame(() => {
            this.scheduled = false;
            if (!this.disposed) this.repaint(); // tab closed before the frame fired
        });
    }

    private clearAll(): void {
        for (const e of this.entries) {
            for (const d of e.items) d.dispose();
            e.marker.dispose();
        }
        this.entries.length = 0;
    }

    private repaint(): void {
        // Reflow (resize / buffer switch / rule change) invalidates cell columns,
        // so drop everything and rebuild from scratch.
        if (this.fullRebuild) {
            this.clearAll();
            this.fullRebuild = false;
        }
        const buf = this.term.buffer.active;
        if (buf.type === "alternate" || !this.compiled.length) {
            this.clearAll();
            return;
        }

        // Index live entries by their CURRENT line; prune any whose marker was
        // disposed (its line scrolled out of the scrollback buffer).
        const byLine = new Map<number, LineEntry>();
        const live: LineEntry[] = [];
        for (const e of this.entries) {
            if (e.marker.isDisposed || e.marker.line === -1) {
                for (const d of e.items) d.dispose();
                continue;
            }
            byLine.set(e.marker.line, e);
            live.push(e);
        }

        // Plan the visible viewport, then diff against what is already there.
        const visStart = buf.viewportY;
        const visEnd = buf.viewportY + this.term.rows;
        const visible: VisibleLine[] = [];
        for (let absLine = visStart; absLine < visEnd; absLine++) {
            const line = buf.getLine(absLine);
            if (!line) continue;
            const plan = planLine(readLineCells(line), this.compiled);
            visible.push({ line: absLine, sig: sigOf(plan), plan });
        }
        const { disposeLines, createLines } = reconcile(
            visible,
            new Map(Array.from(byLine, ([l, e]) => [l, e.sig])),
        );

        const removed = new Set<LineEntry>();
        for (const l of disposeLines) {
            const e = byLine.get(l);
            if (!e) continue;
            for (const d of e.items) d.dispose();
            e.marker.dispose();
            removed.add(e);
        }

        const cursorAbs = buf.baseY + buf.cursorY;
        const created: LineEntry[] = [];
        for (const v of createLines) {
            const entry = this.createEntry(v.line - cursorAbs, v.plan, v.sig);
            if (entry) created.push(entry);
        }

        this.entries = live.filter((e) => !removed.has(e)).concat(created);
    }

    /** Register one marker for a line and a decoration per match on it. */
    private createEntry(offset: number, plan: LineDecoration[], sig: string): LineEntry | null {
        const marker = this.term.registerMarker(offset);
        if (!marker || marker.line === -1) return null;
        const items: IDecoration[] = [];
        for (const d of plan) {
            const dec = this.term.registerDecoration({
                marker,
                x: d.x,
                width: d.width,
                foregroundColor: d.color,
                layer: "top",
            });
            if (dec) items.push(dec);
        }
        if (!items.length) {
            marker.dispose();
            return null;
        }
        return { marker, sig, items };
    }

    dispose(): void {
        this.disposed = true;
        this.clearAll();
        for (const d of this.disposables) d.dispose();
        this.disposables.length = 0;
    }
}
