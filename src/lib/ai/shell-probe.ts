/**
 * Remote-shell probe: classification core (pure, unit-tested).
 *
 * We paste PROBE_COMMAND into the PTY and read the echoed/evaluated lines back.
 * The hard part: with ECHO=on (default) the PTY echoes our literal command line
 * `echo P=$PSEdition=$$=E` back, and that literal — group1=`$PSEdition`,
 * group2=`$$` — is BYTE-IDENTICAL to what a real cmd.exe prints (cmd doesn't
 * expand `$`-vars). So the echoed input and a genuine cmd output cannot be told
 * apart by content. If we trusted it, every ECHO=on POSIX/PowerShell host would
 * look like cmd until its evaluated line arrived — and on a slow link (evaluated
 * line lands after the deadline) we'd cache cmd permanently (per-profile, whole
 * process lifetime), breaking every later AI command on that profile.
 *
 * Fix: anchor matches to line start (`^…` with the `m` flag) so ONLY standalone
 * EVALUATED output lines are classified — the echoed input line carries `echo `
 * (or a shell prompt) before `P=`, so `P=` is never line-initial there:
 *   - POSIX:      `$PSEdition` empty, `$$` = PID  → `P==<pid>=E`        → posix
 *   - PowerShell: `$PSEdition` = Desktop/Core      → `P=Desktop=...=E`   → powershell
 *   - cmd.exe:    nothing expands                  → `P=$PSEdition=$$=E` → cmd
 *     (reached only via the real output line — the echo copy is now excluded).
 * A slow link where no evaluated line arrives in time classifies as nothing →
 * the caller times out and keeps the POSIX fallback (no false cmd cache).
 */
import type { ShellKind } from "./types.ts";

/** The line pasted into the PTY. Paired with PROBE_RE: the `echo ` prefix is
 *  exactly what the lookbehind keys on to drop the echoed copy — keep them in
 *  sync. */
export const PROBE_COMMAND = "echo P=$PSEdition=$$=E";

// Anchor to line start (`^` + `m` flag): the echoed input line carries `echo `
// (or a shell prompt) before `P=`, so `P=` is never line-initial there — only
// standalone EVALUATED output lines (`P=...=E` at column 0) match. `[^=\r\n]`
// stops a capture from spanning a line / CRLF boundary; no end-anchor because
// `$` wouldn't match before the `\r` in CRLF output. Deliberately NOT a
// `(?<!echo )` lookbehind — older WebKit / WKWebView (macOS < 13.3) rejects
// lookbehind at parse time, which would throw on module load and break the app.
const PROBE_RE = /^P=([^=\r\n]*)=([^=\r\n]*)=E/gm;

/** Map one evaluated output line's two captured groups to a shell family.
 *  null = ambiguous (e.g. PS 4.x without $PSEdition) → caller doesn't cache. */
export function classifyShell(psed: string, dollar: string): ShellKind | null {
  // PS 5+ exposes $PSEdition = 'Desktop' (Windows PowerShell) or 'Core' (PS 7).
  if (psed === "Desktop" || psed === "Core") return "powershell";
  // cmd.exe expands no `$`-style vars — the whole token comes back literal.
  if (psed === "$PSEdition" && dollar === "$$") return "cmd";
  // POSIX: $PSEdition expands to empty, $$ is the shell PID (digits).
  if (psed === "" && /^\d+$/.test(dollar)) return "posix";
  // Ambiguous: PS 4.x (no $PSEdition) / fish $$ noise / custom prompt — don't
  // guess; return null so the caller leaves it uncached and re-probes later.
  return null;
}

/**
 * Classify the accumulated probe buffer (echo line already excluded by PROBE_RE).
 *  - kind = posix/powershell when an unambiguous evaluated line is present →
 *    the caller decides immediately.
 *  - kind = null, cmd = true when the only signal is a (non-echo) cmd line →
 *    the caller treats this as cmd ONLY at the deadline, giving a posix/ps
 *    evaluated line the chance to arrive first; a slow link with no evaluated
 *    line at all stays cmd = false → no cache write.
 */
export function classifyProbeBuffer(buffer: string): { kind: ShellKind | null; cmd: boolean } {
  let cmd = false;
  for (const [, psed, dollar] of buffer.matchAll(PROBE_RE)) {
    const kind = classifyShell(psed, dollar);
    if (kind === "powershell" || kind === "posix") return { kind, cmd: false };
    if (kind === "cmd") cmd = true;
  }
  return { kind: null, cmd };
}
