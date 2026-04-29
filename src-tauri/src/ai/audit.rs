//! 内存审计日志 + 保存到人类可读文本。
//!
//! 决议 #4：审计 panel 时间线一目了然，用户主动点保存才落盘。
//! 文件名形如 `rssh-diagnose-<session>-<ts>.log`。

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub at: DateTime<Utc>,
    pub kind: AuditKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditKind {
    SessionStarted {
        skill: String,
        target: String, // ssh:<id> 或 local:<id>
    },
    SessionEnded,
    LlmRequest {
        model: String,
        redacted_payload: String,
    },
    LlmResponse {
        text: String,
        tokens_in: Option<u32>,
        tokens_out: Option<u32>,
    },
    CommandProposed {
        id: String,
        cmd: String,
        explain: String,
        side_effect: String,
    },
    CommandRejected {
        id: String,
        reason: String,
    },
    CommandExecuted {
        id: String,
        exit_code: i32,
        output_redacted: String,
        original_bytes: usize,
        truncated_bytes: usize,
        duration_ms: u64,
    },
    DownloadProposed {
        id: String,
        remote_path: String,
        max_mb: u32,
    },
    DownloadCompleted {
        id: String,
        local_path: String,
        bytes: u64,
    },
    Note {
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AuditLog {
    pub entries: Vec<AuditEntry>,
}

impl AuditLog {
    pub fn push(&mut self, kind: AuditKind) {
        self.entries.push(AuditEntry {
            at: Utc::now(),
            kind,
        });
    }

    /// 保存为人类可读文本（grep / less 友好）。
    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        let mut s = String::new();
        s.push_str(&format!(
            "# rssh AI 排障审计\n# 共 {} 条记录\n# 生成时间: {}\n\n",
            self.entries.len(),
            Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        ));
        for e in &self.entries {
            s.push_str(&format!("[{}] ", e.at.format("%Y-%m-%d %H:%M:%S%.3fZ")));
            match &e.kind {
                AuditKind::SessionStarted { skill, target } => {
                    s.push_str(&format!("SESSION_STARTED  skill={skill} target={target}\n"));
                }
                AuditKind::SessionEnded => s.push_str("SESSION_ENDED\n"),
                AuditKind::LlmRequest {
                    model,
                    redacted_payload,
                } => {
                    s.push_str(&format!("LLM_REQUEST      model={model}\n"));
                    s.push_str("---PAYLOAD (脱敏后)---\n");
                    s.push_str(redacted_payload);
                    s.push_str("\n---END---\n");
                }
                AuditKind::LlmResponse {
                    text,
                    tokens_in,
                    tokens_out,
                } => {
                    s.push_str(&format!(
                        "LLM_RESPONSE     in={} out={}\n",
                        fmt_opt(tokens_in),
                        fmt_opt(tokens_out)
                    ));
                    s.push_str("---TEXT---\n");
                    s.push_str(text);
                    s.push_str("\n---END---\n");
                }
                AuditKind::CommandProposed {
                    id,
                    cmd,
                    explain,
                    side_effect,
                } => {
                    s.push_str(&format!("CMD_PROPOSED     id={id}\n"));
                    s.push_str(&format!("  cmd:        {cmd}\n"));
                    s.push_str(&format!("  含义:       {explain}\n"));
                    s.push_str(&format!("  副作用:     {side_effect}\n"));
                }
                AuditKind::CommandRejected { id, reason } => {
                    s.push_str(&format!("CMD_REJECTED     id={id} reason={reason}\n"));
                }
                AuditKind::CommandExecuted {
                    id,
                    exit_code,
                    output_redacted,
                    original_bytes,
                    truncated_bytes,
                    duration_ms,
                } => {
                    s.push_str(&format!(
                        "CMD_EXECUTED     id={id} exit={exit_code} bytes={original_bytes} truncated={truncated_bytes} dur={duration_ms}ms\n"
                    ));
                    s.push_str("---OUTPUT (脱敏后)---\n");
                    s.push_str(output_redacted);
                    s.push_str("\n---END---\n");
                }
                AuditKind::DownloadProposed {
                    id,
                    remote_path,
                    max_mb,
                } => {
                    s.push_str(&format!(
                        "DOWNLOAD_PROPOSED id={id} remote={remote_path} max_mb={max_mb}\n"
                    ));
                }
                AuditKind::DownloadCompleted {
                    id,
                    local_path,
                    bytes,
                } => {
                    s.push_str(&format!(
                        "DOWNLOAD_DONE    id={id} local={local_path} bytes={bytes}\n"
                    ));
                }
                AuditKind::Note { message } => {
                    s.push_str(&format!("NOTE             {message}\n"));
                }
                AuditKind::Error { message } => {
                    s.push_str(&format!("ERROR            {message}\n"));
                }
            }
            s.push('\n');
        }
        std::fs::write(path, s)
    }
}

fn fmt_opt<T: std::fmt::Display>(v: &Option<T>) -> String {
    v.as_ref()
        .map(|x| x.to_string())
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_round_trip() {
        let mut log = AuditLog::default();
        log.push(AuditKind::SessionStarted {
            skill: "cpu-java".into(),
            target: "ssh:abc".into(),
        });
        log.push(AuditKind::CommandProposed {
            id: "c1".into(),
            cmd: "uname -a".into(),
            explain: "探明 OS".into(),
            side_effect: "只读".into(),
        });
        assert_eq!(log.entries.len(), 2);
    }

    #[test]
    fn save_to_temp_file() {
        let mut log = AuditLog::default();
        log.push(AuditKind::Note {
            message: "测试".into(),
        });
        let path = std::env::temp_dir().join("rssh-audit-test.log");
        log.save_to_file(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("NOTE"));
        assert!(content.contains("测试"));
        let _ = std::fs::remove_file(&path);
    }
}
