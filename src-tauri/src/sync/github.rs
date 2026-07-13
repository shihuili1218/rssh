use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::sync::metadata::SyncMetadata;
use crate::sync::remote::RemoteBackup;

const API_BASE: &str = "https://api.github.com";
const BACKUP_FILE: &str = "rssh_backup.json";
const METADATA_FILE: &str = "rssh_backup.meta.json";

/// GitHub 配置同步。
pub struct GitHubSync {
    pub token: String,
    pub owner: String,
    pub repo: String,
    pub branch: String,
    client: reqwest::Client,
    api_base: String,
}

#[derive(Deserialize)]
struct FileResponse {
    sha: Option<String>,
    content: Option<String>,
}

impl GitHubSync {
    pub fn from_settings(token: &str, repo_slug: &str, branch: &str) -> AppResult<Self> {
        Self::with_client(token, repo_slug, branch, API_BASE, reqwest::Client::new())
    }

    fn with_client(
        token: &str,
        repo_slug: &str,
        branch: &str,
        api_base: &str,
        client: reqwest::Client,
    ) -> AppResult<Self> {
        let parts: Vec<&str> = repo_slug.split('/').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err(AppError::config("github_repo_format", json!({})));
        }
        Ok(Self {
            token: token.to_string(),
            owner: parts[0].to_string(),
            repo: parts[1].to_string(),
            branch: branch.to_string(),
            client,
            api_base: api_base.trim_end_matches('/').to_string(),
        })
    }

    fn contents_url(&self, file: &str) -> String {
        format!(
            "{}/repos/{}/{}/contents/{file}",
            self.api_base, self.owner, self.repo
        )
    }

    async fn push_file(&self, file: &str, content: &str, message: &str) -> AppResult<()> {
        let url = self.contents_url(file);
        let sha = match self
            .client
            .get(format!("{url}?ref={}", self.branch))
            .headers(self.headers()?)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                resp.json::<FileResponse>().await.ok().and_then(|f| f.sha)
            }
            _ => None,
        };

        let mut body = serde_json::json!({
            "message": format!("{message} {}", chrono::Utc::now().to_rfc3339()),
            "content": STANDARD.encode(content.as_bytes()),
            "branch": self.branch,
        });
        if let Some(sha) = sha {
            body["sha"] = serde_json::Value::String(sha);
        }

        let resp = self
            .client
            .put(&url)
            .headers(self.headers()?)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::other("github_push_failed", json!({ "err": e.to_string() })))?;

        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(AppError::other("github_api_error", json!({ "msg": msg })));
        }
        Ok(())
    }

    /// 推送配置 JSON 到 GitHub。
    pub async fn push(&self, json_content: &str) -> AppResult<()> {
        self.push_file(BACKUP_FILE, json_content, "Update RSSH config")
            .await
    }

    /// 从 GitHub 拉取配置 JSON。
    pub async fn pull(&self) -> AppResult<String> {
        let url = format!("{}?ref={}", self.contents_url(BACKUP_FILE), self.branch);

        let resp = self
            .client
            .get(&url)
            .headers(self.headers()?)
            .send()
            .await
            .map_err(|e| AppError::other("github_pull_failed", json!({ "err": e.to_string() })))?;

        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(AppError::other("github_api_error", json!({ "msg": msg })));
        }

        let file: FileResponse = resp
            .json()
            .await
            .map_err(|e| AppError::other("github_parse_failed", json!({ "err": e.to_string() })))?;

        let raw = file
            .content
            .ok_or_else(|| AppError::other("github_empty_content", json!({})))?
            .replace('\n', "");

        let bytes = STANDARD.decode(&raw).map_err(|e| {
            AppError::config(
                "crypto_base64_decode_failed",
                json!({ "err": e.to_string() }),
            )
        })?;

        String::from_utf8(bytes)
            .map_err(|e| AppError::other("github_utf8_failed", json!({ "err": e.to_string() })))
    }

    /// 推送明文同步元数据到独立 JSON 文件，不改变旧版加密备份文件。
    pub async fn push_metadata(&self, metadata: &SyncMetadata) -> AppResult<()> {
        let content = metadata.to_json()?;
        self.push_file(METADATA_FILE, &content, "Update RSSH config metadata")
            .await
    }

    /// 拉取明文同步元数据。旧版远端没有该文件时返回 `None`；其他错误仍向上返回。
    pub async fn pull_metadata(&self) -> AppResult<Option<SyncMetadata>> {
        let url = format!("{}?ref={}", self.contents_url(METADATA_FILE), self.branch);
        let resp = self
            .client
            .get(&url)
            .headers(self.headers()?)
            .send()
            .await
            .map_err(|e| AppError::other("github_pull_failed", json!({ "err": e.to_string() })))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(AppError::other("github_api_error", json!({ "msg": msg })));
        }

        let file: FileResponse = resp
            .json()
            .await
            .map_err(|e| AppError::other("github_parse_failed", json!({ "err": e.to_string() })))?;
        let raw = file
            .content
            .ok_or_else(|| AppError::other("github_empty_content", json!({})))?
            .replace('\n', "");
        let bytes = STANDARD.decode(&raw).map_err(|e| {
            AppError::config(
                "crypto_base64_decode_failed",
                json!({ "err": e.to_string() }),
            )
        })?;
        let metadata_json = String::from_utf8(bytes)
            .map_err(|e| AppError::other("github_utf8_failed", json!({ "err": e.to_string() })))?;
        SyncMetadata::from_json(&metadata_json).map(Some)
    }

    fn headers(&self) -> AppResult<reqwest::header::HeaderMap> {
        use reqwest::header::HeaderValue;
        // token 来自用户输入，含 CR/LF/non-ASCII 时 HeaderValue::from_str 会失败。
        // 之前的 .parse().unwrap() 会 panic — 这里转成可恢复错误。
        let bearer = HeaderValue::from_str(&format!("Bearer {}", self.token)).map_err(|e| {
            AppError::config("github_token_invalid", json!({ "err": e.to_string() }))
        })?;
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("Authorization", bearer);
        // 这三条全是 ASCII 字面量，from_static 编译期保证合法 — 真没必要 unwrap。
        h.insert(
            "Accept",
            HeaderValue::from_static("application/vnd.github+json"),
        );
        h.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );
        h.insert("User-Agent", HeaderValue::from_static("RSSH"));
        Ok(h)
    }
}

