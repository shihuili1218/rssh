import { describe, it, expect } from "vitest";
import { formatTokenCount } from "./tokens.ts";

describe("formatTokenCount", () => {
  it("renders small counts verbatim", () => {
    expect(formatTokenCount(0)).toBe("0");
    expect(formatTokenCount(999)).toBe("999");
  });

  it("compacts thousands with one decimal", () => {
    expect(formatTokenCount(1000)).toBe("1k");
    expect(formatTokenCount(1234)).toBe("1.2k");
    expect(formatTokenCount(45678)).toBe("45.7k");
  });

  it("drops the decimal at three digits", () => {
    expect(formatTokenCount(123_456)).toBe("123k");
    expect(formatTokenCount(999_499)).toBe("999k");
  });

  it("compacts millions, promoting where k would round to 1000k", () => {
    expect(formatTokenCount(999_999)).toBe("1M");
    expect(formatTokenCount(1_000_000)).toBe("1M");
    expect(formatTokenCount(1_234_567)).toBe("1.2M");
    expect(formatTokenCount(250_000_000)).toBe("250M");
  });
});
