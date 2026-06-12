import { describe, it, expect } from "vitest";
import { restoreTimeline } from "./timeline.ts";
import type { ChatItem } from "./types.ts";

const STALE = "stale";

function roundtrip(items: unknown[]): ChatItem[] {
  return restoreTimeline(JSON.stringify(items), STALE);
}

describe("restoreTimeline", () => {
  it("returns [] for corrupt or non-array json", () => {
    expect(restoreTimeline("not json", STALE)).toEqual([]);
    expect(restoreTimeline('{"kind":"user"}', STALE)).toEqual([]);
  });

  it("preserves plain bubbles verbatim", () => {
    const items = [
      { kind: "user", text: "hi", at: 1 },
      { kind: "assistant", id: "a1", text: "hello", at: 2, streaming: false },
      { kind: "error", text: "boom", at: 3 },
      { kind: "note", text: "n", at: 4 },
    ];
    expect(roundtrip(items)).toEqual(items);
  });

  it("kills a resurrected streaming cursor", () => {
    const [a] = roundtrip([
      { kind: "assistant", id: "a1", text: "partial", at: 1, streaming: true },
    ]);
    expect(a.kind === "assistant" && a.streaming).toBe(false);
  });

  it("drops an empty non-cancelled assistant placeholder, keeps a cancelled one", () => {
    const items = roundtrip([
      { kind: "assistant", id: "a1", text: "", at: 1, streaming: true },
      { kind: "assistant", id: "a2", text: "", at: 2, streaming: false, cancelled: true },
    ]);
    expect(items).toHaveLength(1);
    expect(items[0].kind === "assistant" && items[0].id).toBe("a2");
  });

  it("marks an unresolved command card as stale-rejected", () => {
    const [c] = roundtrip([
      { kind: "command", cmd: { id: "c1", cmd: "ls" }, at: 1 },
    ]);
    expect(c.kind === "command" && c.rejected?.reason).toBe(STALE);
  });

  it("leaves resolved and rejected command cards alone", () => {
    const items = roundtrip([
      { kind: "command", cmd: { id: "c1", cmd: "ls" }, at: 1, result: { id: "c1", exit_code: 0 } },
      { kind: "command", cmd: { id: "c2", cmd: "ls" }, at: 2, rejected: { reason: "user said no" } },
    ]);
    expect(items[0].kind === "command" && items[0].rejected).toBeUndefined();
    expect(items[1].kind === "command" && items[1].rejected?.reason).toBe("user said no");
  });

  it("drops unknown kinds and garbage entries", () => {
    const items = roundtrip([
      null,
      42,
      { kind: "alien", at: 1 },
      { kind: "user", text: "kept", at: 2 },
    ]);
    expect(items).toEqual([{ kind: "user", text: "kept", at: 2 }]);
  });

  it("strips a non-string diff instead of crashing the diff renderer", () => {
    const [c] = roundtrip([
      { kind: "command", cmd: { id: "c1", cmd: "ls", diff: 42 }, at: 1, rejected: { reason: "no" } },
    ]);
    expect(c.kind === "command" && c.cmd.diff).toBeUndefined();
  });

  it("drops known kinds with mangled bodies instead of crashing render", () => {
    const items = roundtrip([
      { kind: "command", at: 1 },                       // no cmd object
      { kind: "command", cmd: { id: 7, cmd: "x" } },    // id not a string
      { kind: "user", at: 2 },                          // no text
      { kind: "assistant", text: "no id", at: 3 },      // no id
      { kind: "note", text: "no timestamp" },           // no at → "Invalid Date"
      { kind: "note", text: "ok", at: 4 },
    ]);
    expect(items).toEqual([{ kind: "note", text: "ok", at: 4 }]);
  });
});
