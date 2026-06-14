import type { HighlightRule } from "../stores/app.svelte.ts";

const RST = "\x1b[0m";

/** Hex color → ANSI 24-bit true color escape. */
export function ansiColor(hex: string): string {
    const h = hex.replace("#", "");
    if (h.length !== 6) return "";
    const r = parseInt(h.slice(0, 2), 16);
    const g = parseInt(h.slice(2, 4), 16);
    const b = parseInt(h.slice(4, 6), 16);
    return `\x1b[38;2;${r};${g};${b}m`;
}

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

interface Match {
    start: number;
    end: number;
    color: string;
    index: number;
}

/**
 * Apply highlight rules to a plain-text segment (no ANSI escape sequences).
 * Rules are processed in list order; when multiple rules match the same start
 * position, the first one wins and overlapping matches are skipped.
 */
export function highlightPlain(plain: string, compiled: CompiledHighlightRule[]): string {
    const enabled = compiled.filter((c) => c.enabled && c.keyword && c.regex);
    if (!enabled.length) return plain;

    const matches: Match[] = [];

    for (let i = 0; i < enabled.length; i++) {
        const rule = enabled[i];
        const re = rule.regex!;
        re.lastIndex = 0;
        let m: RegExpExecArray | null;
        while ((m = re.exec(plain)) !== null) {
            const start = m.index;
            const end = start + m[0].length;
            if (end === start) {
                // Zero-width match: advance one char to avoid an infinite loop (defensive).
                re.lastIndex = start + 1;
                continue;
            }
            matches.push({ start, end, color: rule.color, index: i });
        }
    }

    matches.sort((a, b) => {
        if (a.start !== b.start) return a.start - b.start;
        return a.index - b.index;
    });

    const parts: string[] = [];
    let pos = 0;

    // Matches are sorted by start; `pos` is the end of the last emitted match.
    // Any match starting before it overlaps an earlier (higher-priority) one — skip it.
    for (const m of matches) {
        if (m.start < pos) continue;
        parts.push(plain.slice(pos, m.start));
        parts.push(ansiColor(m.color));
        parts.push(plain.slice(m.start, m.end));
        parts.push(RST);
        pos = m.end;
    }

    parts.push(plain.slice(pos));
    return parts.join("");
}
