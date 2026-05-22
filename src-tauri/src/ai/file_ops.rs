//! 文件修改工具（match_file / patch_file）的纯文本处理逻辑。
//!
//! 这里只做字符串处理，不碰 PTY 通道、不发 emit、不持有 session 状态——
//! 方便 unit test 覆盖边界条件（多字节字符、跨行 find、找不到、文件首/末等）。

use serde::Serialize;
use similar::TextDiff;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MatchEntry {
    /// find 起始字符所在的 1-based 行号
    pub line: usize,
    /// 命中前后 `before` / `after` 字符的上下文（含 find 本身）
    pub context: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MatchResult {
    pub count: usize,
    pub matches: Vec<MatchEntry>,
}

/// 从 byte 索引 `idx` 往回取 `n` 个字符，返回 char-boundary 上的 byte 索引。
/// `n=0` 直接返回 idx。不足 n 个字符则返回 0。
fn char_back(s: &str, idx: usize, n: usize) -> usize {
    if n == 0 {
        return idx;
    }
    let prefix = &s[..idx];
    prefix
        .char_indices()
        .rev()
        .nth(n - 1)
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// 从 byte 索引 `idx` 往后取 `n` 个字符，返回 char-boundary 上的 byte 索引（不含）。
/// `n=0` 直接返回 idx。不足 n 个字符则返回 s.len()。
fn char_forward(s: &str, idx: usize, n: usize) -> usize {
    if n == 0 {
        return idx;
    }
    let suffix = &s[idx..];
    suffix
        .char_indices()
        .nth(n)
        .map(|(i, _)| idx + i)
        .unwrap_or(s.len())
}

/// 在 `text` 中字面查找 `find` 的所有出现位置，返回每个匹配的行号 + 上下文。
///
/// - 字面匹配（不是 regex）；`find` 可以含 `\n` 多行
/// - 匹配从上一次匹配的**结尾之后**继续，避免重叠
/// - 上下文按字符截取（不会切坏 UTF-8）
/// - `find` 在前/后边界时上下文自动 clamp
pub fn collect_matches(text: &str, find: &str, before: usize, after: usize) -> MatchResult {
    let mut matches = Vec::new();
    if find.is_empty() {
        return MatchResult {
            count: 0,
            matches: Vec::new(),
        };
    }
    let mut search_start = 0;
    while search_start <= text.len() {
        let Some(idx) = text[search_start..].find(find) else {
            break;
        };
        let pos = search_start + idx;
        let end = pos + find.len();
        let line = text[..pos].matches('\n').count() + 1;
        let pre = char_back(text, pos, before);
        let post = char_forward(text, end, after);
        matches.push(MatchEntry {
            line,
            context: text[pre..post].to_string(),
        });
        // 不允许重叠：跳到本次匹配的结尾继续
        search_start = end;
    }
    MatchResult {
        count: matches.len(),
        matches,
    }
}

/// 生成 git-style unified diff（3 行上下文）。用于 patch_file 的审批 UI + 推回 LLM。
pub fn compute_unified_diff(path: &str, old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    diff.unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── 私有辅助函数的直接测试 ─────────────────────────────────────

    #[test]
    fn char_back_returns_idx_when_n_zero() {
        assert_eq!(char_back("hello", 3, 0), 3);
    }

    #[test]
    fn char_back_clamps_at_zero() {
        // 不足 n 个字符 → 返回 0
        assert_eq!(char_back("hi", 2, 100), 0);
    }

    #[test]
    fn char_back_one_char_basic() {
        // "abc"[..2] = "ab"，往回数 1 char 起点 = idx 1
        assert_eq!(char_back("abc", 2, 1), 1);
    }

    #[test]
    fn char_back_multibyte() {
        // "中文ab" — '中' 3 bytes, '文' 3 bytes, 'a' 1, 'b' 1
        // 总长 8 bytes，char_count 4
        // idx=8（末尾），往回 1 char → 'b' 起点 = byte 7
        assert_eq!(char_back("中文ab", 8, 1), 7);
        // 往回 2 → 'a' 起点 = byte 6
        assert_eq!(char_back("中文ab", 8, 2), 6);
        // 往回 3 → '文' 起点 = byte 3
        assert_eq!(char_back("中文ab", 8, 3), 3);
        // 往回 4 → '中' 起点 = byte 0
        assert_eq!(char_back("中文ab", 8, 4), 0);
        // 不足 → 0
        assert_eq!(char_back("中文ab", 8, 5), 0);
    }

    #[test]
    fn char_forward_returns_idx_when_n_zero() {
        assert_eq!(char_forward("hello", 2, 0), 2);
    }

    #[test]
    fn char_forward_clamps_at_end() {
        assert_eq!(char_forward("hi", 0, 100), 2);
    }

    #[test]
    fn char_forward_one_char_basic() {
        assert_eq!(char_forward("abc", 0, 1), 1);
    }

    #[test]
    fn char_forward_multibyte() {
        // "ab中文" — 'a' 1, 'b' 1, '中' 3, '文' 3
        // idx=0，往后 1 → 'b' 起点 = 1
        assert_eq!(char_forward("ab中文", 0, 1), 1);
        // 往后 2 → '中' 起点 = 2
        assert_eq!(char_forward("ab中文", 0, 2), 2);
        // 往后 3 → '文' 起点 = 5
        assert_eq!(char_forward("ab中文", 0, 3), 5);
        // 往后 4 → 末尾 = 8
        assert_eq!(char_forward("ab中文", 0, 4), 8);
        // 超出 → 末尾
        assert_eq!(char_forward("ab中文", 0, 99), 8);
    }

    // ─── collect_matches 行为测试 ───────────────────────────────────

    #[test]
    fn single_match_basic() {
        let r = collect_matches("hello world", "world", 5, 5);
        assert_eq!(r.count, 1);
        assert_eq!(r.matches[0].line, 1);
        assert_eq!(r.matches[0].context, "ello world");
    }

    #[test]
    fn multi_match() {
        let r = collect_matches("foo\nbar\nfoo\nfoo", "foo", 0, 0);
        assert_eq!(r.count, 3);
        assert_eq!(r.matches[0].line, 1);
        assert_eq!(r.matches[1].line, 3);
        assert_eq!(r.matches[2].line, 4);
    }

    #[test]
    fn multiline_find() {
        let text = "line1\nABC\nDEF\nline4";
        let r = collect_matches(text, "ABC\nDEF", 0, 0);
        assert_eq!(r.count, 1);
        assert_eq!(r.matches[0].line, 2);
        assert_eq!(r.matches[0].context, "ABC\nDEF");
    }

    #[test]
    fn empty_find_returns_zero() {
        let r = collect_matches("anything", "", 5, 5);
        assert_eq!(r.count, 0);
    }

    #[test]
    fn no_match() {
        let r = collect_matches("hello", "world", 5, 5);
        assert_eq!(r.count, 0);
    }

    #[test]
    fn context_clamps_at_file_start() {
        let r = collect_matches("abc", "abc", 100, 100);
        assert_eq!(r.count, 1);
        assert_eq!(r.matches[0].context, "abc");
    }

    #[test]
    fn context_clamps_at_file_end() {
        let r = collect_matches("hello", "hello", 5, 100);
        assert_eq!(r.count, 1);
        assert_eq!(r.matches[0].context, "hello");
    }

    #[test]
    fn multibyte_chars_do_not_split() {
        // 中文字符是 3 字节，但 context 按 char 数算，不会切坏字符
        let text = "前缀文本目标字符串后缀";
        let r = collect_matches(text, "目标", 2, 2);
        assert_eq!(r.count, 1);
        // before=2 chars: "文本", find: "目标", after=2 chars: "字符"
        assert_eq!(r.matches[0].context, "文本目标字符");
    }

    #[test]
    fn overlapping_skipped() {
        // "aaaa" 找 "aa" — 不允许重叠，应该是 2 个匹配 (pos 0, pos 2)
        let r = collect_matches("aaaa", "aa", 0, 0);
        assert_eq!(r.count, 2);
    }

    #[test]
    fn line_number_after_crlf_or_lf() {
        let r = collect_matches("aaa\nbbb\nccc\nfoo", "foo", 0, 0);
        assert_eq!(r.matches[0].line, 4);
    }

    #[test]
    fn context_zero_returns_find_only() {
        let r = collect_matches("xxxYYYzzz", "YYY", 0, 0);
        assert_eq!(r.count, 1);
        assert_eq!(r.matches[0].context, "YYY");
    }

    #[test]
    fn diff_basic_change() {
        let d = compute_unified_diff("foo.txt", "alpha\nbeta\ngamma\n", "alpha\nBETA\ngamma\n");
        assert!(d.contains("--- a/foo.txt"));
        assert!(d.contains("+++ b/foo.txt"));
        assert!(d.contains("-beta"));
        assert!(d.contains("+BETA"));
    }

    #[test]
    fn diff_no_change() {
        let d = compute_unified_diff("foo.txt", "alpha\nbeta\n", "alpha\nbeta\n");
        // 完全相同的内容 → diff 只输出 header 或空（不同 similar 版本行为略异，
        // 但绝不会出现 +/-/@ 这种 hunk 标记）
        assert!(!d.contains("\n-") && !d.contains("\n+") && !d.contains("\n@"));
    }

    #[test]
    fn diff_deletion() {
        let d = compute_unified_diff("foo.txt", "line1\nline2\nline3\n", "line1\nline3\n");
        assert!(d.contains("-line2"));
    }

    #[test]
    fn yaml_like_block() {
        // 真实场景模拟：prometheus.yml 里删一段 job
        let text = "scrape_configs:\n  - job_name: foo\n    targets: ['a']\n  - job_name: bullish-test-btc-1\n    targets: ['b']\n  - job_name: bar\n    targets: ['c']\n";
        let r = collect_matches(text, "  - job_name: bullish-test-btc-1\n    targets: ['b']\n", 20, 20);
        assert_eq!(r.count, 1);
        assert_eq!(r.matches[0].line, 4);
        assert!(r.matches[0].context.contains("bullish-test-btc-1"));
    }
}
