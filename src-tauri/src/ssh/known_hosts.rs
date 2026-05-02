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
    for (idx, line) in content.lines().enumerate() {
        if drop_lines.contains(&(idx + 1)) {
            removed += 1;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    std::fs::write(path, out)?;
    Ok(removed)
}
