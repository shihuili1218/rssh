import { describe, it, expect } from "vitest";
import {
  TERM_PRESETS,
  termPresetById,
  isTermPaletteRef,
  parseCustomTermJson,
  DEFAULT_TERM_REF,
} from "./term-palettes.ts";

describe("TERM_PRESETS shape", () => {
  it("has unique non-empty ids and labels", () => {
    const ids = TERM_PRESETS.map((p) => p.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const p of TERM_PRESETS) {
      expect(p.id).toBeTruthy();
      expect(p.label).toBeTruthy();
      expect(typeof p.term.background).toBe("string");
      expect(typeof p.term.foreground).toBe("string");
    }
  });

  it("includes the curated set listed in the source", () => {
    // 不强求 8 个全在（未来增删可接受），只要核心 4 个稳定
    const ids = TERM_PRESETS.map((p) => p.id);
    expect(ids).toContain("dracula");
    expect(ids).toContain("solarized-dark");
    expect(ids).toContain("nord");
    expect(ids).toContain("monokai");
  });
});

describe("termPresetById", () => {
  it("returns the matching preset", () => {
    const p = termPresetById("dracula");
    expect(p?.label).toBe("Dracula");
    expect(p?.term.background).toBe("#282A36");
  });

  it("returns undefined for unknown id", () => {
    expect(termPresetById("nope")).toBeUndefined();
    expect(termPresetById("")).toBeUndefined();
  });
});

describe("isTermPaletteRef", () => {
  it("accepts inherit / preset / custom shapes", () => {
    expect(isTermPaletteRef({ kind: "inherit" })).toBe(true);
    expect(isTermPaletteRef({ kind: "preset", id: "dracula" })).toBe(true);
    expect(
      isTermPaletteRef({
        kind: "custom",
        term: { background: "#000", foreground: "#fff" },
      }),
    ).toBe(true);
  });

  it("rejects malformed inputs", () => {
    expect(isTermPaletteRef(null)).toBe(false);
    expect(isTermPaletteRef(undefined)).toBe(false);
    expect(isTermPaletteRef("inherit")).toBe(false);
    expect(isTermPaletteRef({})).toBe(false);
    // preset 缺 id
    expect(isTermPaletteRef({ kind: "preset" })).toBe(false);
    // preset id 不是 string
    expect(isTermPaletteRef({ kind: "preset", id: 1 })).toBe(false);
    // custom 缺 term
    expect(isTermPaletteRef({ kind: "custom" })).toBe(false);
    // custom term 缺关键字段
    expect(
      isTermPaletteRef({ kind: "custom", term: { background: "#000" } }),
    ).toBe(false);
    // 完全陌生的 kind
    expect(isTermPaletteRef({ kind: "weird" })).toBe(false);
  });

  it("DEFAULT_TERM_REF passes the type guard", () => {
    expect(isTermPaletteRef(DEFAULT_TERM_REF)).toBe(true);
  });
});

describe("parseCustomTermJson", () => {
  it("parses a minimal valid theme", () => {
    const t = parseCustomTermJson('{"background":"#000","foreground":"#fff"}');
    expect(t.background).toBe("#000");
    expect(t.foreground).toBe("#fff");
  });

  it("preserves all string fields", () => {
    const raw = JSON.stringify({
      background: "#000",
      foreground: "#fff",
      cursor: "#aaa",
      red: "#f00",
      green: "#0f0",
    });
    const t = parseCustomTermJson(raw) as Record<string, unknown>;
    expect(t.cursor).toBe("#aaa");
    expect(t.red).toBe("#f00");
    expect(t.green).toBe("#0f0");
  });

  it("normalises Windows Terminal aliases", () => {
    const raw = JSON.stringify({
      background: "#000",
      foreground: "#fff",
      cursorColor: "#cc0000",
      purple: "#aa00aa",
      brightPurple: "#ff00ff",
      selection: "#222",
    });
    const t = parseCustomTermJson(raw) as Record<string, unknown>;
    expect(t.cursor).toBe("#cc0000");
    expect(t.magenta).toBe("#aa00aa");
    expect(t.brightMagenta).toBe("#ff00ff");
    expect(t.selectionBackground).toBe("#222");
    // 别名键应被删掉，避免 xterm.js 看到不认识的 key
    expect(t.cursorColor).toBeUndefined();
    expect(t.purple).toBeUndefined();
    expect(t.brightPurple).toBeUndefined();
    expect(t.selection).toBeUndefined();
  });

  it("alias does NOT clobber existing canonical key", () => {
    // 同时给 cursor 和 cursorColor，cursor 优先
    const raw = JSON.stringify({
      background: "#000",
      foreground: "#fff",
      cursor: "#good",
      cursorColor: "#bad",
    });
    const t = parseCustomTermJson(raw) as Record<string, unknown>;
    expect(t.cursor).toBe("#good");
    expect(t.cursorColor).toBeUndefined();
  });

  it("drops keys with non-string values", () => {
    const raw = JSON.stringify({
      background: "#000",
      foreground: "#fff",
      bogus: 42,
      flag: true,
      nested: { a: 1 },
    });
    const t = parseCustomTermJson(raw) as Record<string, unknown>;
    expect(t.bogus).toBeUndefined();
    expect(t.flag).toBeUndefined();
    expect(t.nested).toBeUndefined();
  });

  it("rejects invalid JSON", () => {
    expect(() => parseCustomTermJson("not json {")).toThrow(/Invalid JSON/);
  });

  it("rejects non-object JSON", () => {
    expect(() => parseCustomTermJson('"a string"')).toThrow(/JSON object/);
    expect(() => parseCustomTermJson("123")).toThrow(/JSON object/);
    expect(() => parseCustomTermJson("null")).toThrow(/JSON object/);
    expect(() => parseCustomTermJson("[1,2]")).toThrow(/background.*foreground/);
  });

  it("rejects missing background or foreground", () => {
    expect(() => parseCustomTermJson('{"background":"#000"}')).toThrow(
      /background.*foreground/,
    );
    expect(() => parseCustomTermJson('{"foreground":"#fff"}')).toThrow(
      /background.*foreground/,
    );
    // 类型不对（数字）
    expect(() =>
      parseCustomTermJson('{"background":1,"foreground":"#fff"}'),
    ).toThrow(/background.*foreground/);
  });
});
