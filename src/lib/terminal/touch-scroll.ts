/**
 * Pure helper for TerminalPane's mobile touch-scroll. xterm 6.0.0 vendors a
 * VS Code touch-gesture service but never calls addTarget(), so touch-drag is
 * wired to nothing — desktop has the wheel, mobile had no scroll path at all.
 * This converts accumulated finger travel (px) into whole terminal lines,
 * carrying the sub-line remainder so the drag stays 1:1 with the finger and
 * never loses precision over a long swipe.
 *
 * Keep PURE. TerminalPane owns the stateful glue (touch listeners, the running
 * remainder, and the xterm.scrollLines call + its direction sign).
 *
 * trunc (toward zero), not floor: it keeps the remainder's sign matching the
 * travel, so dragging up and dragging down behave symmetrically — no special
 * case per direction.
 */
export function accumulateScroll(
  remainder: number,
  deltaPx: number,
  rowHeight: number,
): { lines: number; remainder: number } {
  if (rowHeight <= 0) return { lines: 0, remainder };
  const total = remainder + deltaPx;
  const lines = Math.trunc(total / rowHeight);
  return { lines, remainder: total - lines * rowHeight };
}
