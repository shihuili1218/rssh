import { describe, it, expect } from "vitest";
import { classifyShell, classifyProbeBuffer, PROBE_COMMAND } from "./shell-probe.ts";

describe("classifyShell", () => {
  it("maps PowerShell editions", () => {
    expect(classifyShell("Desktop", "anything")).toBe("powershell");
    expect(classifyShell("Core", "12345")).toBe("powershell");
  });

  it("maps the unexpanded literal to cmd.exe", () => {
    expect(classifyShell("$PSEdition", "$$")).toBe("cmd");
  });

  it("maps empty edition + numeric PID to posix", () => {
    expect(classifyShell("", "12345")).toBe("posix");
  });

  it("returns null for ambiguous signatures (e.g. PS 4.x)", () => {
    expect(classifyShell("", "$$")).toBeNull(); // $$ not numeric
    expect(classifyShell("Foo", "bar")).toBeNull();
  });
});

describe("classifyProbeBuffer", () => {
  // The pasted command, echoed back verbatim by an ECHO=on PTY. Its `P=...=E`
  // is byte-identical to a real cmd.exe output line — it MUST NOT be classified.
  const echo = (prompt: string) => `${prompt}${PROBE_COMMAND}\r\n`;

  it("classifies a POSIX evaluated line", () => {
    expect(classifyProbeBuffer("P==12345=E\r\n")).toEqual({ kind: "posix", cmd: false });
  });

  it("classifies a PowerShell evaluated line", () => {
    expect(classifyProbeBuffer("P=Desktop=token=E\r\n")).toEqual({ kind: "powershell", cmd: false });
  });

  it("ECHO=on POSIX: echo line excluded, evaluated line wins", () => {
    const buf = echo("user@host:~$ ") + "P==12345=E\r\n";
    expect(classifyProbeBuffer(buf)).toEqual({ kind: "posix", cmd: false });
  });

  it("ECHO=on PowerShell: echo line excluded, evaluated line wins", () => {
    const buf = echo("PS C:\\> ") + "P=Core=val=E\r\n";
    expect(classifyProbeBuffer(buf)).toEqual({ kind: "powershell", cmd: false });
  });

  it("ECHO=on cmd.exe: echo excluded, real output flags cmd candidate", () => {
    // cmd prints the literal (no $-expansion); only the non-echo output line counts.
    const buf = echo("C:\\> ") + "P=$PSEdition=$$=E\r\n";
    expect(classifyProbeBuffer(buf)).toEqual({ kind: null, cmd: true });
  });

  it("REGRESSION (Copilot): echo-only buffer must NOT look like cmd", () => {
    // Slow link / POSIX or PowerShell host whose evaluated line hasn't arrived
    // yet. Trusting the echoed line here would poison the per-profile cache as
    // cmd and 60s-timeout every later AI command. Must stay {null,false}.
    expect(classifyProbeBuffer(echo("user@host:~$ "))).toEqual({ kind: null, cmd: false });
    expect(classifyProbeBuffer(echo("PS /home> "))).toEqual({ kind: null, cmd: false });
  });

  it("empty / unrelated buffer → nothing", () => {
    expect(classifyProbeBuffer("")).toEqual({ kind: null, cmd: false });
    expect(classifyProbeBuffer("Last login: ...\r\nmotd\r\n")).toEqual({ kind: null, cmd: false });
  });

  it("posix evaluated line wins even if a cmd-signature output also appears", () => {
    // Defensive: an unambiguous posix line decides regardless of ordering.
    const buf = "P=$PSEdition=$$=E\r\nP==999=E\r\n";
    expect(classifyProbeBuffer(buf)).toEqual({ kind: "posix", cmd: false });
  });

  it("ignores a P=...=E that is not at line start (mid-line noise)", () => {
    // Only column-0 evaluated lines are ours; mid-line coincidences must not match.
    // This is why PROBE_RE uses a `^` anchor rather than a `(?<!echo )` lookbehind.
    expect(classifyProbeBuffer("here is P==5=E inline\r\n")).toEqual({ kind: null, cmd: false });
    expect(classifyProbeBuffer("noise P=$PSEdition=$$=E noise\r\n")).toEqual({ kind: null, cmd: false });
  });

  it("matches both LF-only and CRLF line endings", () => {
    // Guards against re-introducing a `$` end-anchor, which would not match the
    // `\r` in CRLF output and would silently break POSIX/PowerShell detection.
    expect(classifyProbeBuffer("P==7=E\n")).toEqual({ kind: "posix", cmd: false });
    expect(classifyProbeBuffer("P==7=E\r\n")).toEqual({ kind: "posix", cmd: false });
  });
});
