import type { HighlightRule } from "../stores/app.svelte.ts";

export interface CompiledHighlightRule {
    keyword: string;
    color: string;
    enabled: boolean;
    is_regex: boolean;
    is_case_sensitive: boolean;
    source: string;
    regex: RegExp | null;
}

export type HighlightValidationError =
    | { kind: "invalid"; message: string }
    | { kind: "zero_width" }
    | { kind: "name_too_long" };

const MAX_HIGHLIGHT_NAME = 100;

/**
 * Detect regexes that consist solely of zero-width assertions (anchors,
 * word boundaries, lookarounds). Such a pattern cannot visibly highlight text.
 * This is a best-effort check; the runtime also defends against zero-width
 * matches to avoid infinite loops.
 */
function isPureZeroWidth(pattern: string): boolean {
    const s = pattern.trim();
    if (!s) return true;
    let i = 0;
    while (i < s.length) {
        const c = s[i];
        if (c === "^" || c === "$") {
            i++;
            continue;
        }
        if (c === "\\" && (s[i + 1] === "b" || s[i + 1] === "B")) {
            i += 2;
            continue;
        }
        if (c === "(" && s[i + 1] === "?") {
            if (s.slice(i, i + 3) === "(?=" || s.slice(i, i + 3) === "(?!") {
                i += 3;
            } else if (s.slice(i, i + 4) === "(?<=" || s.slice(i, i + 4) === "(?<!") {
                i += 4;
            } else {
                return false;
            }
            let depth = 1;
            while (i < s.length && depth > 0) {
                const ch = s[i];
                if (ch === "\\") {
                    i += 2;
                    continue;
                }
                if (ch === "(") depth++;
                if (ch === ")") depth--;
                i++;
            }
            if (depth !== 0) return false;
            continue;
        }
        return false;
    }
    return true;
}

/**
 * Validate a single highlight rule. Only regex mode needs syntax checking.
 * Returns null when valid; otherwise returns an error kind for the UI to map to i18n.
 */
export function validateHighlightRule(rule: HighlightRule): HighlightValidationError | null {
    if (rule.name.length > MAX_HIGHLIGHT_NAME) {
        return { kind: "name_too_long" };
    }
    if (!rule.is_regex || !rule.keyword) return null;
    const flags = rule.is_case_sensitive ? "g" : "gi";
    if (isPureZeroWidth(rule.keyword)) {
        return { kind: "zero_width" };
    }
    try {
        const re = new RegExp(rule.keyword, flags);
        if (re.test("")) {
            return { kind: "zero_width" };
        }
    } catch (e: any) {
        return { kind: "invalid", message: e?.message || String(e) };
    }
    return null;
}

/** Pre-compile highlight rules into reusable RegExp objects. Invalid rules are marked as regex=null. */
export function compileHighlightRules(rules: HighlightRule[]): CompiledHighlightRule[] {
    return rules.map((rule) => {
        if (!rule.enabled || !rule.keyword) {
            return { ...rule, source: "", regex: null };
        }
        const source = rule.is_regex
            ? rule.keyword
            : rule.keyword.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        const flags = rule.is_case_sensitive ? "g" : "gi";
        try {
            const regex = new RegExp(source, flags);
            return { ...rule, source, regex };
        } catch (e) {
            console.warn("[highlight] invalid regex, skipping:", rule.keyword, e);
            return { ...rule, source, regex: null };
        }
    });
}

export interface HighlightMatch {
    start: number;
    end: number;
    color: string;
}

interface RawMatch extends HighlightMatch {
    index: number; // rule list position, kept only to resolve overlap priority
}

/**
 * Find highlight matches in a plain-text string. Returns matches sorted by
 * start position with overlaps resolved in favour of the earlier rule in the
 * list. Callers turn these ranges into xterm decorations — this function never
 * touches the byte stream, so it cannot corrupt the program's own SGR state.
 */
export function findMatches(text: string, compiled: CompiledHighlightRule[]): HighlightMatch[] {
    const enabled = compiled.filter((c) => c.enabled && c.keyword && c.regex);
    if (!enabled.length) return [];

    const raw: RawMatch[] = [];
    for (let i = 0; i < enabled.length; i++) {
        const re = enabled[i].regex!;
        re.lastIndex = 0;
        let m: RegExpExecArray | null;
        while ((m = re.exec(text)) !== null) {
            const start = m.index;
            const end = start + m[0].length;
            if (end === start) {
                // Zero-width match: advance one char to avoid an infinite loop (defensive).
                re.lastIndex = start + 1;
                continue;
            }
            raw.push({ start, end, color: enabled[i].color, index: i });
        }
    }

    raw.sort((a, b) => (a.start !== b.start ? a.start - b.start : a.index - b.index));

    const out: HighlightMatch[] = [];
    let pos = 0;
    // Matches are sorted by start; `pos` is the end of the last kept match.
    // Any match starting before it overlaps an earlier (higher-priority) one — skip it.
    for (const m of raw) {
        if (m.start < pos) continue;
        out.push({ start: m.start, end: m.end, color: m.color });
        pos = m.end;
    }
    return out;
}

/**
 * One buffer line reduced to what the highlighter needs:
 *   - `text`: the line's visible text as a JS string (UTF-16 code units), which
 *     is what findMatches/RegExp index into
 *   - `cellAt`: cellAt[i] is the cell column of the UTF-16 unit text[i]; it has
 *     length text.length+1 so cellAt[text.length] is the end column. String
 *     offset and cell column diverge two ways: a wide (CJK) glyph spans 2
 *     columns, and one glyph (emoji/combining marks) can be several UTF-16 units
 *     that all map to the same column — this map keeps decorations on real cells.
 */
export interface LineCells {
    text: string;
    cellAt: number[];
}

/** A decoration to place on a single line: cell column, width in cells, color. */
export interface LineDecoration {
    x: number;
    width: number;
    color: string;
}

/**
 * Turn one line's matches into cell-positioned decorations. Pure: the xterm
 * coupling (reading the buffer into LineCells, registering decorations) stays
 * in the caller so this stays unit-testable.
 */
export function planLine(cells: LineCells, compiled: CompiledHighlightRule[]): LineDecoration[] {
    return findMatches(cells.text, compiled)
        .map((m) => ({
            x: cells.cellAt[m.start],
            width: cells.cellAt[m.end] - cells.cellAt[m.start],
            color: m.color,
        }))
        // A 0-width span (a match resolving to no full cell) is not a valid
        // decoration; xterm's width defaults to 1, so drop these outright.
        .filter((d) => d.width > 0);
}
