use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("数据库错误: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("SSH 连接失败: {0}")]
    Ssh(String),

    #[error("SFTP 操作失败: {0}")]
    Sftp(String),

    #[error("PTY 错误: {0}")]
    Pty(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("未找到: {0}")]
    NotFound(String),

    #[error("配置错误: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
