import { describe, expect, it } from "vitest";
import { DEFAULT_PALETTE_ID, PALETTES, paletteById } from "./palettes.ts";

describe("paletteById", () => {
  it("returns the requested preset when the id exists", () => {
    const dracula = paletteById("dracula");
    expect(dracula.id).toBe("dracula");
    expect(dracula.label).toBe("Dracula");
  });

  it("falls back to the default palette for unknown ids", () => {
    const fallback = paletteById("missing-palette");
    expect(fallback.id).toBe(DEFAULT_PALETTE_ID);
  });
});

describe("PALETTES", () => {
  it("contains the default palette id exactly once", () => {
    const matches = PALETTES.filter((palette) => palette.id === DEFAULT_PALETTE_ID);
    expect(matches).toHaveLength(1);
  });

  it("uses unique ids for every preset", () => {
    const ids = PALETTES.map((palette) => palette.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("provides required ui and terminal colors for every preset", () => {
    for (const palette of PALETTES) {
      expect(palette.ui.bg).toBeTruthy();
      expect(palette.ui.surface).toBeTruthy();
      expect(palette.ui.accent).toBeTruthy();
      expect(palette.ui.text).toBeTruthy();
      expect(palette.term.background).toBeTruthy();
      expect(palette.term.foreground).toBeTruthy();
      expect(palette.term.black).toBeTruthy();
      expect(palette.term.white).toBeTruthy();
      expect(palette.term.brightBlack).toBeTruthy();
      expect(palette.term.brightWhite).toBeTruthy();
    }
  });
});
