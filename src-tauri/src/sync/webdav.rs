use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde_json::json;
use url::Url;

use crate::error::{AppError, AppResult};

const BACKUP_FILE: &str = "rssh_backup.enc";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ERROR_BODY_LEN: usize = 2048;

/// WebDAV 配置同步。
pub struct WebDavSync {
    pub url: String,
    pub username: String,
    pub password: String,
    client: Client,
}

impl std::fmt::Debug for WebDavSync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDavSync")
            .field("url", &self.url)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl WebDavSync {
    pub fn from_settings(url: &str, username: &str, password: &str) -> AppResult<Self> {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(format!("rssh/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| {
                AppError::other(
                    "webdav_client_build_failed",
                    json!({ "err": e.to_string() }),
                )
            })?;
        Self::with_client(url, username, password, client)
    }

    pub fn with_client(
        url: &str,
        username: &str,
        password: &str,
        client: Client,
    ) -> AppResult<Self> {
        if url.is_empty() {
            return Err(AppError::config("webdav_url_missing", json!({})));
        }
        let parsed = Url::parse(url)
            .map_err(|e| AppError::config("webdav_url_invalid", json!({ "err": e.to_string() })))?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(AppError::config(
                "webdav_url_invalid",
                json!({ "err": "URL scheme must be http or https" }),
            ));
        }
        if parsed.host_str().map_or(true, |h| h.is_empty()) {
            return Err(AppError::config(
                "webdav_url_invalid",
                json!({ "err": "URL must have a valid host" }),
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(AppError::config("webdav_url_userinfo_forbidden", json!({})));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(AppError::config(
                "webdav_url_query_fragment_forbidden",
                json!({}),
            ));
        }

        let mut base = parsed;
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }

        Ok(Self {
            url: base.to_string(),
            username: username.to_string(),
            password: password.to_string(),
            client,
        })
    }

    fn build_file_url(&self) -> AppResult<Url> {
        Url::parse(&self.url)
            .map_err(|e| AppError::other("webdav_url_invalid", json!({ "err": e.to_string() })))?
            .join(BACKUP_FILE)
            .map_err(|e| AppError::other("webdav_url_invalid", json!({ "err": e.to_string() })))
    }

    /// 推送加密配置到 WebDAV。
    pub async fn push(&self, content: &str) -> AppResult<()> {
        let url = self.build_file_url()?;

        let resp = self
            .client
            .put(url)
            .basic_auth(&self.username, Some(&self.password))
            .body(content.to_string())
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::other("webdav_timeout", json!({}))
                } else {
                    AppError::other("webdav_push_failed", json!({ "err": e.to_string() }))
                }
            })?;

        if resp.status().is_success() {
            return Ok(());
        }

        let status = resp.status().as_u16();
        let msg = Self::truncate_error_body(resp).await;
        if status == StatusCode::UNAUTHORIZED.as_u16() || status == StatusCode::FORBIDDEN.as_u16() {
            return Err(AppError::other(
                "webdav_auth_failed",
                json!({ "status": status }),
            ));
        }

        Err(AppError::other(
            "webdav_api_error",
            json!({ "status": status, "msg": msg }),
        ))
    }

    /// 从 WebDAV 拉取加密配置。
    pub async fn pull(&self) -> AppResult<String> {
        let url = self.build_file_url()?;

        let resp = self
            .client
            .get(url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::other("webdav_timeout", json!({}))
                } else {
                    AppError::other("webdav_pull_failed", json!({ "err": e.to_string() }))
                }
            })?;

        if resp.status() == StatusCode::NOT_FOUND {
            return Err(AppError::other("webdav_not_found", json!({})));
        }

        if resp.status().is_success() {
            return resp.text().await.map_err(|e| {
                AppError::other("webdav_pull_failed", json!({ "err": e.to_string() }))
            });
        }

        let status = resp.status().as_u16();
        let msg = Self::truncate_error_body(resp).await;
        if status == StatusCode::UNAUTHORIZED.as_u16() || status == StatusCode::FORBIDDEN.as_u16() {
            return Err(AppError::other(
                "webdav_auth_failed",
                json!({ "status": status }),
            ));
        }

        Err(AppError::other(
            "webdav_api_error",
            json!({ "status": status, "msg": msg }),
        ))
    }

    /// 用 `resp.chunk()` 逐块读取错误响应体，累计到 MAX_ERROR_BODY_LEN 即停止，
    /// 避免一次性缓冲超大响应。
    async fn truncate_error_body(mut resp: reqwest::Response) -> String {
        let mut buf = Vec::with_capacity(MAX_ERROR_BODY_LEN);
        while let Ok(Some(chunk)) = resp.chunk().await {
            let remaining = MAX_ERROR_BODY_LEN.saturating_sub(buf.len());
            if remaining == 0 {
                break;
            }
            let take = chunk.len().min(remaining);
            buf.extend_from_slice(&chunk[..take]);
        }
        String::from_utf8_lossy(&buf).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_settings_accepts_valid_https_url() {
        let s = WebDavSync::from_settings("https://dav.example.com/rssh/", "u", "p").unwrap();
        assert_eq!(s.url, "https://dav.example.com/rssh/");
        assert_eq!(s.username, "u");
        assert_eq!(s.password, "p");
    }

    #[test]
    fn from_settings_adds_trailing_slash() {
        let s = WebDavSync::from_settings("https://dav.example.com/rssh", "u", "p").unwrap();
        assert_eq!(s.url, "https://dav.example.com/rssh/");
    }

    #[test]
    fn from_settings_accepts_http_url() {
        let s = WebDavSync::from_settings("http://192.168.1.2/webdav/", "u", "p").unwrap();
        assert_eq!(s.url, "http://192.168.1.2/webdav/");
    }

    #[test]
    fn debug_does_not_expose_password() {
        let s = WebDavSync::from_settings("https://dav.example.com/", "u", "secret").unwrap();
        let out = format!("{s:?}");
        assert!(!out.contains("secret"), "password must not appear in Debug");
    }

    #[test]
    fn from_settings_rejects_empty_url() {
        let err = WebDavSync::from_settings("", "u", "p").unwrap_err();
        assert_eq!(err.code(), "webdav_url_missing");
    }

    #[test]
    fn from_settings_rejects_invalid_scheme() {
        let err = WebDavSync::from_settings("ftp://dav.example.com/", "u", "p").unwrap_err();
        assert_eq!(err.code(), "webdav_url_invalid");
    }

    #[test]
    fn from_settings_rejects_missing_host() {
        let err = WebDavSync::from_settings("http://", "u", "p").unwrap_err();
        assert_eq!(err.code(), "webdav_url_invalid");
    }

    #[test]
    fn from_settings_rejects_userinfo() {
        let err = WebDavSync::from_settings("https://u:p@dav.example.com/", "u", "p").unwrap_err();
        assert_eq!(err.code(), "webdav_url_userinfo_forbidden");
    }

    #[test]
    fn from_settings_rejects_query_and_fragment() {
        let err = WebDavSync::from_settings("https://dav.example.com/?x=1", "u", "p").unwrap_err();
        assert_eq!(err.code(), "webdav_url_query_fragment_forbidden");

        let err = WebDavSync::from_settings("https://dav.example.com/#frag", "u", "p").unwrap_err();
        assert_eq!(err.code(), "webdav_url_query_fragment_forbidden");
    }

    #[tokio::test]
    async fn push_succeeds_on_201() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("PUT", "/rssh_backup.enc")
            .with_status(201)
            .create_async()
            .await;
        let sync = WebDavSync::from_settings(&server.url(), "u", "p").unwrap();
        sync.push("payload").await.unwrap();
    }

    #[tokio::test]
    async fn push_uses_correct_nested_path() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("PUT", "/rssh/rssh_backup.enc")
            .with_status(201)
            .match_header("authorization", "Basic dTpw")
            .create_async()
            .await;
        let base = format!("{}/rssh", server.url());
        let sync = WebDavSync::from_settings(&base, "u", "p").unwrap();
        sync.push("payload").await.unwrap();
    }

    #[tokio::test]
    async fn pull_returns_body_on_200() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/rssh_backup.enc")
            .with_status(200)
            .with_body("encrypted-data")
            .create_async()
            .await;
        let sync = WebDavSync::from_settings(&server.url(), "u", "p").unwrap();
        assert_eq!(sync.pull().await.unwrap(), "encrypted-data");
    }

    #[tokio::test]
    async fn pull_not_found_on_404() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/rssh_backup.enc")
            .with_status(404)
            .create_async()
            .await;
        let sync = WebDavSync::from_settings(&server.url(), "u", "p").unwrap();
        let err = sync.pull().await.unwrap_err();
        assert_eq!(err.code(), "webdav_not_found");
    }

    #[tokio::test]
    async fn push_401_returns_auth_failed() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("PUT", "/rssh_backup.enc")
            .with_status(401)
            .create_async()
            .await;
        let sync = WebDavSync::from_settings(&server.url(), "u", "p").unwrap();
        let err = sync.push("payload").await.unwrap_err();
        assert_eq!(err.code(), "webdav_auth_failed");
    }

    #[tokio::test]
    async fn push_403_returns_auth_failed() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("PUT", "/rssh_backup.enc")
            .with_status(403)
            .create_async()
            .await;
        let sync = WebDavSync::from_settings(&server.url(), "u", "p").unwrap();
        let err = sync.push("payload").await.unwrap_err();
        assert_eq!(err.code(), "webdav_auth_failed");
    }
}