#[async_trait::async_trait]
impl RemoteBackup for GitHubSync {
    async fn read_payload(&self) -> AppResult<String> {
        self.pull().await
    }

    async fn read_metadata(&self) -> AppResult<Option<SyncMetadata>> {
        self.pull_metadata().await
    }

    async fn write_payload(&self, content: &str) -> AppResult<()> {
        self.push(content).await
    }

    async fn write_metadata(&self, metadata: &SyncMetadata) -> AppResult<()> {
        self.push_metadata(metadata).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::metadata::SyncMetadata;
    use mockito::Matcher;

    #[tokio::test]
    async fn pull_metadata_returns_decoded_metadata() {
        let mut server = mockito::Server::new_async().await;
        let metadata = SyncMetadata {
            version: 6,
            config_digest: format!("sha256:{}", "a".repeat(64)),
        };
        let encoded = STANDARD.encode(metadata.to_json().unwrap());
        let _mock = server
            .mock("GET", "/repos/acme/config/contents/rssh_backup.meta.json")
            .match_query(Matcher::UrlEncoded("ref".into(), "main".into()))
            .match_header("authorization", "Bearer token")
            .with_status(200)
            .with_body(serde_json::json!({ "sha": "abc", "content": encoded }).to_string())
            .create_async()
            .await;
        let sync = GitHubSync::with_client(
            "token",
            "acme/config",
            "main",
            &server.url(),
            reqwest::Client::new(),
        )
        .unwrap();

        assert_eq!(sync.pull_metadata().await.unwrap(), Some(metadata));
    }

    #[tokio::test]
    async fn pull_metadata_returns_none_only_for_missing_file() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/repos/acme/config/contents/rssh_backup.meta.json")
            .match_query(Matcher::UrlEncoded("ref".into(), "main".into()))
            .with_status(404)
            .create_async()
            .await;
        let sync = GitHubSync::with_client(
            "token",
            "acme/config",
            "main",
            &server.url(),
            reqwest::Client::new(),
        )
        .unwrap();

        assert_eq!(sync.pull_metadata().await.unwrap(), None);
    }

    #[tokio::test]
    async fn pull_metadata_does_not_hide_authentication_errors() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/repos/acme/config/contents/rssh_backup.meta.json")
            .match_query(Matcher::UrlEncoded("ref".into(), "main".into()))
            .with_status(401)
            .with_body("bad credentials")
            .create_async()
            .await;
        let sync = GitHubSync::with_client(
            "token",
            "acme/config",
            "main",
            &server.url(),
            reqwest::Client::new(),
        )
        .unwrap();

