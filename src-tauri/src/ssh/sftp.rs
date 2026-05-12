use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{AppError, AppResult};
use crate::models::Credential;
use crate::ssh::client;

/// 用户取消时返回的 i18n code。前端 transfers.svelte.ts 通过 `errStr.includes(...)`
/// 匹配此字面值识别"用户取消"。改名时前后端必须同步。
pub const CANCELLED_CODE: &str = "transfer_cancelled";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub struct SftpHandle {
    sftp: russh_sftp::client::SftpSession,
    /// 父 SSH 会话 id；None 表示这条 SFTP 走的是独立 TCP（`SftpHandle::connect`）。
    /// SSH 关闭时会按这个字段反向找到所有 children 并清理。
    parent_ssh_id: Option<String>,
}

impl SftpHandle {
    pub fn parent_ssh_id(&self) -> Option<&str> {
        self.parent_ssh_id.as_deref()
    }

    /// Open SFTP subsystem on an existing SSH connection.
    pub async fn from_handle(
        ssh_handle: &crate::ssh::client::SshHandle,
        parent_ssh_id: String,
    ) -> AppResult<Self> {
        let channel = {
            let h = ssh_handle.lock().await;
            h.channel_open_session()
                .await
                .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "open channel", "err": e.to_string() })))?
        };

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "request_subsystem", "err": e.to_string() })))?;

        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| AppError::sftp("sftp_init_failed", json!({ "err": e.to_string() })))?;

        Ok(Self {
            sftp,
            parent_ssh_id: Some(parent_ssh_id),
        })
    }

    pub async fn connect(
        host: String,
        port: u16,
        credential: Credential,
        known_hosts_path: PathBuf,
        timeout_secs: u64,
    ) -> AppResult<Self> {
        let config = crate::ssh::client::default_client_config();
        let log = crate::ssh::client::null_logger();
        let mut handle =
            client::ssh_connect(config, host, port, known_hosts_path, timeout_secs, log, None)
                .await?;

        client::authenticate(&mut handle, credential, None).await?;

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "open channel", "err": e.to_string() })))?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "request_subsystem", "err": e.to_string() })))?;

        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| AppError::sftp("sftp_init_failed", json!({ "err": e.to_string() })))?;

        Ok(Self {
            sftp,
            parent_ssh_id: None,
        })
    }

    pub async fn home_dir(&self) -> AppResult<String> {
        self.sftp
            .canonicalize(".")
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "canonicalize", "err": e.to_string() })))
    }

    pub async fn list_dir(&self, path: &str) -> AppResult<Vec<RemoteEntry>> {
        let entries = self
            .sftp
            .read_dir(path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "read_dir", "err": e.to_string() })))?;

        let mut result: Vec<RemoteEntry> = entries
            .map(|e| {
                let name = e.file_name();
                let is_dir = e.file_type().is_dir();
                let size = e.metadata().size.unwrap_or(0);
                RemoteEntry { name, is_dir, size }
            })
            .collect();

        result.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(result)
    }

    pub async fn download(&self, remote_path: &str) -> AppResult<Vec<u8>> {
        self.sftp
            .read(remote_path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "read", "err": e.to_string() })))
    }

    pub async fn upload(&self, remote_path: &str, data: &[u8]) -> AppResult<()> {
        self.sftp
            .write(remote_path, data)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "write", "err": e.to_string() })))
    }

    pub async fn mkdir(&self, path: &str) -> AppResult<()> {
        self.sftp
            .create_dir(path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "create_dir", "err": e.to_string() })))
    }

    /// Stream-download a remote file to a local path with a hard size cap.
    /// 优先用 metadata.size 早期 bail；metadata.size=None 或服务器撒谎时，
    /// 流式 cap 也会在超限的瞬间中止下载。
    ///
    /// 原子性：下载先写入 `<local_path>.part`，全部成功后 rename 到 local_path。
    /// 任何失败路径（size cap、IO error、metadata 错误）都会清理 .part，
    /// **不会污染 local_path**。无前端进度事件——AI 排障流程用。
    pub async fn download_to_path(
        &self,
        remote_path: &str,
        local_path: &Path,
        max_bytes: u64,
    ) -> AppResult<u64> {
        // 预检：metadata 已知大小且超限就早 bail，省一次 open。size=None
        // 时不能假装它是 0（之前的 unwrap_or(0) 会让任何"未声明大小"的文件
        // 直接绕过 max_bytes）—— 交给下面的 streaming cap 兜底。
        let meta = self
            .sftp
            .metadata(remote_path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "metadata", "err": e.to_string() })))?;
        if let Some(size) = meta.size {
            if size > max_bytes {
                return Err(AppError::sftp(
                    "sftp_file_too_large",
                    json!({ "path": remote_path, "size": size, "limit": max_bytes }),
                ));
            }
        }

        // tmp_path = "<local_path>.part"。用 with_file_name 保留目录前缀。
        let file_name = local_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| AppError::sftp("sftp_io_failed", json!({ "op": "create", "err": "invalid local path" })))?;
        let tmp_path = local_path.with_file_name(format!("{file_name}.part"));

        let result: AppResult<u64> = async {
            let mut remote_file = self
                .sftp
                .open(remote_path)
                .await
                .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "open", "err": e.to_string() })))?;
            let mut local_file = tokio::fs::File::create(&tmp_path).await?;

            let mut transferred: u64 = 0;
            let mut buf = vec![0u8; 32768];
            loop {
                let n = remote_file
                    .read(&mut buf)
                    .await
                    .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "read", "err": e.to_string() })))?;
                if n == 0 {
                    break;
                }
                // 运行时 cap：metadata 缺失 / 撒谎 / 下载途中文件增长都靠这里兜底。
                // 这是 max_bytes 的唯一权威检查点，预检只是优化。
                let next = transferred + n as u64;
                if next > max_bytes {
                    // 主动关闭 server-side handle，免得让远端 fd 等到 session drop 才回收。
                    let _ = remote_file.shutdown().await;
                    return Err(AppError::sftp(
                        "sftp_file_too_large",
                        json!({ "path": remote_path, "size": next, "limit": max_bytes }),
                    ));
                }
                local_file.write_all(&buf[..n]).await?;
                transferred = next;
            }

            remote_file
                .shutdown()
                .await
                .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "close", "err": e.to_string() })))?;
            local_file.shutdown().await?;
            Ok(transferred)
        }
        .await;

        match result {
            Ok(n) => {
                // Windows `MoveFileExW` 默认不覆盖已存在目标 —— ai/session.rs
                // 在同一 session 内反复用同一 local_path，第二次 rename 就会
                // 失败。Unix 上 rename 本来就覆盖，预先 remove 是 no-op。
                let _ = tokio::fs::remove_file(local_path).await;
                tokio::fs::rename(&tmp_path, local_path).await?;
                Ok(n)
            }
            Err(e) => {
                // best-effort cleanup — 即使 unlink 失败，也只是留一个 .part，
                // 不会污染 local_path（消费端约定的产出路径）。
                let _ = tokio::fs::remove_file(&tmp_path).await;
                Err(e)
            }
        }
    }

    /// Stream-download a remote file to a local path, emitting progress events.
    ///
    /// `cancel` 是用户取消的旗子；streaming 循环每个 chunk 之间查一次。
    /// 命中即提前返回 `Sftp(CANCELLED_MSG)`，下游靠这条文本识别"取消"和"出错"。
    pub async fn download_streaming(
        &self,
        remote_path: &str,
        local_path: &Path,
        app: &tauri::AppHandle,
        transfer_id: &str,
        cancel: Arc<AtomicBool>,
    ) -> AppResult<u64> {
        use tauri::Emitter;

        // Get file size for progress
        let meta = self
            .sftp
            .metadata(remote_path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "metadata", "err": e.to_string() })))?;
        let total = meta.size.unwrap_or(0);

        let mut remote_file = self
            .sftp
            .open(remote_path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "open", "err": e.to_string() })))?;

        let mut local_file = tokio::fs::File::create(local_path).await?;

        let mut transferred: u64 = 0;
        let mut buf = vec![0u8; 32768];
        // 事件名每个 chunk emit 一次；预算一次避免循环里反复 String 分配。
        let event = format!("sftp:progress:{transfer_id}");

        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AppError::sftp(CANCELLED_CODE, json!({})));
            }
            let n = remote_file
                .read(&mut buf)
                .await
                .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "read", "err": e.to_string() })))?;
            if n == 0 {
                break;
            }

            local_file.write_all(&buf[..n]).await?;
            transferred += n as u64;

            let _ = app.emit(
                &event,
                serde_json::json!({ "transferred": transferred, "total": total }),
            );
        }

        remote_file
            .shutdown()
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "close", "err": e.to_string() })))?;

        Ok(transferred)
    }

    /// Stream-upload a local file to a remote path, emitting progress events.
    pub async fn upload_streaming(
        &self,
        local_path: &Path,
        remote_path: &str,
        app: &tauri::AppHandle,
        transfer_id: &str,
        cancel: Arc<AtomicBool>,
    ) -> AppResult<u64> {
        use tauri::Emitter;

        let local_meta = tokio::fs::metadata(local_path).await?;
        let total = local_meta.len();

        let mut local_file = tokio::fs::File::open(local_path).await?;

        let mut remote_file = self
            .sftp
            .create(remote_path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "create", "err": e.to_string() })))?;

        let mut transferred: u64 = 0;
        let mut buf = vec![0u8; 32768];
        // 事件名每个 chunk emit 一次；预算一次避免循环里反复 String 分配。
        let event = format!("sftp:progress:{transfer_id}");

        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AppError::sftp(CANCELLED_CODE, json!({})));
            }
            let n = local_file.read(&mut buf).await?;
            if n == 0 {
                break;
            }

            remote_file
                .write_all(&buf[..n])
                .await
                .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "write", "err": e.to_string() })))?;
            transferred += n as u64;

            let _ = app.emit(
                &event,
                serde_json::json!({ "transferred": transferred, "total": total }),
            );
        }

        remote_file
            .shutdown()
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "close", "err": e.to_string() })))?;

        Ok(transferred)
    }
}
