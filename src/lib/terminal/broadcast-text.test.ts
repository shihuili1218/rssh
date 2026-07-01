import { describe, it, expect } from "vitest";
import { pickBroadcastText } from "./broadcast-text.ts";

describe("pickBroadcastText", () => {
  const doc = "echo one\necho two\necho three";

  it("returns the whole doc when there is only a caret (empty selection)", () => {
    expect(pickBroadcastText(doc, [{ from: 5, to: 5 }])).toBe(doc);
  });

  it("returns the whole doc when there are no ranges at all", () => {
    expect(pickBroadcastText(doc, [])).toBe(doc);
  });

  it("returns only the selected slice when there is a selection", () => {
    const from = doc.indexOf("echo two");
    expect(pickBroadcastText(doc, [{ from, to: from + "echo two".length }])).toBe("echo two");
  });

  it("joins multiple non-empty selections with newlines", () => {
    const t = doc.indexOf("echo three");
    expect(
      pickBroadcastText(doc, [
        { from: 0, to: 8 },
        { from: t, to: t + "echo three".length },
      ]),
    ).toBe("echo one\necho three");
  });

  it("ignores empty ranges among non-empty ones", () => {
    const t = doc.indexOf("echo two");
    expect(
      pickBroadcastText(doc, [
        { from: 2, to: 2 },
        { from: t, to: t + "echo two".length },
      ]),
    ).toBe("echo two");
  });
});