        let err = sync.pull_metadata().await.unwrap_err();
        assert_eq!(err.code(), "github_api_error");
    }

    #[tokio::test]
    async fn pull_metadata_rejects_invalid_metadata_json() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/repos/acme/config/contents/rssh_backup.meta.json")
            .match_query(Matcher::UrlEncoded("ref".into(), "main".into()))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "sha": "abc",
                    "content": STANDARD.encode("not-json"),
                })
                .to_string(),
            )
            .create_async()
            .await;
        let sync = GitHubSync::with_client(
            "token",
            "acme/config",
            "main",
            &server.url(),
            reqwest::Client::new(),
        )
        .unwrap();

        let err = sync.pull_metadata().await.unwrap_err();
        assert_eq!(err.code(), "sync_metadata_invalid");
    }

    #[tokio::test]
    async fn pull_metadata_does_not_hide_network_errors() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let unavailable = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let sync = GitHubSync::with_client(
            "token",
            "acme/config",
            "main",
            &unavailable,
            reqwest::Client::new(),
        )
        .unwrap();

        let err = sync.pull_metadata().await.unwrap_err();
        assert_eq!(err.code(), "github_pull_failed");
    }

    #[tokio::test]
    async fn pull_still_reads_legacy_backup_file() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/repos/acme/config/contents/rssh_backup.json")
            .match_query(Matcher::UrlEncoded("ref".into(), "main".into()))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "sha": "abc",
                    "content": STANDARD.encode("encrypted-backup"),
                })
                .to_string(),
            )
            .create_async()
            .await;
        let sync = GitHubSync::with_client(
            "token",
            "acme/config",
            "main",
            &server.url(),
            reqwest::Client::new(),
        )
        .unwrap();

        assert_eq!(sync.pull().await.unwrap(), "encrypted-backup");
    }

    #[tokio::test]
    async fn push_still_writes_legacy_backup_file() {
        let mut server = mockito::Server::new_async().await;
        let _get = server
            .mock("GET", "/repos/acme/config/contents/rssh_backup.json")
            .match_query(Matcher::UrlEncoded("ref".into(), "main".into()))
            .with_status(404)
            .create_async()
            .await;
        let _put = server
            .mock("PUT", "/repos/acme/config/contents/rssh_backup.json")
            .match_body(Matcher::PartialJson(serde_json::json!({
                "content": STANDARD.encode("encrypted-backup"),
                "branch": "main",
            })))
            .with_status(201)
            .create_async()
            .await;
        let sync = GitHubSync::with_client(
            "token",
            "acme/config",
            "main",
            &server.url(),
            reqwest::Client::new(),
        )
        .unwrap();

        sync.push("encrypted-backup").await.unwrap();
    }

    #[tokio::test]
    async fn push_metadata_writes_plain_json_file() {
        let mut server = mockito::Server::new_async().await;
        let metadata = SyncMetadata {
            version: 6,
            config_digest: format!("sha256:{}", "b".repeat(64)),
        };
        let encoded = STANDARD.encode(metadata.to_json().unwrap());
        let _get = server
            .mock("GET", "/repos/acme/config/contents/rssh_backup.meta.json")
            .match_query(Matcher::UrlEncoded("ref".into(), "main".into()))
            .with_status(404)
            .create_async()
            .await;
        let _put = server
            .mock("PUT", "/repos/acme/config/contents/rssh_backup.meta.json")
            .match_header("authorization", "Bearer token")
            .match_body(Matcher::PartialJson(serde_json::json!({
                "content": encoded,
                "branch": "main",
            })))
            .with_status(201)
            .create_async()
            .await;
        let sync = GitHubSync::with_client(
            "token",
            "acme/config",
            "main",
            &server.url(),
            reqwest::Client::new(),
        )
        .unwrap();

        sync.push_metadata(&metadata).await.unwrap();
    }

    #[tokio::test]
    async fn push_metadata_updates_existing_file_with_sha() {
        let mut server = mockito::Server::new_async().await;
        let metadata = SyncMetadata {
            version: 7,
            config_digest: format!("sha256:{}", "c".repeat(64)),
        };
        let _get = server
            .mock("GET", "/repos/acme/config/contents/rssh_backup.meta.json")
            .match_query(Matcher::UrlEncoded("ref".into(), "main".into()))
            .with_status(200)
            .with_body(serde_json::json!({ "sha": "existing-sha" }).to_string())
            .create_async()
            .await;
        let _put = server
            .mock("PUT", "/repos/acme/config/contents/rssh_backup.meta.json")
            .match_body(Matcher::PartialJson(serde_json::json!({
                "sha": "existing-sha",
                "branch": "main",
            })))
            .with_status(200)
            .create_async()
            .await;
        let sync = GitHubSync::with_client(
            "token",
            "acme/config",
            "main",
            &server.url(),
            reqwest::Client::new(),
        )
        .unwrap();

        sync.push_metadata(&metadata).await.unwrap();
    }
}
