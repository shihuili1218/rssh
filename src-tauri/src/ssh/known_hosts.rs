//! known_hosts 路径策略：复用系统标准位置 `~/.ssh/known_hosts`，
//! 让用户在 OpenSSH / rssh / 其他 SSH 客户端之间共享同一份信任链。
//!
//! Android 没有 home，退到 app_data_dir/.ssh/known_hosts。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 解析 known_hosts 文件路径。`fallback_dir` 仅 Android 用。
pub fn path_for(fallback_dir: &Path) -> PathBuf {
    #[cfg(target_os = "android")]
    {
        return fallback_dir.join(".ssh").join("known_hosts");
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = fallback_dir;
        if let Some(home) = dirs::home_dir() {
            home.join(".ssh").join("known_hosts")
        } else {
            // 不正常的环境（CI、容器无 HOME）兜底
            fallback_dir.join("known_hosts")
        }
    }
}

/// 删除 `host:port` 在 known_hosts 中的所有匹配条目，返回删除条数。
/// 用于 host key 变更后用户在终端中确认 'replace' 的场景——移动端没有
/// 命令行 `ssh-keygen -R`，所以由我们就地实现。
///
/// 复用 russh 的 `known_host_keys_path` 定位匹配行（含 hashed `|1|...` 条目），
/// 只重写文件，不解析 host 模式——把模式匹配 deleg 给 russh 是好品味。
pub fn remove_host(host: &str, port: u16, path: &Path) -> std::io::Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let matches = russh::keys::known_hosts::known_host_keys_path(host, port, path)
        .map_err(|e| std::io::Error::other(format!("known_host_keys_path: {e}")))?;
    if matches.is_empty() {
        return Ok(0);
    }
    let drop_lines: HashSet<usize> = matches.iter().map(|(n, _)| *n).collect();
    let content = std::fs::read_to_string(path)?;
    let mut out = String::new();
    let mut removed = 0;
    // russh 的行号是"非注释行"序号：注释行 skip 不递增，其他（含空行）递增。
    // 我们用并行计数器 mirror 这套规则，否则注释/空行会让物理 idx 与 russh 行号错位，
    // 导致误删邻近条目。判断"是不是注释"用首字节比对，与 russh 一致——
    // 不要 trim：以空格开头再跟 `#`，russh 视作有效行，我们也要跟着。
    let mut russh_line: usize = 1;
    for line in content.lines() {
        let is_comment = line.as_bytes().first() == Some(&b'#');
        if is_comment {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if drop_lines.contains(&russh_line) {
            removed += 1;
        } else {
            out.push_str(line);
            out.push('\n');
        }
        russh_line += 1;
    }
    std::fs::write(path, out)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个合法的 ed25519 公钥（base64 已校验，固定字面量供测试用）。
    /// 不是任何真实主机的密钥，只是格式合法的样本。
    const SAMPLE_ED25519: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILZqOrxKsAQNKvmw0XX0pQrDrlBJpZj9PLfZN1RNxVFL";

    fn write_known_hosts(path: &Path, lines: &[&str]) {
        let body = lines.join("\n") + "\n";
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn remove_host_returns_zero_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope");
        assert_eq!(remove_host("anything", 22, &p).unwrap(), 0);
    }

    #[test]
    fn remove_host_returns_zero_when_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("known_hosts");
        write_known_hosts(
            &p,
            &[&format!("alpha.example {SAMPLE_ED25519}"), &format!("beta.example {SAMPLE_ED25519}")],
        );
        assert_eq!(remove_host("gamma.example", 22, &p).unwrap(), 0);
        // 文件未被修改：两行都还在
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("alpha.example"));
        assert!(after.contains("beta.example"));
    }

    #[test]
    fn remove_host_drops_matching_line_keeps_others() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("known_hosts");
        write_known_hosts(
            &p,
            &[
                &format!("target.example {SAMPLE_ED25519}"),
                &format!("other.example {SAMPLE_ED25519}"),
            ],
        );
        let removed = remove_host("target.example", 22, &p).unwrap();
        assert_eq!(removed, 1);
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(!after.contains("target.example"));
        assert!(after.contains("other.example"));
    }

    #[test]
    fn remove_host_drops_all_matching_lines() {
        // 同一个 host 写多行（轮换 key 时容易出现）— 应当全删
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("known_hosts");
        write_known_hosts(
            &p,
            &[
                &format!("dup.example {SAMPLE_ED25519}"),
                &format!("keep.example {SAMPLE_ED25519}"),
                &format!("dup.example {SAMPLE_ED25519}"),
            ],
        );
        let removed = remove_host("dup.example", 22, &p).unwrap();
        assert_eq!(removed, 2);
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(!after.contains("dup.example"));
        assert!(after.contains("keep.example"));
    }

    /// 回归 net：known_hosts 含空行/注释时不能误删邻近条目。
    /// 早期实现把物理行号当成 russh 行号，alpha 会被误删。
    #[test]
    fn remove_host_preserves_unrelated_garbage_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("known_hosts");
        let body = format!(
            "# comment line\n\nalpha.example {SAMPLE_ED25519}\ntarget.example {SAMPLE_ED25519}\n# tail\n"
        );
        std::fs::write(&p, body).unwrap();
        let removed = remove_host("target.example", 22, &p).unwrap();
        assert_eq!(removed, 1);
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("# comment line"));
        assert!(after.contains("alpha.example"));
        assert!(after.contains("# tail"));
        assert!(!after.contains("target.example"));
    }

    #[test]
    fn path_for_returns_some_path_under_home_or_fallback() {
        // path_for 在 cfg(not(android)) 下走 home_dir() 或 fallback_dir。
        // 不验证具体路径（CI 环境 HOME 不一定存在），只保证返回值以
        // `known_hosts` 结尾且非空。
        let dir = tempfile::tempdir().unwrap();
        let p = path_for(dir.path());
        assert!(p.ends_with("known_hosts"), "got {}", p.display());
    }
}
