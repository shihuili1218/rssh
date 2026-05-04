use std::sync::{Mutex, MutexGuard};

use serde::Serialize;

/// i18n 错误消息：`code` 对应前端 `error.<code>` 翻译键，`params` 用于占位符替换。
///
/// `Display` 输出形如 `__rssh_err__|{"code":"...","params":{...}}`，前端 `errMsg()`
/// 识别此前缀走翻译表。每个 `AppError` 业务变体都装一个 `CodedMsg`——所有错误
/// 必须 i18n，没有"裸字符串报错信息"的逃生通道。
#[derive(Debug, Clone)]
pub struct CodedMsg {
    pub code: &'static str,
    pub params: serde_json::Value,
}

impl CodedMsg {
    pub fn new(code: &'static str, params: serde_json::Value) -> Self {
        Self { code, params }
    }
}

impl std::fmt::Display for CodedMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let payload = serde_json::json!({ "code": self.code, "params": &self.params });
        write!(f, "__rssh_err__|{payload}")
    }
}

impl std::error::Error for CodedMsg {}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// SQLite 错误 — `From<rusqlite::Error>` 自动包装为 CodedMsg。
    #[error(transparent)]
    Database(CodedMsg),

    /// 标准 IO 错误 — `From<std::io::Error>` 自动包装为 CodedMsg。
    #[error(transparent)]
    Io(CodedMsg),

    /// 锁中毒——编程 bug，固定 i18n code，无参数。
    #[error("__rssh_err__|{{\"code\":\"lock_poisoned\",\"params\":{{}}}}")]
    Lock,

    /// SSH 协议 / 连接 / 认证错误。
    #[error(transparent)]
    Ssh(CodedMsg),

    /// SFTP 操作错误。
    #[error(transparent)]
    Sftp(CodedMsg),

    /// 本地 PTY 错误。
    #[error(transparent)]
    Pty(CodedMsg),

    /// 资源未找到（profile / credential / session …）。
    #[error(transparent)]
    NotFound(CodedMsg),

    /// 配置 / 用户输入校验错误。
    #[error(transparent)]
    Config(CodedMsg),

    /// 不好归到上述具体分类的业务错误：外部 API 错误、内部 channel 状态、
    /// 批处理错误聚合、平台限制等。
    #[error(transparent)]
    Other(CodedMsg),
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(CodedMsg::new(
            "db_error",
            serde_json::json!({ "err": e.to_string() }),
        ))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(CodedMsg::new(
            "io_error",
            serde_json::json!({ "err": e.to_string() }),
        ))
    }
}

impl AppError {
    pub fn ssh(code: &'static str, params: serde_json::Value) -> Self {
        Self::Ssh(CodedMsg::new(code, params))
    }
    pub fn sftp(code: &'static str, params: serde_json::Value) -> Self {
        Self::Sftp(CodedMsg::new(code, params))
    }
    pub fn pty(code: &'static str, params: serde_json::Value) -> Self {
        Self::Pty(CodedMsg::new(code, params))
    }
    pub fn not_found(code: &'static str, params: serde_json::Value) -> Self {
        Self::NotFound(CodedMsg::new(code, params))
    }
    pub fn config(code: &'static str, params: serde_json::Value) -> Self {
        Self::Config(CodedMsg::new(code, params))
    }
    pub fn other(code: &'static str, params: serde_json::Value) -> Self {
        Self::Other(CodedMsg::new(code, params))
    }

    /// 仅取出 i18n code，不带 params——用于嵌套错误聚合，避免把整个协议串塞进
    /// 外层 params。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Database(c)
            | Self::Io(c)
            | Self::Ssh(c)
            | Self::Sftp(c)
            | Self::Pty(c)
            | Self::NotFound(c)
            | Self::Config(c)
            | Self::Other(c) => c.code,
            Self::Lock => "lock_poisoned",
        }
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
