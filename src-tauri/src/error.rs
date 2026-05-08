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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 前端 `errMsg()` 硬编码识别这个前缀，改了字面值即破整套 i18n 翻译。
    const PROTO_PREFIX: &str = "__rssh_err__|";

    // ── 字节级 wire format 钉死 ────────────────────────────────────
    //
    // 协议契约 = 字节流，不是 JSON 等价类。下面这组断言用 `assert_eq!`
    // 钉死完整字符串，覆盖：前缀、字段顺序（code 在前 params 在后）、无空白、
    // 转义规则。serde_json 升级或谁手贱改 Display 模板，这里立刻红。
    // 后面的 shape 测试保留——它们记录"我们关心什么语义"，但只有这组
    // 字节级测试能 catch wire format 漂移。

    #[test]
    fn coded_msg_display_exact_value() {
        let m = CodedMsg::new("foo", json!({"x": 1}));
        assert_eq!(
            m.to_string(),
            r#"__rssh_err__|{"code":"foo","params":{"x":1}}"#
        );
    }

    #[test]
    fn coded_msg_display_empty_params_exact_value() {
        let m = CodedMsg::new("ssh_auth_rejected", json!({}));
        assert_eq!(
            m.to_string(),
            r#"__rssh_err__|{"code":"ssh_auth_rejected","params":{}}"#
        );
    }

    #[test]
    fn coded_msg_display_string_param_exact_value() {
        // 字符串 param 的转义形态：必须用 \" 而不是 ' 等
        let m = CodedMsg::new("ssh_connect_failed", json!({"err": "timeout"}));
        assert_eq!(
            m.to_string(),
            r#"__rssh_err__|{"code":"ssh_connect_failed","params":{"err":"timeout"}}"#
        );
    }

    #[test]
    fn app_error_lock_exact_value() {
        // Lock 没装 CodedMsg，靠 thiserror 模板。这里钉死该模板渲染结果，
        // 改了 #[error("...")] 文字立刻红——必须和 CodedMsg::Display 输出
        // 字节相同。
        assert_eq!(
            AppError::Lock.to_string(),
            r#"__rssh_err__|{"code":"lock_poisoned","params":{}}"#
        );
    }

    #[test]
    fn app_error_ssh_variant_exact_value() {
        let e = AppError::ssh("ssh_connect_failed", json!({"host": "h", "port": 22}));
        assert_eq!(
            e.to_string(),
            r#"__rssh_err__|{"code":"ssh_connect_failed","params":{"host":"h","port":22}}"#
        );
    }

    #[test]
    fn app_error_serialize_for_tauri_exact_value() {
        // Tauri 把 AppError 当 string 序列化送给前端：JSON 字符串字面量
        // 形态 = `"__rssh_err__|{...}"`（外层加引号 + 内层 JSON 中的 " 被转义）。
        let e = AppError::not_found("profile_not_found", json!({"id": "abc"}));
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(
            json,
            r#""__rssh_err__|{\"code\":\"profile_not_found\",\"params\":{\"id\":\"abc\"}}""#
        );
    }

    #[test]
    fn from_io_error_exact_value() {
        // 路径 io::Error → AppError::Io → wire format。断言 code 是 io_error
        // 且 params.err 携带 io 错误的 Display 文案。
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "boom");
        let e: AppError = io.into();
        assert_eq!(
            e.to_string(),
            r#"__rssh_err__|{"code":"io_error","params":{"err":"boom"}}"#
        );
    }

    // ── 结构语义断言（保留）───────────────────────────────────────

    #[test]
    fn coded_msg_display_starts_with_prefix() {
        let m = CodedMsg::new("foo", json!({"x": 1}));
        let s = m.to_string();
        assert!(s.starts_with(PROTO_PREFIX), "got {s}");
    }

    #[test]
    fn coded_msg_display_carries_code_and_params_as_json() {
        let m = CodedMsg::new("ssh_connect_failed", json!({"err": "timeout"}));
        let s = m.to_string();
        let payload = s.strip_prefix(PROTO_PREFIX).unwrap();
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(v["code"], "ssh_connect_failed");
        assert_eq!(v["params"]["err"], "timeout");
    }

    #[test]
    fn coded_msg_display_handles_empty_params() {
        let m = CodedMsg::new("lock_poisoned", json!({}));
        let s = m.to_string();
        let payload = s.strip_prefix(PROTO_PREFIX).unwrap();
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(v["code"], "lock_poisoned");
        assert!(v["params"].is_object());
        assert!(v["params"].as_object().unwrap().is_empty());
    }

    #[test]
    fn coded_msg_display_handles_param_types() {
        // 前端 t() 接 string|number 占位符；嵌套对象也允许（虽然 t 不展开）
        let m = CodedMsg::new(
            "x",
            json!({"s": "hello", "n": 42, "nested": {"k": "v"}}),
        );
        let s = m.to_string();
        let payload = s.strip_prefix(PROTO_PREFIX).unwrap();
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(v["params"]["s"], "hello");
        assert_eq!(v["params"]["n"], 42);
        assert_eq!(v["params"]["nested"]["k"], "v");
    }

    #[test]
    fn app_error_variants_serialize_through_protocol() {
        // 抽几个有代表性的变体，确认 Display impl 都走 CodedMsg 那条路
        let cases = [
            (
                AppError::ssh("ssh_connect_failed", json!({"host": "h"})),
                "ssh_connect_failed",
            ),
            (
                AppError::sftp("sftp_io_failed", json!({})),
                "sftp_io_failed",
            ),
            (AppError::pty("pty_error", json!({})), "pty_error"),
            (
                AppError::not_found("profile_not_found", json!({"id": "x"})),
                "profile_not_found",
            ),
            (
                AppError::config("bad_config", json!({})),
                "bad_config",
            ),
            (AppError::other("oops", json!({})), "oops"),
        ];
        for (err, expected_code) in cases {
            let s = err.to_string();
            assert!(s.starts_with(PROTO_PREFIX), "missing prefix in {s}");
            let payload = s.strip_prefix(PROTO_PREFIX).unwrap();
            let v: serde_json::Value = serde_json::from_str(payload).unwrap();
            assert_eq!(v["code"], expected_code);
            assert_eq!(err.code(), expected_code);
        }
    }

    #[test]
    fn app_error_lock_variant_uses_hardcoded_format() {
        // Lock 变体没装 CodedMsg，靠 thiserror 模板字符串。和 CodedMsg 串
        // **必须**外观一致，否则前端识别不出来。
        let s = AppError::Lock.to_string();
        assert!(s.starts_with(PROTO_PREFIX), "got {s}");
        let payload = s.strip_prefix(PROTO_PREFIX).unwrap();
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(v["code"], "lock_poisoned");
        assert_eq!(AppError::Lock.code(), "lock_poisoned");
    }

    #[test]
    fn from_io_error_wraps_with_io_error_code() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "boom");
        let e: AppError = io.into();
        assert_eq!(e.code(), "io_error");
        let s = e.to_string();
        assert!(s.starts_with(PROTO_PREFIX));
    }

    #[test]
    fn serialize_for_tauri_command_returns_protocol_string() {
        // Tauri 把 AppError 序列化送给前端——必须是单一 string 形态，
        // 内含协议串。如果谁把 #[derive(Serialize)] 装上去（变成 enum），
        // 前端 errMsg() 会拿到对象而非字符串，整套翻译走不通。
        let e = AppError::ssh("x", json!({}));
        let json = serde_json::to_string(&e).unwrap();
        // serde_json 把 string 序列化成带引号的 JSON 字符串字面量
        assert!(json.starts_with(&format!("\"{PROTO_PREFIX}")), "got {json}");
        assert!(json.ends_with('"'));
    }
}
