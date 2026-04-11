use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

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
    pub async fn connect(
        host: &str,
        port: u16,
        credential: &Credential,
        known_hosts_path: PathBuf,
    ) -> AppResult<Self> {
        let config = Arc::new(russh::client::Config::default());
        let mut handle = client::ssh_connect(config, host, port, known_hosts_path).await
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
}
