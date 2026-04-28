use std::sync::{Mutex, MutexGuard};

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

    #[error("锁已中毒")]
    Lock,

    /// i18n 错误码 + 参数。前端按 `error.<code>` 翻译。
    /// 序列化形态：`__rssh_err__|{"code":"...","params":{...}}`，前端识别前缀走翻译表，
    /// 否则原样显示。这样老的 String 错误不破坏。
    #[error("__rssh_err__|{0}")]
    Coded(String),

    #[error("{0}")]
    Other(String),
}

impl AppError {
    /// 构造一个 i18n 错误：`code` 对应前端 `error.<code>` 翻译键，`params` 用于占位符替换。
    pub fn coded(code: &'static str, params: serde_json::Value) -> Self {
        let payload = serde_json::json!({ "code": code, "params": params });
        Self::Coded(payload.to_string())
    }
}

/// Acquire a std::sync::Mutex lock, mapping PoisonError to AppError::Lock.
/// Replaces the repeated `.lock().map_err(|_| AppError::Other("..lock..".into()))` pattern.
pub fn locked<T>(m: &Mutex<T>) -> AppResult<MutexGuard<'_, T>> {
    m.lock().map_err(|_| AppError::Lock)
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
