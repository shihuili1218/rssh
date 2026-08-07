import { describe, expect, it } from "vitest";
import {
  COMMAND_BLOCK_MAX_LINES_DEFAULT,
  COMMAND_BLOCK_MAX_LINES_MAX,
  COMMAND_BLOCK_MAX_LINES_MIN,
  TERMINAL_SCROLLBACK_LINES,
  commandBlockFoldCacheLines,
  normalizeCommandBlockMaxLines,
} from "./limits.ts";

describe("command-block terminal limits", () => {
  it("normalizes missing, fractional, and out-of-range settings", () => {
    expect(normalizeCommandBlockMaxLines(null)).toBe(COMMAND_BLOCK_MAX_LINES_DEFAULT);
    expect(normalizeCommandBlockMaxLines("not-a-number")).toBe(COMMAND_BLOCK_MAX_LINES_DEFAULT);
    expect(normalizeCommandBlockMaxLines(42.9)).toBe(42);
    expect(normalizeCommandBlockMaxLines(1)).toBe(COMMAND_BLOCK_MAX_LINES_MIN);
    expect(normalizeCommandBlockMaxLines(50_000)).toBe(COMMAND_BLOCK_MAX_LINES_MAX);
  });

  it("keeps visible plus cached rows strictly below xterm scrollback", () => {
    for (const visible of [COMMAND_BLOCK_MAX_LINES_MIN, 200, COMMAND_BLOCK_MAX_LINES_MAX]) {
      expect(visible + commandBlockFoldCacheLines(visible)).toBeLessThan(TERMINAL_SCROLLBACK_LINES);
    }
  });
});
