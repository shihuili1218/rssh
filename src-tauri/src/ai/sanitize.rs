//! 脱敏 + 截断 + 命令形态校验。
//!
//! 这是发往 LLM 的所有内容必经的过滤层。设计原则：
//! - 脱敏在 rssh 本地完成，不依赖 LLM 自律
//! - shape validator 不区分远端 OS（Linux/macOS/*BSD），不 hardcode "必须有 -b"
//! - 破坏性命令清单写死，LLM 提到一律拦死

use regex::Regex;
use serde::Serialize;

use super::llm::ChatMessage;

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
        // 大写 / 混合大小写 hex 也要脱敏（.NET / 某些 token 生成器输出大写 UUID/hash）。
        (r"\b[0-9a-fA-F]{32,}\b", "<REDACTED:hex>"),
    ]
    .into_iter()
    .map(|(p, r)| RedactRule::new(p, r).expect("internal redact pattern compile"))
    .collect()
}

pub fn redact(text: &str, rules: &[RedactRule]) -> String {
    let mut out = text.to_string();
    for r in rules {
        out = r
            .pattern
            .replace_all(&out, r.replacement.as_str())
            .into_owned();
    }
    out
}

/// 对一条 ChatMessage 的所有自由文本字段过 redact。
/// `tool_calls` 不动 —— 它们是 LLM 从已脱敏 history 生成的，本就不含真敏感数据；
/// 改写它会破坏前端粘贴执行的 cmd 字面（用户终端会 echo `<REDACTED:...>`）。
pub fn redact_message(msg: &ChatMessage, rules: &[RedactRule]) -> ChatMessage {
    match msg {
        ChatMessage::User { content } => ChatMessage::User {
            content: redact(content, rules),
        },
        ChatMessage::Assistant {
            content,
            tool_calls,
            reasoning_content,
        } => ChatMessage::Assistant {
            content: redact(content, rules),
            tool_calls: tool_calls.clone(),
            reasoning_content: reasoning_content.as_ref().map(|r| redact(r, rules)),
        },
        ChatMessage::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => ChatMessage::ToolResult {
            tool_call_id: tool_call_id.clone(),
            content: redact(content, rules),
            is_error: *is_error,
        },
    }
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
    text.push_str(&format!("\n... [TRUNCATED: dropped {dropped} bytes] ..."));
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
    #[error("Destructive command not allowed: {0}")]
    Destructive(String),
    #[error("Interactive screen-redrawing command requires batch flags: {0}")]
    Interactive(String),
    #[error("Loop sampling must carry an explicit count (interval count): {0}")]
    UnboundedLoop(String),
    /// 写文件相关：所有 free-form 写命令一律拒绝，文件修改唯一合法路径 = patch_file 工具。
    /// 错误信息里带上具体形态，方便 LLM 知道为何被拒。
    #[error("Write command not allowed — use patch_file: {0}")]
    Write(String),
    #[error("Empty command")]
    Empty,
}

pub const DESTRUCTIVE: &[&str] = &[
    "rm",
    "dd",
    "mkfs",
    "iptables",
    "ip6tables",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    "kill",
    "pkill",
    "killall",
    "mount",
    "umount",
    "exec",
];

pub const INTERACTIVE_BARE: &[&str] = &[
    "htop", "watch", "less", "more", "vi", "vim", "nano", "tmux", "screen", "iotop",
];

/// 这些工具没有 `interval count` 两个数字结尾就是无限循环。
pub const COUNTED_LOOP: &[&str] = &["vmstat", "iostat", "pidstat", "mpstat", "sar", "jstat"];

/// 写文件动词。这些命令的存在本身就在改文件系统状态，统一拒绝，LLM 改文件走 patch_file。
pub const WRITE_VERBS: &[&str] = &["tee", "cp", "mv", "ln", "install"];

/// 全拒的脚本解释器：python 等可以通过 `open(..., 'w')` 写文件绕过 validator。
/// 业务上 LLM 也不需要 python——读文件用 cat/grep/awk(read-only)，改文件用 patch_file。
pub const INTERPRETERS_DENIED: &[&str] = &["python", "python3", "python2"];

