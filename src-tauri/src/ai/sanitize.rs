//! 脱敏 + 截断 + 命令形态校验。
//!
//! 这是发往 LLM 的所有内容必经的过滤层。设计原则：
//! - 脱敏在 rssh 本地完成，不依赖 LLM 自律
//! - shape validator 不区分远端 OS（Linux/macOS/*BSD），不 hardcode "必须有 -b"
//! - 破坏性命令清单写死，LLM 提到一律拦死

use regex::Regex;
use serde::Serialize;

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 1_048_576; // 1 MB

// ─── 脱敏 ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RedactRule {
    pub pattern: Regex,
    pub replacement: String,
}

impl RedactRule {
    pub fn new(pattern: &str, replacement: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            pattern: Regex::new(pattern)?,
            replacement: replacement.to_string(),
        })
    }
}

/// 默认脱敏规则集。设计文档 1.2 节列出。
pub fn default_rules() -> Vec<RedactRule> {
    [
        (r"\b10\.\d{1,3}\.\d{1,3}\.\d{1,3}\b", "<REDACTED:ip-10>"),
        (
            r"\b172\.(1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3}\b",
            "<REDACTED:ip-172>",
        ),
        (r"\b192\.168\.\d{1,3}\.\d{1,3}\b", "<REDACTED:ip-192>"),
        (r"Bearer [A-Za-z0-9_\-\.]{20,}", "<REDACTED:bearer>"),
        (r"sk-[A-Za-z0-9_\-]{20,}", "<REDACTED:sk-key>"),
        (
            r"eyJ[A-Za-z0-9_\-]{20,}\.[A-Za-z0-9_\-]{20,}\.[A-Za-z0-9_\-]+",
            "<REDACTED:jwt>",
        ),
        (r"\b[0-9a-f]{32,}\b", "<REDACTED:hex>"),
    ]
    .into_iter()
    .map(|(p, r)| RedactRule::new(p, r).expect("internal redact pattern compile"))
    .collect()
}

pub fn redact(text: &str, rules: &[RedactRule]) -> String {
    let mut out = text.to_string();
    for r in rules {
        out = r.pattern.replace_all(&out, r.replacement.as_str()).into_owned();
    }
    out
}

// ─── 字符编码 + 截断 ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct Truncated {
    pub text: String,
    pub original_bytes: usize,
    pub truncated_bytes: usize,
}

pub fn decode_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 头部完整保留 + 尾部截断。在 char boundary 切，避免截掉半个 UTF-8 字符。
pub fn truncate(input: &str, max_bytes: usize) -> Truncated {
    if input.len() <= max_bytes {
        return Truncated {
            text: input.to_string(),
            original_bytes: input.len(),
            truncated_bytes: 0,
        };
    }
    let mut cut = max_bytes;
    while cut > 0 && !input.is_char_boundary(cut) {
        cut -= 1;
    }
    let head = &input[..cut];
    let dropped = input.len() - cut;
    let mut text = head.to_string();
    text.push_str(&format!(
        "\n... [TRUNCATED: 截断了 {dropped} 字节] ..."
    ));
    Truncated {
        text,
        original_bytes: input.len(),
        truncated_bytes: dropped,
    }
}

// ─── Shape validator ─────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShapeError {
    #[error("禁止的破坏性命令: {0}")]
    Destructive(String),
    #[error("交互式刷屏命令必须用批处理形态: {0}")]
    Interactive(String),
    #[error("循环采样必须显式带次数（interval count）: {0}")]
    UnboundedLoop(String),
    #[error("空命令")]
    Empty,
}

pub const DESTRUCTIVE: &[&str] = &[
    "rm", "dd", "mkfs", "iptables", "ip6tables", "shutdown", "reboot", "halt", "poweroff",
    "kill", "pkill", "killall", "mount", "umount", "exec",
];

pub const INTERACTIVE_BARE: &[&str] = &[
    "htop", "watch", "less", "more", "vi", "vim", "nano", "tmux", "screen", "iotop",
];

/// 这些工具没有 `interval count` 两个数字结尾就是无限循环。
pub const COUNTED_LOOP: &[&str] = &["vmstat", "iostat", "pidstat", "mpstat", "sar", "jstat"];

