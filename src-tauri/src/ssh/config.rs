use serde::{Deserialize, Serialize};

/// 从 ~/.ssh/config 解析出 SSH 连接配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Maximum `Include` nesting depth — same cap as OpenSSH's
/// MAX_INCLUDE_DEPTH. Also bounds self/cyclic includes.
const MAX_INCLUDE_DEPTH: usize = 16;

/// Read an ssh config file and splice the contents of `Include` directives
/// in place, recursively. `base` is the directory non-absolute patterns
/// resolve against (OpenSSH semantics: `~/.ssh` for user configs).
///
/// Missing or unreadable included files are skipped, like a glob with no
/// matches. IO errors on the root file propagate to the caller.
pub fn load_with_includes(path: &std::path::Path, base: &std::path::Path) -> std::io::Result<String> {
    let content = std::fs::read_to_string(path)?;
    let mut out = String::new();
    splice_includes(&content, base, 0, &mut out);
    Ok(out)
}

fn splice_includes(content: &str, base: &std::path::Path, depth: usize, out: &mut String) {
    for line in content.lines() {
        match include_patterns(line) {
            Some(patterns) if depth < MAX_INCLUDE_DEPTH => {
                for pat in patterns.split_whitespace() {
                    splice_pattern(pat.trim_matches('"'), base, depth, out);
                }
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
}

/// `Include p1 p2 …` → Some("p1 p2 …"); anything else → None.
fn include_patterns(line: &str) -> Option<&str> {
    let (key, value) = line.trim().split_once(char::is_whitespace)?;
    key.eq_ignore_ascii_case("include").then_some(value.trim())
}

fn splice_pattern(pattern: &str, base: &std::path::Path, depth: usize, out: &mut String) {
    let expanded = expand_tilde(pattern);
    let full = if std::path::Path::new(&expanded).is_absolute() {
        expanded
    } else {
        base.join(&expanded).to_string_lossy().into_owned()
    };
    // glob yields matches in alphabetical order; a bad pattern means no files.
    let Ok(paths) = glob::glob(&full) else { return };
    for path in paths.flatten() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            splice_includes(&content, base, depth + 1, out);
        }
    }
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

    // ── Include expansion ──────────────────────────────────────────────

    use std::fs;
    use std::path::Path;

    fn write(dir: &Path, rel: &str, content: &str) -> std::path::PathBuf {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
        p
    }

    fn aliases(entries: &[SshConfigEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.host_alias.as_str()).collect()
    }

    #[test]
    fn include_splices_glob_matches() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "config.d/alpha", "Host alpha\n    HostName 10.0.0.1\n");
        write(dir.path(), "config.d/beta", "Host beta\n    HostName 10.0.0.2\n");
        let root = write(
            dir.path(),
            "config",
            "Host main\n    HostName main.example\n\nInclude config.d/*\n",
        );
        let content = load_with_includes(&root, dir.path()).unwrap();
        let entries = parse(&content);
        assert_eq!(aliases(&entries), ["main", "alpha", "beta"]);
    }

    #[test]
    fn include_nested_two_levels() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "level2", "Host deep\n    HostName deep.example\n");
        write(dir.path(), "level1", "Include level2\nHost mid\n    HostName mid.example\n");
        let root = write(dir.path(), "config", "Include level1\n");
        let content = load_with_includes(&root, dir.path()).unwrap();
        assert_eq!(aliases(&parse(&content)), ["deep", "mid"]);
    }

    #[test]
    fn include_missing_target_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = write(
            dir.path(),
            "config",
            "Include nope/*\nInclude absent-file\nHost real\n    HostName r.example\n",
        );
        let content = load_with_includes(&root, dir.path()).unwrap();
        assert_eq!(aliases(&parse(&content)), ["real"]);
    }

    #[test]
    fn include_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let abs = write(other.path(), "extra", "Host abs\n    HostName abs.example\n");
        let root = write(dir.path(), "config", &format!("Include {}\n", abs.display()));
        let content = load_with_includes(&root, dir.path()).unwrap();
        assert_eq!(aliases(&parse(&content)), ["abs"]);
    }

    #[test]
    fn include_self_reference_terminates() {
        let dir = tempfile::tempdir().unwrap();
        let root = write(dir.path(), "config", "Include config\nHost x\n    HostName x.example\n");
        // Must not loop forever; depth cap bounds the recursion.
        let content = load_with_includes(&root, dir.path()).unwrap();
        assert!(parse(&content).iter().all(|e| e.host_alias == "x"));
    }

    #[test]
    fn include_multiple_patterns_on_one_line() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a", "Host a\n    HostName a.example\n");
        write(dir.path(), "b", "Host b\n    HostName b.example\n");
        let root = write(dir.path(), "config", "Include a b\n");
        let content = load_with_includes(&root, dir.path()).unwrap();
        assert_eq!(aliases(&parse(&content)), ["a", "b"]);
    }

    #[test]
    fn no_include_content_passes_through_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let src = "Host one\n    HostName 1.example\n\n# comment\n";
        let root = write(dir.path(), "config", src);
        assert_eq!(load_with_includes(&root, dir.path()).unwrap(), src);
    }

    #[test]
    fn include_root_missing_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_with_includes(&dir.path().join("config"), dir.path()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }
}
