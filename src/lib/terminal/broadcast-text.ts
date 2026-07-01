export interface DocRange {
  from: number;
  to: number;
}

/**
 * Pick what the broadcast editor should send: the selected text when there is a
 * non-empty selection, otherwise the whole document. Multiple non-empty
 * selections (multi-cursor) are joined with newlines. Callers still apply their
 * own trailing-newline and blank-input guards.
 */
export function pickBroadcastText(doc: string, ranges: DocRange[]): string {
  const selected = ranges.filter((r) => r.to > r.from);
  if (selected.length === 0) return doc;
  return selected.map((r) => doc.slice(r.from, r.to)).join("\n");
}
