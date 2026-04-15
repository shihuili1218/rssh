//! known_hosts 路径策略：复用系统标准位置 `~/.ssh/known_hosts`，
//! 让用户在 OpenSSH / rssh / 其他 SSH 客户端之间共享同一份信任链。
//!
//! Android 没有 home，退到 app_data_dir/.ssh/known_hosts。

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
