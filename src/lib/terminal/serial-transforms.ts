/**
 * Pure serial I/O transforms used by TerminalPane's serial path. Extracted here
 * so the fiddly bits (newline normalization, hex parse, login-script parse) are
 * unit-testable — the component itself can't be exercised without a DOM + xterm.
 *
 * Keep these PURE (no terminal/session refs). TerminalPane owns the stateful
 * glue (hex echo buffer, slow-send throttle, expect/send matching loop).
 */

/** What to send when the user presses Enter. */
export function inputNewline(mode: string): string {
  return mode === "lf" ? "\n" : mode === "crlf" ? "\r\n" : "\r";
}

/**
 * Normalize incoming line endings to CRLF for xterm (fixes "staircase" output
 * from LF-only devices). No lookbehind/lookahead — older WebViews choke on them;
 * these forms are idempotent (already-correct CRLF is left intact, never doubled).
 */
export function normalizeIncoming(text: string, mode: string): string {
  switch (mode) {
    case "crlf":
      return text.replace(/\r\n|\r|\n/g, "\r\n");
    case "lf":
      return text.replace(/\r?\n/g, "\r\n");
    case "cr":
      return text.replace(/\r\n?/g, "\r\n");
    default:
      return text; // raw
  }
}

/** Render bytes as an uppercase hex dump: `[0x0a, 0xff]` → `"0A FF "`. */
export function bytesToHex(bytes: Uint8Array | number[]): string {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0").toUpperCase() + " ";
  return out;
}

/**
 * Parse a hex-input string into bytes. Whitespace is ignored; an odd trailing
 * nibble is dropped; non-hex pairs (NaN) are skipped. `"de ad"` → `[0xde, 0xad]`.
 */
export function parseHexInput(hex: string): number[] {
  const clean = hex.replace(/\s+/g, "");
  const out: number[] = [];
  for (let i = 0; i + 1 < clean.length; i += 2) {
    const pair = clean.slice(i, i + 2);
    // Strict hex only. parseInt("0g", 16) returns 0 (it stops at the first bad
    // char instead of failing), which would silently send a wrong byte to the
    // device — so validate the whole pair before accepting it.
    if (!/^[0-9a-fA-F]{2}$/.test(pair)) continue;
    out.push(parseInt(pair, 16));
  }
  return out;
}

export type LoginStep = { kind: "expect" | "send"; text: string };

/**
 * Parse a login script into expect/send steps. Each line is `expect <text>` or
 * `send <text>` (case-insensitive); blank or unrecognized lines are ignored.
 */
export function parseLoginScript(script: string): LoginStep[] {
  const steps: LoginStep[] = [];
  // Split EOL-agnostically: a CRLF-saved script must not leave a trailing "\r"
  // in each line (it would end up inside the captured expect/send text and break
  // the device-output match).
  for (const line of script.split(/\r?\n/)) {
    const m = line.match(/^\s*(expect|send)\s+(.*)$/i);
    if (m) steps.push({ kind: m[1].toLowerCase() as "expect" | "send", text: m[2] });
  }
  return steps;
}

/**
 * What the Backspace key sends to the device. xterm emits DEL (0x7f) by default;
 * remap to BS (0x08) or the VT220 "Delete" sequence (CSI 3~) as the device wants.
 * Anything unrecognized keeps DEL (the no-remap default).
 */
export function backspaceBytes(mode: string): string {
  return mode === "bs" ? "\x08" : mode === "csi3" ? "\x1b[3~" : "\x7f";
}

/**
 * Remap the editing keys to the device's delete byte: Backspace (xterm sends
 * DEL 0x7f) and Delete (xterm sends VT220 "CSI 3~", i.e. ESC [ 3 ~).
 *
 * In the default `del` mode BOTH keys pass through unchanged — correct for a
 * VT/readline peer, where Backspace erases left and Delete (CSI 3~) erases
 * right. A bare device understands only ONE delete code and treats CSI 3~ as
 * junk (echoing a stray `~`); so in `bs`/`csi3` mode BOTH keys emit that single
 * configured byte — pressing either Backspace or Delete deletes. Arrow keys and
 * other CSI sequences (`\x1b[A`, `\x1b[5~`, …) are left untouched.
 */
export function remapEditingKeys(data: string, mode: string): string {
  const bs = backspaceBytes(mode);
  if (bs === "\x7f") return data; // del mode: leave Backspace & Delete as-is
  return data.replace(/\x7f/g, bs).replace(/\x1b\[3~/g, bs);
}

/**
 * Convert every line break in transmitted text to the configured EOL. Applies to
 * the Enter key AND to pasted multi-line content, so a device that only accepts
 * CR still gets CR when you paste an LF-terminated script. `mode` is the same
 * cr|lf|crlf value as `inputNewline`.
 */
export function normalizeOutgoing(text: string, mode: string): string {
  return text.replace(/\r\n|\r|\n/g, inputNewline(mode));
}
