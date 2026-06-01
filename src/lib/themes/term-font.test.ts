import { describe, it, expect } from "vitest";
import { BASE_FONT_STACK, composeTermFontStack } from "./term-font.ts";

describe("composeTermFontStack", () => {
  it("empty choice → base stack unchanged (historical default)", () => {
    expect(composeTermFontStack("")).toBe(BASE_FONT_STACK);
  });

  it("blank/whitespace choice trims to empty → base stack", () => {
    expect(composeTermFontStack("   ")).toBe(BASE_FONT_STACK);
  });

  it("prepends the quoted family before the base stack", () => {
    expect(composeTermFontStack("Menlo")).toBe(`"Menlo", ${BASE_FONT_STACK}`);
  });

  it("handles family names containing spaces", () => {
    expect(composeTermFontStack("JetBrains Mono")).toBe(
      `"JetBrains Mono", ${BASE_FONT_STACK}`,
    );
  });

  it("invariant: result always ends with the base stack (glyph coverage preserved)", () => {
    for (const choice of ["", "Menlo", "Fira Code", "  Comic Mono  "]) {
      expect(composeTermFontStack(choice).endsWith(BASE_FONT_STACK)).toBe(true);
    }
  });

  it("escapes quotes/backslashes in the family name (valid CSS token)", () => {
    expect(composeTermFontStack('Ev"il')).toBe(`"Ev\\"il", ${BASE_FONT_STACK}`);
    expect(composeTermFontStack("Menlo")).toBe(`"Menlo", ${BASE_FONT_STACK}`);
  });
});
