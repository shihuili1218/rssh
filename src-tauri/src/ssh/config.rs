use serde::Serialize;

/// 从 ~/.ssh/config 解析出 SSH 连接配置。
#[derive(Debug, Clone, Serialize)]
pub struct SshConfigEntry {
    pub host_alias: String,
    pub hostname: String,
    pub port: u16,
    pub user: Option<String>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
}

/// 解析 SSH config 文件，返回所有非通配符条目。
pub fn parse(content: &str) -> Vec<SshConfigEntry> {
    let mut entries = Vec::new();
    let mut current: Option<SshConfigEntry> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = match line.split_once(char::is_whitespace) {
            Some((k, v)) => (k.to_lowercase(), v.trim().to_string()),
            None => continue,
        };

        match key.as_str() {
            "host" => {
                if let Some(entry) = current.take() {
                    if !entry.host_alias.contains('*') {
                        entries.push(entry);
                    }
                }
                let alias = value
                    .split_whitespace()
                    .next()
                    .unwrap_or(&value)
                    .to_string();
                current = Some(SshConfigEntry {
                    host_alias: alias,
                    hostname: String::new(),
                    port: 22,
                    user: None,
                    identity_file: None,
                    proxy_jump: None,
                });
            }
            "hostname" => {
                if let Some(ref mut entry) = current {
                    entry.hostname = value;
                }
            }
            "port" => {
                if let Some(ref mut entry) = current {
                    entry.port = value.parse().unwrap_or(22);
                }
            }
            "user" => {
                if let Some(ref mut entry) = current {
                    entry.user = Some(value);
                }
            }
            "identityfile" => {
                if let Some(ref mut entry) = current {
                    let expanded = expand_tilde(&value);
                    entry.identity_file = Some(expanded);
                }
            }
            "proxyjump" => {
                if let Some(ref mut entry) = current {
                    entry.proxy_jump = Some(value);
                }
            }
            _ => {}
        }
    }

    if let Some(entry) = current {
        if !entry.host_alias.contains('*') {
            entries.push(entry);
        }
    }

    entries
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_entry() {
        let cfg = "\
Host alpha
    HostName 10.0.0.1
    User root
    Port 2222
";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.host_alias, "alpha");
        assert_eq!(e.hostname, "10.0.0.1");
        assert_eq!(e.port, 2222);
        assert_eq!(e.user.as_deref(), Some("root"));
        assert!(e.identity_file.is_none());
        assert!(e.proxy_jump.is_none());
    }

    #[test]
    fn parse_default_port_when_missing() {
        let cfg = "Host bare\n    HostName x.example\n";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].port, 22);
    }

    #[test]
    fn parse_invalid_port_falls_back_to_22() {
        // 实现里 `value.parse().unwrap_or(22)` — 坏值不应让进程炸
        let cfg = "Host bad\n    HostName x\n    Port not-a-number\n";
        let entries = parse(cfg);
        assert_eq!(entries[0].port, 22);
    }

    #[test]
    fn parse_skips_wildcard_hosts() {
        let cfg = "\
Host *
    User defaultuser

Host real
    HostName real.example
";
        let entries = parse(cfg);
        // 通配符 `*` 整段丢掉，只保留 real
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].host_alias, "real");
    }

    #[test]
    fn parse_first_alias_when_host_has_multiple_tokens() {
        // OpenSSH 允许 `Host a b c` 复用同一段配置；这里只取第一个 alias
        let cfg = "Host alpha beta gamma\n    HostName x\n";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].host_alias, "alpha");
    }

    #[test]
    fn parse_proxy_jump_and_identity_file() {
        let cfg = "\
Host inner
    HostName inner.local
    ProxyJump bastion
    IdentityFile ~/.ssh/work_ed25519
";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.proxy_jump.as_deref(), Some("bastion"));
        // ~/ 必须被展开为绝对路径（home 在 CI 里可能不同，所以只校验形态）
        let id = e.identity_file.as_deref().unwrap();
        assert!(!id.starts_with("~/"));
        assert!(id.ends_with("/.ssh/work_ed25519") || id.ends_with("\\.ssh\\work_ed25519"));
    }

    #[test]
    fn parse_ignores_comments_and_blank_lines() {
        let cfg = "\
# top-level comment

Host x
    # inline-ish
    HostName host.example
    Port 22

";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hostname, "host.example");
    }

    #[test]
    fn parse_multiple_entries_independent() {
        let cfg = "\
Host one
    HostName 1.example
    User a

Host two
    HostName 2.example
    User b
    Port 2202
";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].host_alias, "one");
        assert_eq!(entries[0].port, 22);
        assert_eq!(entries[1].host_alias, "two");
        assert_eq!(entries[1].port, 2202);
        assert_eq!(entries[1].user.as_deref(), Some("b"));
    }

    #[test]
    fn parse_keys_are_case_insensitive() {
        // OpenSSH 关键字大小写无关，实现里 `key.to_lowercase()`
        let cfg = "HOST alpha\n    HOSTNAME 1.2.3.4\n    PORT 33\n";
        let entries = parse(cfg);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hostname, "1.2.3.4");
        assert_eq!(entries[0].port, 33);
    }

    #[test]
    fn parse_empty_input() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n# only comment\n").is_empty());
    }
}
