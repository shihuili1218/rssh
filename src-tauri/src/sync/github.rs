use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;

use crate::error::{AppError, AppResult};

const API_BASE: &str = "https://api.github.com";
const BACKUP_FILE: &str = "rssh_backup.json";

/// GitHub 配置同步。
pub struct GitHubSync {
    pub token: String,
    pub owner: String,
    pub repo: String,
    pub branch: String,
}

#[derive(Deserialize)]
struct FileResponse {
    sha: Option<String>,
    content: Option<String>,
}

impl GitHubSync {
    pub fn from_settings(token: &str, repo_slug: &str, branch: &str) -> AppResult<Self> {
        let parts: Vec<&str> = repo_slug.split('/').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err(AppError::Config("Repo 格式应为 owner/repo".into()));
        }
        Ok(Self {
            token: token.to_string(),
            owner: parts[0].to_string(),
            repo: parts[1].to_string(),
            branch: branch.to_string(),
        })
    }

    /// 推送配置 JSON 到 GitHub。
    pub async fn push(&self, json_content: &str) -> AppResult<()> {
        let client = reqwest::Client::new();
        let url = format!(
            "{API_BASE}/repos/{}/{}/contents/{BACKUP_FILE}",
            self.owner, self.repo
        );

        // 获取现有文件 SHA（更新需要）
        let sha = match client
            .get(format!("{url}?ref={}", self.branch))
            .headers(self.headers())
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                resp.json::<FileResponse>().await.ok().and_then(|f| f.sha)
            }
            _ => None,
        };

        let encoded = STANDARD.encode(json_content.as_bytes());
        let mut body = serde_json::json!({
            "message": format!("Update RSSH config {}", chrono::Utc::now().to_rfc3339()),
            "content": encoded,
            "branch": self.branch,
        });
        if let Some(s) = sha {
            body["sha"] = serde_json::Value::String(s);
        }

        let resp = client
            .put(&url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Other(format!("GitHub push 失败: {e}")))?;

        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(AppError::Other(format!("GitHub {msg}")));
        }
        Ok(())
    }

    /// 从 GitHub 拉取配置 JSON。
    pub async fn pull(&self) -> AppResult<String> {
        let client = reqwest::Client::new();
        let url = format!(
            "{API_BASE}/repos/{}/{}/contents/{BACKUP_FILE}?ref={}",
            self.owner, self.repo, self.branch
        );

        let resp = client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| AppError::Other(format!("GitHub pull 失败: {e}")))?;

        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(AppError::Other(format!("GitHub {msg}")));
        }

        let file: FileResponse = resp
            .json()
            .await
            .map_err(|e| AppError::Other(format!("解析失败: {e}")))?;

        let raw = file
            .content
            .ok_or_else(|| AppError::Other("GitHub 返回无内容".into()))?
            .replace('\n', "");

        let bytes = STANDARD
            .decode(&raw)
            .map_err(|e| AppError::Other(format!("Base64 解码失败: {e}")))?;

        String::from_utf8(bytes).map_err(|e| AppError::Other(format!("UTF-8 错误: {e}")))
    }

    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            "Authorization",
            format!("Bearer {}", self.token).parse().unwrap(),
        );
        h.insert("Accept", "application/vnd.github+json".parse().unwrap());
        h.insert("X-GitHub-Api-Version", "2022-11-28".parse().unwrap());
        h.insert("User-Agent", "RSSH".parse().unwrap());
        h
    }
}
