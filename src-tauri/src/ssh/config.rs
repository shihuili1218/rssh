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
                let alias = value.split_whitespace().next().unwrap_or(&value).to_string();
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

