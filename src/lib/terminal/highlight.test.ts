import { describe, it, expect } from "vitest";
import type { HighlightRule } from "../stores/app.svelte.ts";
import {
    compileHighlightRules,
    findMatches,
    planLine,
    validateHighlightRule,
} from "./highlight.ts";

function rule(
    keyword: string,
    opts: Partial<HighlightRule> = {}
): HighlightRule {
    return {
        keyword,
        name: "",
        color: "#FF6B6B",
        enabled: true,
        is_regex: false,
        is_case_sensitive: false,
        ...opts,
    };
}

function matches(text: string, rules: HighlightRule[]) {
    return findMatches(text, compileHighlightRules(rules));
}

describe("validateHighlightRule", () => {
    it("accepts valid regex", () => {
        expect(validateHighlightRule(rule("\\d+", { is_regex: true }))).toBeNull();
    });

    it("rejects invalid regex", () => {
        const err = validateHighlightRule(rule("(\\d+", { is_regex: true }));
        expect(err?.kind).toBe("invalid");
    });

    it("rejects zero-width regex", () => {
        const err = validateHighlightRule(rule("^$", { is_regex: true }));
        expect(err?.kind).toBe("zero_width");
    });

    it("rejects pure lookaround as zero-width", () => {
        const err = validateHighlightRule(rule("(?=ERROR)", { is_regex: true }));
        expect(err?.kind).toBe("zero_width");
    });

    it("rejects name longer than 100 chars", () => {
        const err = validateHighlightRule(rule("\\d+", { is_regex: true, name: "a".repeat(101) }));
        expect(err?.kind).toBe("name_too_long");
    });

    it("ignores non-regex rules", () => {
        expect(validateHighlightRule(rule("ERROR"))).toBeNull();
    });
});

describe("findMatches", () => {
    it("finds date regex from issue #102", () => {
        const input = "log: 2026-06-09 09:05:02 done";
        const r = rule(
            "\\d{4}[-/]\\d{2}[-/]\\d{2}\\s\\d{2}:\\d{2}:\\d{2}",
            { is_regex: true, color: "#6EDAA0" }
        );
        expect(matches(input, [r])).toEqual([
            { start: 5, end: 24, color: "#6EDAA0" },
        ]);
    });

    it("matches literal keyword case-insensitively by default", () => {
        expect(matches("error ERROR", [rule("ERROR")])).toEqual([
            { start: 0, end: 5, color: "#FF6B6B" },
            { start: 6, end: 11, color: "#FF6B6B" },
        ]);
    });

    it("respects literal case sensitivity when enabled", () => {
        expect(matches("error ERROR", [rule("ERROR", { is_case_sensitive: true })])).toEqual([
            { start: 6, end: 11, color: "#FF6B6B" },
        ]);
    });

    it("matches regex case-insensitively by default", () => {
        expect(matches("ABC abc", [rule("[a-z]+", { is_regex: true })])).toEqual([
            { start: 0, end: 3, color: "#FF6B6B" },
            { start: 4, end: 7, color: "#FF6B6B" },
        ]);
    });

    it("respects regex case sensitivity when enabled", () => {
        expect(matches("ABC abc", [rule("[a-z]+", { is_regex: true, is_case_sensitive: true })])).toEqual([
            { start: 4, end: 7, color: "#FF6B6B" },
        ]);
    });

    it("treats regex alternation as a single rule", () => {
        expect(matches("foo bar", [rule("foo|bar", { is_regex: true })])).toEqual([
            { start: 0, end: 3, color: "#FF6B6B" },
            { start: 4, end: 7, color: "#FF6B6B" },
        ]);
    });

    it("skips disabled and invalid rules without throwing", () => {
        expect(matches("hello", [
            rule("(\\d+", { is_regex: true }),
            rule("hello"),
        ])).toEqual([{ start: 0, end: 5, color: "#FF6B6B" }]);
    });

    it("keeps the first rule when multiple rules overlap at same position", () => {
        expect(matches("ERRORs", [
            rule("ERROR", { color: "#FF0000" }),
            rule("[A-Z]+", { is_regex: true, color: "#00FF00" }),
        ])).toEqual([{ start: 0, end: 5, color: "#FF0000" }]);
    });

    it("returns no matches when no rules are enabled", () => {
        expect(matches("nothing here", [rule("ERROR", { enabled: false })])).toEqual([]);
    });
});

describe("planLine", () => {
    // Identity cell map: each char occupies exactly one cell (ASCII).
    function ascii(text: string) {
        const cellAt = Array.from({ length: text.length + 1 }, (_, i) => i);
        return { text, cellAt };
    }

    it("maps an ASCII match to its cell column and width", () => {
        const plan = planLine(ascii("ERROR ok"), compileHighlightRules([rule("ERROR")]));
        expect(plan).toEqual([{ x: 0, width: 5, color: "#FF6B6B" }]);
    });

    it("accounts for wide (CJK) characters before the match", () => {
        // "你ERROR": 你 occupies cells 0-1, so E starts at cell column 2.
        const cells = { text: "你ERROR", cellAt: [0, 2, 3, 4, 5, 6, 7] };
        const plan = planLine(cells, compileHighlightRules([rule("ERROR")]));
        expect(plan).toEqual([{ x: 2, width: 5, color: "#FF6B6B" }]);
    });

    it("gives a wide-character match a 2-cell width", () => {
        const cells = { text: "你好", cellAt: [0, 2, 4] };
        const plan = planLine(cells, compileHighlightRules([rule("你")]));
        expect(plan).toEqual([{ x: 0, width: 2, color: "#FF6B6B" }]);
    });

    it("resolves overlaps by rule priority like findMatches", () => {
        const plan = planLine(ascii("ERRORs"), compileHighlightRules([
            rule("ERROR", { color: "#FF0000" }),
            rule("[A-Z]+", { is_regex: true, color: "#00FF00" }),
        ]));
        expect(plan).toEqual([{ x: 0, width: 5, color: "#FF0000" }]);
    });

    it("returns nothing when no rule matches", () => {
        expect(planLine(ascii("clean line"), compileHighlightRules([rule("ERROR")]))).toEqual([]);
    });

    it("drops a match that maps to zero cell width", () => {
        // A match whose start and end resolve to the same cell column (e.g. a
        // regex matching one half of a surrogate pair) would otherwise produce
        // an invalid 0-width decoration.
        const cells = { text: "ab", cellAt: [0, 0, 1] };
        expect(planLine(cells, compileHighlightRules([rule("a")]))).toEqual([]);
    });
});
