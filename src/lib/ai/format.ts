/**
 * Small display formatters for the AI panel (chat + audit). Kept separate from
 * tokens.ts (which is strictly token-count formatting for the toolbar).
 */

/** Max chars of a command shown inline before we ellipsize. Commands the AI
 *  proposes (and rssh blocks) can be very long; the panel only needs the gist. */
const CMD_DISPLAY_MAX = 120;

/** Truncate a command for inline display, appending "…" when clipped. The full
 *  text still lives in the audit entry / tool result — this is display-only. */
export function truncateCommand(cmd: string, max: number = CMD_DISPLAY_MAX): string {
  return cmd.length > max ? cmd.slice(0, max) + "…" : cmd;
}

/** Human-readable byte size: "812 B", "45.6 KB", "12.3 MB", "1.4 GB".
 *  One decimal for KB and up; bytes stay integer. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(1)} ${units[i]}`;
}
