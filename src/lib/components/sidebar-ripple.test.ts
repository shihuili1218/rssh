import { describe, expect, it } from "vitest";
import { rippleWidth, RIPPLE } from "./sidebar-ripple.ts";

describe("rippleWidth", () => {
  it("narrows by STEP per level then clamps to FLOOR", () => {
    expect([0, 1, 2, 3, 4].map(rippleWidth)).toEqual([240, 192, 144, 96, 96]);
  });

  it("focused row (distance 0) is FOCUS width", () => {
    expect(rippleWidth(0)).toBe(RIPPLE.FOCUS);
  });

  it("never drops below FLOOR for far rows", () => {
    expect(rippleWidth(99)).toBe(RIPPLE.FLOOR);
  });
});
