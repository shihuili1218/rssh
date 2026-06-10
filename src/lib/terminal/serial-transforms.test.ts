import { describe, it, expect } from "vitest";
import {
  inputNewline,
  normalizeIncoming,
  bytesToHex,
  parseHexInput,
  parseLoginScript,
  backspaceBytes,
  normalizeOutgoing,
} from "./serial-transforms";

describe("inputNewline", () => {
  it("maps cr / lf / crlf and defaults to CR", () => {
    expect(inputNewline("cr")).toBe("\r");
    expect(inputNewline("lf")).toBe("\n");
    expect(inputNewline("crlf")).toBe("\r\n");
    expect(inputNewline("garbage")).toBe("\r");
  });
});

describe("normalizeIncoming", () => {
  it("raw passes through untouched", () => {
    expect(normalizeIncoming("a\nb\rc", "raw")).toBe("a\nb\rc");
  });

  it("lf: lone LF → CRLF, existing CRLF not doubled (idempotent)", () => {
    expect(normalizeIncoming("a\nb", "lf")).toBe("a\r\nb");
    expect(normalizeIncoming("a\r\nb", "lf")).toBe("a\r\nb");
    // re-running is a no-op
    expect(normalizeIncoming(normalizeIncoming("a\nb", "lf"), "lf")).toBe("a\r\nb");
  });

  it("cr: lone CR → CRLF, existing CRLF not doubled", () => {
    expect(normalizeIncoming("a\rb", "cr")).toBe("a\r\nb");
    expect(normalizeIncoming("a\r\nb", "cr")).toBe("a\r\nb");
  });

  it("crlf: normalizes any mix to CRLF", () => {
    expect(normalizeIncoming("a\nb\rc\r\nd", "crlf")).toBe("a\r\nb\r\nc\r\nd");
  });
});

describe("bytesToHex", () => {
  it("renders uppercase, zero-padded, space-separated", () => {
    expect(bytesToHex([0x0a, 0xff, 0x00])).toBe("0A FF 00 ");
    expect(bytesToHex(new Uint8Array([0xde, 0xad]))).toBe("DE AD ");
    expect(bytesToHex([])).toBe("");
  });
});

describe("parseHexInput", () => {
  it("parses spaced and unspaced hex equally", () => {
    expect(parseHexInput("de ad be ef")).toEqual([0xde, 0xad, 0xbe, 0xef]);
    expect(parseHexInput("deadbeef")).toEqual([0xde, 0xad, 0xbe, 0xef]);
  });
  it("drops an odd trailing nibble", () => {
    expect(parseHexInput("abc")).toEqual([0xab]);
  });
  it("skips non-hex pairs (NaN)", () => {
    expect(parseHexInput("zz")).toEqual([]);
    expect(parseHexInput("")).toEqual([]);
  });
});

describe("parseLoginScript", () => {
  it("parses alternating expect/send, case-insensitive, ignoring junk lines", () => {
    const steps = parseLoginScript(
      "expect login:\nsend root\n\n# a comment\nEXPECT Password:\nSEND secret",
    );
    expect(steps).toEqual([
      { kind: "expect", text: "login:" },
      { kind: "send", text: "root" },
      { kind: "expect", text: "Password:" },
      { kind: "send", text: "secret" },
    ]);
  });
  it("returns [] for empty / all-junk input", () => {
    expect(parseLoginScript("")).toEqual([]);
    expect(parseLoginScript("nonsense\n   ")).toEqual([]);
  });
});

describe("backspaceBytes", () => {
  it("maps bs / csi3 and defaults to DEL", () => {
    expect(backspaceBytes("del")).toBe("\x7f");
    expect(backspaceBytes("bs")).toBe("\x08");
    expect(backspaceBytes("csi3")).toBe("\x1b[3~");
    expect(backspaceBytes("garbage")).toBe("\x7f");
  });
});

describe("normalizeOutgoing", () => {
  it("converts every line break (CRLF/CR/LF mix) to the configured EOL", () => {
    expect(normalizeOutgoing("a\nb\r\nc\rd", "cr")).toBe("a\rb\rc\rd");
    expect(normalizeOutgoing("a\nb\r\nc\rd", "crlf")).toBe("a\r\nb\r\nc\r\nd");
    expect(normalizeOutgoing("a\nb", "lf")).toBe("a\nb");
  });
  it("leaves text without line breaks untouched", () => {
    expect(normalizeOutgoing("plain text", "cr")).toBe("plain text");
    expect(normalizeOutgoing("", "crlf")).toBe("");
  });
});