fn bare(t: &str) -> &str {
    t.rsplit('/').next().unwrap_or(t)
}

pub fn validate(cmd: &str) -> Result<(), ShapeError> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return Err(ShapeError::Empty);
    }

    // fork bomb 形态 — 在 token 化之前查（特殊字符不便 split）
    let no_space: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if no_space.contains(":(){:|:&};:") {
        return Err(ShapeError::Destructive("fork bomb".into()));
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return Err(ShapeError::Empty);
    }

    // 命令名提取（去路径前缀、扔掉管道/重定向等元字符 token）
    let first = bare(tokens[0]);

    // 1. 破坏性命令名（含管道 / && / ; / | 后第一个 token）
    let mut at_command_head = true;
    for t in &tokens {
        if matches!(*t, "|" | "||" | "&&" | ";" | "&") {
            at_command_head = true;
            continue;
        }
        if at_command_head {
            let b = bare(t);
            if DESTRUCTIVE.contains(&b) {
                return Err(ShapeError::Destructive(b.to_string()));
            }
            at_command_head = false;
        }
    }

    // 2. chmod -R / chown -R
    if (first == "chmod" || first == "chown")
        && tokens
            .iter()
            .any(|t| t.starts_with("-R") || *t == "--recursive")
    {
        return Err(ShapeError::Destructive(format!("{first} -R")));
    }

    // 3. tail -f / -F
    if first == "tail"
        && tokens.iter().any(|t| *t == "-f" || *t == "-F")
    {
        return Err(ShapeError::Interactive("tail -f".into()));
    }

    // 4. 单独的交互式命令 / 没批处理标志的 top
    if INTERACTIVE_BARE.contains(&first) && tokens.len() == 1 {
        return Err(ShapeError::Interactive(first.to_string()));
    }
    if first == "top" {
        // Linux: -b -n N    macOS: -l N    放过任一形态
        let has_batch = tokens
            .iter()
            .skip(1)
            .any(|t| t.starts_with("-b") || t.starts_with("-l"));
        if !has_batch {
            return Err(ShapeError::Interactive("top（缺 -b 或 -l 批处理标志）".into()));
        }
    }

    // 5. 循环采样必须有 ≥2 个连续数字（interval + count）
    if COUNTED_LOOP.contains(&first) {
        let mut consecutive: u32 = 0;
        let mut maxc: u32 = 0;
        for t in tokens.iter().skip(1) {
            if t.parse::<u64>().is_ok() {
                consecutive += 1;
                maxc = maxc.max(consecutive);
            } else {
                consecutive = 0;
            }
        }
        if maxc < 2 {
            return Err(ShapeError::UnboundedLoop(format!(
                "{first} 需要 'interval count' 两个连续数字"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_internal_ips() {
        let rules = default_rules();
        assert_eq!(redact("connect 10.0.0.1:8080", &rules), "connect <REDACTED:ip-10>:8080");
        assert_eq!(redact("172.16.0.5", &rules), "<REDACTED:ip-172>");
        assert_eq!(redact("172.32.0.5", &rules), "172.32.0.5"); // 不在 16-31 范围
        assert_eq!(redact("192.168.1.1", &rules), "<REDACTED:ip-192>");
        assert_eq!(redact("8.8.8.8", &rules), "8.8.8.8"); // 公网 IP 不动
    }

    #[test]
    fn redact_tokens() {
        let rules = default_rules();
        assert!(redact("Bearer abcdefghijklmnopqrstuvwxyz1234", &rules).contains("<REDACTED:bearer>"));
        assert!(redact("key=sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ", &rules).contains("<REDACTED:sk-key>"));
        assert!(
            redact(
                "eyJabcdefghijklmnop1234.abcdefghijklmnop1234.abc",
                &rules
            )
            .contains("<REDACTED:jwt>")
        );
    }

    #[test]
    fn redact_long_hex() {
        let rules = default_rules();
        assert!(redact("0123456789abcdef0123456789abcdef", &rules).contains("<REDACTED:hex>"));
        assert_eq!(redact("short=abc123", &rules), "short=abc123");
    }

    #[test]
    fn truncate_preserves_head() {
        let big = "a".repeat(2_000_000);
        let r = truncate(&big, 1_000_000);
        assert!(r.text.starts_with(&"a".repeat(1_000_000)));
        assert!(r.text.contains("TRUNCATED"));
        assert_eq!(r.original_bytes, 2_000_000);
        assert_eq!(r.truncated_bytes, 1_000_000);
    }

    #[test]
    fn truncate_passthrough_short() {
        let r = truncate("hello", 100);
        assert_eq!(r.text, "hello");
        assert_eq!(r.truncated_bytes, 0);
    }

    #[test]
    fn shape_destructive() {
        assert!(matches!(validate("rm -rf /tmp/foo"), Err(ShapeError::Destructive(_))));
        assert!(matches!(validate("kill -9 1234"), Err(ShapeError::Destructive(_))));
        assert!(matches!(validate("dd if=/dev/zero of=/tmp/x"), Err(ShapeError::Destructive(_))));
        assert!(matches!(validate("ps -ef && kill -9 123"), Err(ShapeError::Destructive(_))));
        assert!(matches!(validate("foo | rm -rf /"), Err(ShapeError::Destructive(_))));
    }

    #[test]
    fn shape_chmod_recursive_blocked() {
        assert!(matches!(validate("chmod -R 755 /"), Err(ShapeError::Destructive(_))));
        assert!(validate("chmod 755 /tmp/foo").is_ok());
    }

    #[test]
    fn shape_fork_bomb() {
        assert!(matches!(validate(":(){:|:&};:"), Err(ShapeError::Destructive(_))));
        assert!(matches!(validate(":(){ :|:& };:"), Err(ShapeError::Destructive(_))));
    }

    #[test]
    fn shape_top() {
        assert!(matches!(validate("top"), Err(ShapeError::Interactive(_))));
        assert!(matches!(validate("top -d 1"), Err(ShapeError::Interactive(_)))); // -d 不是批处理
        assert!(validate("top -bn1").is_ok());
        assert!(validate("top -b -n 1").is_ok());
        assert!(validate("top -l 1 -n 20").is_ok()); // macOS
    }

    #[test]
    fn shape_unbounded_loop() {
        assert!(matches!(validate("vmstat 1"), Err(ShapeError::UnboundedLoop(_))));
        assert!(matches!(validate("vmstat -t 1"), Err(ShapeError::UnboundedLoop(_))));
        assert!(validate("vmstat 1 5").is_ok());
        assert!(validate("vmstat -t 1 5").is_ok());
        assert!(validate("jstat -gcutil 1234 1000 10").is_ok());
        assert!(validate("pidstat -p 1234 1 5").is_ok());
    }

    #[test]
    fn shape_tail_follow_blocked() {
        assert!(matches!(validate("tail -f /var/log/messages"), Err(ShapeError::Interactive(_))));
        assert!(matches!(validate("tail -F /var/log/messages"), Err(ShapeError::Interactive(_))));
        assert!(validate("tail -n 100 /var/log/messages").is_ok());
    }

    #[test]
    fn shape_exec_blocked() {
        assert!(matches!(validate("exec foo"), Err(ShapeError::Destructive(_))));
    }

    #[test]
    fn shape_empty() {
        assert!(matches!(validate(""), Err(ShapeError::Empty)));
        assert!(matches!(validate("   "), Err(ShapeError::Empty)));
    }

    #[test]
    fn shape_benign_passes() {
        assert!(validate("uname -a").is_ok());
        assert!(validate("ps -eo pid,pcpu,comm --sort=-pcpu | head -20").is_ok());
        assert!(validate("free -h").is_ok());
        assert!(validate("cat /etc/os-release").is_ok());
        assert!(validate("jstack 1234").is_ok());
        assert!(validate("which java").is_ok());
    }

    #[test]
    fn shape_destructive_with_path() {
        assert!(matches!(validate("/bin/rm /tmp/foo"), Err(ShapeError::Destructive(_))));
        assert!(matches!(validate("/usr/bin/kill 1"), Err(ShapeError::Destructive(_))));
    }
}