/// 重定向白名单：只有目标是 /dev/null 的重定向放行（保留 `cmd > /dev/null 2>&1` 这种丢弃输出用法）。
/// 标准 fd 复制（2>&1 / 1>&2）也是丢弃/复用 fd，不写文件，放行。
fn is_safe_redirect_token(t: &str) -> bool {
    matches!(
        t,
        "2>&1" | "1>&2" | ">/dev/null" | ">>/dev/null" | "&>/dev/null" | "&>>/dev/null"
    )
}

/// 扫描所有 token 找写文件重定向。返回第一个不安全的形态字符串供错误信息引用。
///
/// 识别形态：
/// - `> path` / `>> path`（带空格分隔，target 是下一个 token）
/// - `>path` / `>>path`（紧贴写在一起，token 自身含 `>`）
/// - `2>file` 等带 fd 前缀的写（token 含 `>` 但不是白名单 fd 复制）
fn find_write_redirect(tokens: &[&str]) -> Option<String> {
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];

        if is_safe_redirect_token(t) {
            i += 1;
            continue;
        }

        // 形态: token 完全等于 ">" 或 ">>"，target 在下一个 token
        if t == ">" || t == ">>" {
            let target = tokens.get(i + 1).copied().unwrap_or("");
            if target != "/dev/null" {
                return Some(format!("{t} {target}"));
            }
            i += 2;
            continue;
        }

        // 形态: token 包含 `>`（非白名单），如 ">/tmp/foo" / "2>err.log" / ">>append.log"
        // 凡含 `>` 一律拒（除非全 token 命中 is_safe_redirect_token，前面已 continue 掉）
        if t.contains('>') {
            return Some(t.to_string());
        }
        i += 1;
    }
    None
}

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

    // 1. 命令头扫描：DESTRUCTIVE / WRITE_VERBS / INTERPRETERS_DENIED
    //    任何 |, ||, &&, ;, & 之后的第一个 token 都是新命令头。
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
            if WRITE_VERBS.contains(&b) {
                return Err(ShapeError::Write(format!(
                    "{b} (file modification must go through patch_file)"
                )));
            }
            if INTERPRETERS_DENIED.contains(&b) {
                return Err(ShapeError::Write(format!(
                    "{b} (rssh blocks script interpreters; use patch_file / match_file for file work)"
                )));
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
    if first == "tail" && tokens.iter().any(|t| *t == "-f" || *t == "-F") {
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
            return Err(ShapeError::Interactive(
                "top (missing -b or -l batch flag)".into(),
            ));
        }
    }

    // 5. in-place 编辑：sed -i / awk -i inplace / perl -i (含组合短选项 -pi/-ni 等)
    if first == "sed"
        && tokens.iter().any(|t| {
            *t == "-i"
                || t.starts_with("-i")
                || *t == "--in-place"
                || t.starts_with("--in-place")
        })
    {
        return Err(ShapeError::Write(
            "sed -i (in-place edit; use patch_file)".into(),
        ));
    }
    if first == "perl"
        && tokens.iter().any(|t| {
            t.starts_with('-') && !t.starts_with("--") && t.len() > 1 && t[1..].contains('i')
        })
    {
        return Err(ShapeError::Write(
            "perl -i (in-place edit; use patch_file)".into(),
        ));
    }
    if first == "awk" {
        let mut prev_i = false;
        for t in tokens.iter().skip(1) {
            if *t == "-i" {
                prev_i = true;
                continue;
            }
            if prev_i && *t == "inplace" {
                return Err(ShapeError::Write(
                    "awk -i inplace (in-place edit; use patch_file)".into(),
                ));
            }
            prev_i = false;
        }
    }

    // 6. touch 时间戳标志：留 touch 本身合法（创建空文件），但拒所有改 mtime 形态
    if first == "touch" {
        for t in tokens.iter().skip(1) {
            let bad = matches!(*t, "-a" | "-m" | "-am" | "-ma")
                || t.starts_with("-d")
                || t.starts_with("-t")
                || t.starts_with("-r")
                || t.starts_with("--date=")
                || t.starts_with("--time=")
                || t.starts_with("--reference=");
            if bad {
                return Err(ShapeError::Write(format!(
                    "touch {t} (timestamp change; touch may only create empty files)"
                )));
            }
        }
    }

    // 7. 写文件重定向（> / >>，白名单 /dev/null）
    if let Some(form) = find_write_redirect(&tokens) {
        return Err(ShapeError::Write(format!(
            "redirect '{form}' (file modification must go through patch_file; '/dev/null' is the only allowed target)"
        )));
    }

    // 8. 循环采样必须有 ≥2 个连续数字（interval + count）
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
                "{first} requires two consecutive numbers 'interval count'"
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
        assert_eq!(
            redact("connect 10.0.0.1:8080", &rules),
            "connect <REDACTED:ip-10>:8080"
        );
        assert_eq!(redact("172.16.0.5", &rules), "<REDACTED:ip-172>");
        assert_eq!(redact("172.32.0.5", &rules), "172.32.0.5"); // 不在 16-31 范围
        assert_eq!(redact("192.168.1.1", &rules), "<REDACTED:ip-192>");
        assert_eq!(redact("8.8.8.8", &rules), "8.8.8.8"); // 公网 IP 不动
    }

    #[test]
    fn redact_tokens() {
        let rules = default_rules();
        assert!(
            redact("Bearer abcdefghijklmnopqrstuvwxyz1234", &rules).contains("<REDACTED:bearer>")
        );
        assert!(redact("key=sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ", &rules).contains("<REDACTED:sk-key>"));
        assert!(
            redact("eyJabcdefghijklmnop1234.abcdefghijklmnop1234.abc", &rules)
                .contains("<REDACTED:jwt>")
        );
    }

    #[test]
    fn redact_long_hex() {
        let rules = default_rules();
        assert!(redact("0123456789abcdef0123456789abcdef", &rules).contains("<REDACTED:hex>"));
        // 大写 / 混合大小写也必须命中（之前只匹配小写 → false negative）
        assert!(redact("0123456789ABCDEF0123456789ABCDEF", &rules).contains("<REDACTED:hex>"));
        assert!(redact("DeAdBeEfDeAdBeEfDeAdBeEfDeAdBeEf", &rules).contains("<REDACTED:hex>"));
        assert_eq!(redact("short=abc123", &rules), "short=abc123");
    }

    #[test]
    fn redact_message_user_content() {
        let rules = default_rules();
        let m = ChatMessage::User {
            content: "ssh root@10.0.0.1".into(),
        };
        match redact_message(&m, &rules) {
            ChatMessage::User { content } => assert!(content.contains("<REDACTED:ip-10>")),
            _ => panic!("variant changed"),
        }
    }

    #[test]
    fn redact_message_assistant_content_and_reasoning() {
        let rules = default_rules();
        let m = ChatMessage::Assistant {
            content: "checked 192.168.1.1".into(),
            tool_calls: vec![],
            reasoning_content: Some("thinking about 172.16.0.1".into()),
        };
        match redact_message(&m, &rules) {
            ChatMessage::Assistant {
                content,
                reasoning_content,
                ..
            } => {
                assert!(content.contains("<REDACTED:ip-192>"));
                assert!(reasoning_content.unwrap().contains("<REDACTED:ip-172>"));
            }
            _ => panic!("variant changed"),
        }
    }

    #[test]
    fn redact_message_tool_result() {
        let rules = default_rules();
        let m = ChatMessage::ToolResult {
            tool_call_id: "tc1".into(),
            content: "Bearer abcdefghijklmnopqrstuvwxyz1234".into(),
            is_error: false,
        };
        match redact_message(&m, &rules) {
            ChatMessage::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_call_id, "tc1");
                assert!(content.contains("<REDACTED:bearer>"));
                assert!(!is_error);
            }
            _ => panic!("variant changed"),
        }
    }

    /// LLM 工具调用的 input 是 LLM 从已脱敏 history 生成的；改写它会破坏
    /// 前端粘进终端的 cmd 字面。redact_message 必须保留 tool_calls 不变。
    #[test]
    fn redact_message_preserves_tool_calls() {
        let rules = default_rules();
        let tc = super::super::llm::ToolCall {
            id: "tc1".into(),
            name: "run_command".into(),
            input: serde_json::json!({"cmd": "ping 10.0.0.1"}),
        };
        let m = ChatMessage::Assistant {
            content: "running".into(),
            tool_calls: vec![tc.clone()],
            reasoning_content: None,
        };
        match redact_message(&m, &rules) {
            ChatMessage::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].input["cmd"], "ping 10.0.0.1");
            }
            _ => panic!("variant changed"),
        }
    }

    #[test]
    fn redact_message_idempotent() {
        let rules = default_rules();
        let original = ChatMessage::User {
            content: "ssh 10.0.0.1".into(),
        };
        let once = redact_message(&original, &rules);
        let twice = redact_message(&once, &rules);
        // 已脱敏内容再过一遍 redact 必须等价
        match (once, twice) {
            (ChatMessage::User { content: a }, ChatMessage::User { content: b }) => {
                assert_eq!(a, b);
            }
            _ => panic!("variant changed"),
        }
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
        assert!(matches!(
            validate("rm -rf /tmp/foo"),
            Err(ShapeError::Destructive(_))
        ));
        assert!(matches!(
            validate("kill -9 1234"),
            Err(ShapeError::Destructive(_))
        ));
        assert!(matches!(
            validate("dd if=/dev/zero of=/tmp/x"),
            Err(ShapeError::Destructive(_))
        ));
        assert!(matches!(
            validate("ps -ef && kill -9 123"),
            Err(ShapeError::Destructive(_))
        ));
        assert!(matches!(
            validate("foo | rm -rf /"),
            Err(ShapeError::Destructive(_))
        ));
    }

    #[test]
    fn shape_chmod_recursive_blocked() {
        assert!(matches!(
            validate("chmod -R 755 /"),
            Err(ShapeError::Destructive(_))
        ));
        assert!(validate("chmod 755 /tmp/foo").is_ok());
    }

    #[test]
    fn shape_fork_bomb() {
        assert!(matches!(
            validate(":(){:|:&};:"),
            Err(ShapeError::Destructive(_))
        ));
        assert!(matches!(
            validate(":(){ :|:& };:"),
            Err(ShapeError::Destructive(_))
        ));
    }

    #[test]
    fn shape_top() {
        assert!(matches!(validate("top"), Err(ShapeError::Interactive(_))));
        assert!(matches!(
            validate("top -d 1"),
            Err(ShapeError::Interactive(_))
        )); // -d 不是批处理
        assert!(validate("top -bn1").is_ok());
        assert!(validate("top -b -n 1").is_ok());
        assert!(validate("top -l 1 -n 20").is_ok()); // macOS
    }

    #[test]
    fn shape_unbounded_loop() {
        assert!(matches!(
            validate("vmstat 1"),
            Err(ShapeError::UnboundedLoop(_))
        ));
        assert!(matches!(
            validate("vmstat -t 1"),
            Err(ShapeError::UnboundedLoop(_))
        ));
        assert!(validate("vmstat 1 5").is_ok());
        assert!(validate("vmstat -t 1 5").is_ok());
        assert!(validate("jstat -gcutil 1234 1000 10").is_ok());
        assert!(validate("pidstat -p 1234 1 5").is_ok());
    }

    #[test]
    fn shape_tail_follow_blocked() {
        assert!(matches!(
            validate("tail -f /var/log/messages"),
            Err(ShapeError::Interactive(_))
        ));
        assert!(matches!(
            validate("tail -F /var/log/messages"),
            Err(ShapeError::Interactive(_))
        ));
        assert!(validate("tail -n 100 /var/log/messages").is_ok());
    }

    #[test]
    fn shape_exec_blocked() {
        assert!(matches!(
            validate("exec foo"),
            Err(ShapeError::Destructive(_))
        ));
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
        assert!(matches!(
            validate("/bin/rm /tmp/foo"),
            Err(ShapeError::Destructive(_))
        ));
        assert!(matches!(
            validate("/usr/bin/kill 1"),
            Err(ShapeError::Destructive(_))
        ));
    }

    // ─── 写命令拦截 ─────────────────────────────────────────────────

    #[test]
    fn shape_write_verbs_blocked() {
        assert!(matches!(validate("tee /tmp/foo"), Err(ShapeError::Write(_))));
        assert!(matches!(validate("cp a b"), Err(ShapeError::Write(_))));
        assert!(matches!(validate("mv a b"), Err(ShapeError::Write(_))));
        assert!(matches!(
            validate("ln -s a b"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("install -m 644 a b"),
            Err(ShapeError::Write(_))
        ));
        // 通过路径前缀仍要拒
        assert!(matches!(validate("/bin/cp a b"), Err(ShapeError::Write(_))));
        // 管道后第二个命令是写动词也要拒
        assert!(matches!(
            validate("cat x | tee /tmp/y"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("ls && cp a b"),
            Err(ShapeError::Write(_))
        ));
    }

    #[test]
    fn shape_interpreters_blocked() {
        assert!(matches!(
            validate("python -c 'open(\"x\",\"w\")'"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("python3 script.py"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("python2 -m foo"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("/usr/bin/python3 -"),
            Err(ShapeError::Write(_))
        ));
        // 管道后位也要拒
        assert!(matches!(
            validate("echo x | python3 -"),
            Err(ShapeError::Write(_))
        ));
    }

    #[test]
    fn shape_inplace_edit_blocked() {
        // sed -i
        assert!(matches!(
            validate("sed -i 's/a/b/' file"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("sed -i.bak 's/a/b/' file"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("sed --in-place 's/a/b/' file"),
            Err(ShapeError::Write(_))
        ));
        // sed 没有 -i 仍是 read-only 用法
        assert!(validate("sed 's/a/b/' file").is_ok());
        assert!(validate("sed -n '1,10p' file").is_ok());

        // awk -i inplace
        assert!(matches!(
            validate("awk -i inplace '{print}' file"),
            Err(ShapeError::Write(_))
        ));
        assert!(validate("awk '{print $1}' file").is_ok());

        // perl -i / -pi / -nie / -i.bak
        assert!(matches!(
            validate("perl -i -pe 's/a/b/' file"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("perl -pi -e 's/a/b/' file"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("perl -i.bak -pe 's/a/b/' file"),
            Err(ShapeError::Write(_))
        ));
        // perl -ne / -e 是 read-only
        assert!(validate("perl -ne 'print if /foo/' file").is_ok());
        assert!(validate("perl -e 'print 1'").is_ok());
    }

    #[test]
    fn shape_touch_timestamp_blocked() {
        // -a / -m / -am 改 mtime/atime
        assert!(matches!(
            validate("touch -a file"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("touch -m file"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("touch -am file"),
            Err(ShapeError::Write(_))
        ));
        // -d / -t / -r
        assert!(matches!(
            validate("touch -d '2026-01-01' file"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("touch -t 202601011200 file"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("touch -r refer file"),
            Err(ShapeError::Write(_))
        ));
        // long options
        assert!(matches!(
            validate("touch --date=2026-01-01 file"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("touch --reference=ref file"),
            Err(ShapeError::Write(_))
        ));
        // 创建空文件放行
        assert!(validate("touch /tmp/foo").is_ok());
        assert!(validate("touch a b c").is_ok());
    }

    #[test]
    fn shape_write_redirect_blocked() {
        // 带空格的 > / >>
        assert!(matches!(
            validate("echo hi > /tmp/foo"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("echo hi >> /tmp/foo"),
            Err(ShapeError::Write(_))
        ));
        // 紧贴形态
        assert!(matches!(
            validate("echo hi >/tmp/foo"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("echo hi >>/tmp/foo"),
            Err(ShapeError::Write(_))
        ));
        // fd 重定向写文件
        assert!(matches!(
            validate("cmd 2> err.log"),
            Err(ShapeError::Write(_))
        ));
        assert!(matches!(
            validate("cmd 2>err.log"),
            Err(ShapeError::Write(_))
        ));
        // bash &> 写文件
        assert!(matches!(
            validate("cmd &> out.log"),
            Err(ShapeError::Write(_))
        ));
    }

    #[test]
    fn shape_devnull_passes() {
        // /dev/null 是唯一放行的写目标
        assert!(validate("cmd > /dev/null").is_ok());
        assert!(validate("cmd >/dev/null").is_ok());
        assert!(validate("cmd >> /dev/null").is_ok());
        assert!(validate("cmd >>/dev/null").is_ok());
        assert!(validate("cmd > /dev/null 2>&1").is_ok());
        assert!(validate("cmd >/dev/null 2>&1").is_ok());
        assert!(validate("cmd 2>&1 | grep foo").is_ok());
        assert!(validate("cmd &>/dev/null").is_ok());
        // 1>&2 fd 复制（不写文件）
        assert!(validate("echo err 1>&2").is_ok());
    }

    #[test]
    fn shape_pipe_preserves_read_only() {
        // 写动词在管道头/尾都要拒，但纯读管道要放行
        assert!(validate("ps -ef | grep java | head -10").is_ok());
        assert!(validate("cat /etc/hosts | wc -l").is_ok());
        assert!(validate("ls -la | sort").is_ok());
    }
}
