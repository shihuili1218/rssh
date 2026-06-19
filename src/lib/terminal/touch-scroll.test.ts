import { describe, it, expect } from "vitest";
import { accumulateScroll } from "./touch-scroll.ts";

describe("accumulateScroll", () => {
  it("holds sub-row travel as remainder, scrolls nothing yet", () => {
    expect(accumulateScroll(0, 5, 20)).toEqual({ lines: 0, remainder: 5 });
  });

  it("emits one line exactly on a row boundary, no leftover", () => {
    expect(accumulateScroll(0, 20, 20)).toEqual({ lines: 1, remainder: 0 });
  });

  it("emits one line and carries the overshoot", () => {
    expect(accumulateScroll(0, 25, 20)).toEqual({ lines: 1, remainder: 5 });
  });

  it("carries remainder across calls until a row completes", () => {
    // 15px held + 10px move = 25px → 1 line, 5px carried
    expect(accumulateScroll(15, 10, 20)).toEqual({ lines: 1, remainder: 5 });
  });

  it("is symmetric for the opposite direction (finger up)", () => {
    // trunc toward zero keeps remainder sign matching travel → no up/down skew
    expect(accumulateScroll(0, -25, 20)).toEqual({ lines: -1, remainder: -5 });
  });

  it("emits multiple lines on a fast flick", () => {
    expect(accumulateScroll(0, 65, 20)).toEqual({ lines: 3, remainder: 5 });
  });

  it("is a no-op when row height is unknown (0), preserving remainder", () => {
    expect(accumulateScroll(7, 100, 0)).toEqual({ lines: 0, remainder: 7 });
  });
});
