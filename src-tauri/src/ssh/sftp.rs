use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{AppError, AppResult};
use crate::models::Credential;
use crate::ssh::client;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub struct SftpHandle {
    sftp: russh_sftp::client::SftpSession,
}

impl SftpHandle {
    /// Open SFTP subsystem on an existing SSH connection.
    pub async fn from_handle(ssh_handle: &crate::ssh::client::SshHandle) -> AppResult<Self> {
        let channel = {
            let h = ssh_handle.lock().await;
            h.channel_open_session()
                .await
                .map_err(|e| AppError::Sftp(format!("open channel: {e}")))?
        };

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| AppError::Sftp(format!("{e}")))?;

        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| AppError::Sftp(format!("SFTP 初始化失败: {e}")))?;

        Ok(Self { sftp })
    }

    pub async fn connect(
        host: &str,
        port: u16,
        credential: &Credential,
        known_hosts_path: PathBuf,
        timeout_secs: u64,
    ) -> AppResult<Self> {
        let config = crate::ssh::client::default_client_config();
        let mut handle = client::ssh_connect(config, host, port, known_hosts_path, timeout_secs).await
            .map_err(|e| AppError::Sftp(format!("SSH 连接失败: {e}")))?;

        client::authenticate(&mut handle, credential).await
            .map_err(|e| AppError::Sftp(format!("认证失败: {e}")))?;

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| AppError::Sftp(format!("{e}")))?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| AppError::Sftp(format!("{e}")))?;

        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| AppError::Sftp(format!("SFTP 初始化失败: {e}")))?;

        Ok(Self { sftp })
    }

    pub async fn home_dir(&self) -> AppResult<String> {
        self.sftp
            .canonicalize(".")
            .await
            .map_err(|e| AppError::Sftp(format!("{e}")))
    }

    pub async fn list_dir(&self, path: &str) -> AppResult<Vec<RemoteEntry>> {
        let entries = self
            .sftp
            .read_dir(path)
            .await
            .map_err(|e| AppError::Sftp(format!("{e}")))?;

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
            .map_err(|e| AppError::Sftp(format!("{e}")))
    }

    pub async fn upload(&self, remote_path: &str, data: &[u8]) -> AppResult<()> {
        self.sftp
            .write(remote_path, data)
            .await
            .map_err(|e| AppError::Sftp(format!("{e}")))
    }

    pub async fn mkdir(&self, path: &str) -> AppResult<()> {
        self.sftp
            .create_dir(path)
            .await
            .map_err(|e| AppError::Sftp(format!("{e}")))
    }

    /// Stream-download a remote file to a local path with a hard size cap.
    /// 超过 max_bytes 直接 bail（不开始下载）。无前端进度事件——AI 排障流程
    /// 用，前端不需要进度条。
    pub async fn download_to_path(
        &self,
        remote_path: &str,
        local_path: &Path,
        max_bytes: u64,
    ) -> AppResult<u64> {
        let meta = self
            .sftp
            .metadata(remote_path)
            .await
            .map_err(|e| AppError::Sftp(format!("metadata: {e}")))?;
        let total = meta.size.unwrap_or(0);
        if total > max_bytes {
            return Err(AppError::Sftp(format!(
                "文件 {} 大小 {} 字节超过限制 {} 字节",
                remote_path, total, max_bytes
            )));
        }

        let mut remote_file = self
            .sftp
            .open(remote_path)
            .await
            .map_err(|e| AppError::Sftp(format!("open: {e}")))?;
        let mut local_file = tokio::fs::File::create(local_path).await?;

        let mut transferred: u64 = 0;
        let mut buf = vec![0u8; 32768];
        loop {
            let n = remote_file
                .read(&mut buf)
                .await
                .map_err(|e| AppError::Sftp(format!("read: {e}")))?;
            if n == 0 {
                break;
            }
            local_file.write_all(&buf[..n]).await?;
            transferred += n as u64;
        }

        remote_file
            .shutdown()
            .await
            .map_err(|e| AppError::Sftp(format!("close: {e}")))?;

        Ok(transferred)
    }

    /// Stream-download a remote file to a local path, emitting progress events.
    pub async fn download_streaming(
        &self,
        remote_path: &str,
        local_path: &Path,
        app: &tauri::AppHandle,
        transfer_id: &str,
    ) -> AppResult<u64> {
        use tauri::Emitter;

        // Get file size for progress
        let meta = self.sftp.metadata(remote_path).await
            .map_err(|e| AppError::Sftp(format!("{e}")))?;
        let total = meta.size.unwrap_or(0);

        let mut remote_file = self.sftp.open(remote_path).await
            .map_err(|e| AppError::Sftp(format!("{e}")))?;

        let mut local_file = tokio::fs::File::create(local_path).await?;

        let mut transferred: u64 = 0;
        let mut buf = vec![0u8; 32768];

        loop {
            let n = remote_file.read(&mut buf).await
                .map_err(|e| AppError::Sftp(format!("read: {e}")))?;
            if n == 0 { break; }

            local_file.write_all(&buf[..n]).await?;
            transferred += n as u64;

            let _ = app.emit(
                "sftp:progress",
                serde_json::json!({ "id": transfer_id, "transferred": transferred, "total": total }),
            );
        }

        remote_file.shutdown().await
            .map_err(|e| AppError::Sftp(format!("close: {e}")))?;

        Ok(transferred)
    }

    /// Stream-upload a local file to a remote path, emitting progress events.
    pub async fn upload_streaming(
        &self,
        local_path: &Path,
        remote_path: &str,
        app: &tauri::AppHandle,
        transfer_id: &str,
    ) -> AppResult<u64> {
        use tauri::Emitter;

        let local_meta = tokio::fs::metadata(local_path).await?;
        let total = local_meta.len();

        let mut local_file = tokio::fs::File::open(local_path).await?;

        let mut remote_file = self.sftp.create(remote_path).await
            .map_err(|e| AppError::Sftp(format!("{e}")))?;

        let mut transferred: u64 = 0;
        let mut buf = vec![0u8; 32768];

        loop {
            let n = local_file.read(&mut buf).await?;
            if n == 0 { break; }

            remote_file.write_all(&buf[..n]).await
                .map_err(|e| AppError::Sftp(format!("write: {e}")))?;
            transferred += n as u64;

            let _ = app.emit(
                "sftp:progress",
                serde_json::json!({ "id": transfer_id, "transferred": transferred, "total": total }),
            );
        }

        remote_file.shutdown().await
            .map_err(|e| AppError::Sftp(format!("close: {e}")))?;

        Ok(transferred)
    }
}
