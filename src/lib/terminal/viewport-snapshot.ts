/**
 * A read-only "minimap" projection of a terminal's visible viewport: which
 * cells carry ink and where the cursor sits. Deliberately tiny (one byte per
 * cell) so EditPane can paint many live previews cheaply — no xterm renderer,
 * no DOM clone, no second PTY. Colour is intentionally left out for now.
 */
export interface ViewportSnapshot {
  cols: number;
  rows: number;
  /** row-major, length cols*rows. 1 = non-blank glyph, 0 = blank. */
  filled: Uint8Array;
  /** cursor position within the viewport (0-based), or null if off-screen. */
  cursor: { x: number; y: number } | null;
}

/** Minimal structural slice of xterm we read — a real Terminal satisfies it. */
export interface ViewportCell {
  getChars(): string;
  getWidth(): number;
}
export interface ViewportLine {
  getCell(x: number): ViewportCell | undefined;
}
export interface ViewportBuffer {
  viewportY: number;
  baseY: number;
  cursorX: number;
  cursorY: number;
  getLine(y: number): ViewportLine | undefined;
}
export interface ViewportSource {
  cols: number;
  rows: number;
  buffer: { active: ViewportBuffer };
}

/**
 * Project the visible viewport into a ViewportSnapshot. Reads the tab's
 * existing terminal buffer — no new connection, no replay. A cell is ink
 * unless it is empty or a lone space. getLine takes an absolute row index, so
 * the visible rows are viewportY .. viewportY + rows - 1.
 */
export function readViewportSnapshot(src: ViewportSource): ViewportSnapshot {
  const { cols, rows } = src;
  const buf = src.buffer.active;
  const filled = new Uint8Array(cols * rows);

  for (let r = 0; r < rows; r++) {
    const line = buf.getLine(buf.viewportY + r);
    if (!line) continue;
    const base = r * cols;
    for (let c = 0; c < cols; c++) {
      const cell = line.getCell(c);
      if (!cell || cell.getWidth() === 0) continue; // skip empty + wide-glyph trailing cell
      const chars = cell.getChars();
      if (chars !== "" && chars !== " ") filled[base + c] = 1;
    }
  }

  // Cursor row is absolute (baseY + cursorY); translate into the viewport so a
  // scrolled-back terminal doesn't paint the dot on the wrong row.
  const cx = buf.cursorX;
  const cy = buf.baseY + buf.cursorY - buf.viewportY;
  const cursor = cx >= 0 && cx < cols && cy >= 0 && cy < rows ? { x: cx, y: cy } : null;
  return { cols, rows, filled, cursor };
}

/**
 * Rebuild the visible viewport as one string per row — the readable text behind
 * the hover preview. Same read-only, no-new-connection contract as
 * readViewportSnapshot. Interior blanks become spaces so an equal-width <pre>
 * stays aligned; a wide glyph's trailing cell (width 0) is skipped so the glyph
 * isn't doubled. Rows are right-trimmed.
 */
export function readViewportText(src: ViewportSource): string[] {
  const { cols, rows } = src;
  const buf = src.buffer.active;
  const lines: string[] = [];

  for (let r = 0; r < rows; r++) {
    const line = buf.getLine(buf.viewportY + r);
    if (!line) {
      lines.push("");
      continue;
    }
    let s = "";
    for (let c = 0; c < cols; c++) {
      const cell = line.getCell(c);
      if (!cell) {
        s += " ";
        continue;
      }
      if (cell.getWidth() === 0) continue; // trailing half of a wide glyph
      const ch = cell.getChars();
      s += ch === "" ? " " : ch;
    }
    lines.push(s.replace(/\s+$/, ""));
  }

  return lines;
}
