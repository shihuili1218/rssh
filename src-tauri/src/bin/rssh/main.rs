//! `rssh` / `rssh-cli` CLI 入口。
//!
//! 所有具体子命令实现在 `commands::*`，IO/格式 helper 在 `helpers::*`。
//! 本文件只负责：
//! - clap 命令枚举定义
//! - main() 派发
//! - Linux 上 GUI shadow 启动（CLI 不带子命令 + 有 DISPLAY → fork GUI）
//! - lib AppError → CLI 用户可读字符串

use std::sync::{Arc, OnceLock};

use clap::{Parser, Subcommand};

use rssh_lib::db::Db;

mod commands;
mod ctx;
mod helpers;

use commands::config::ConfigCmd;
use ctx::CliCtx;

// ═══════════════════════════════════════════════════════════════════
// CLI definition
// ═══════════════════════════════════════════════════════════════════

#[derive(Parser)]
#[command(name = "rssh", version, about = "RSSH — SSH connection manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// List profiles (default), credentials, or forwards
    Ls {
        /// "cred", "fwd", or a name filter for profiles
        query: Option<String>,
    },
    /// Connect via SSH, or start a port forward
    Open {
        /// Profile name, or "fwd" for port forward
        target: String,
        /// Forward name (when target is "fwd")
        name: Option<String>,
    },
    /// Add a profile, credential, or forward
    Add {
        /// "profile", "cred", or "fwd"
        kind: String,
    },
    /// Edit a profile, credential, or forward
    Edit { kind: String, name: String },
    /// Remove a profile, credential, or forward
    Rm { kind: String, name: String },
    /// Configuration: export, import, GitHub sync
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// Generate shell completion script
    Completions {
        /// "zsh" or "bash"
        shell: String,
    },
    /// (hidden) Output entity names for tab completion
    #[command(hide = true, name = "_names")]
    Names { kind: String },
}

// ═══════════════════════════════════════════════════════════════════
// main
// ═══════════════════════════════════════════════════════════════════

/// On Linux the CLI is installed as `/usr/local/bin/rssh`, which shadows the
/// GUI binary at `/usr/bin/rssh`.  When invoked without a subcommand, detect
/// the GUI binary and launch it instead — so `rssh` opens the app, while
/// `rssh ls`, `rssh open …` etc. still go through the CLI path.
#[cfg(target_os = "linux")]
fn try_launch_gui() -> bool {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    if commands::open::in_rssh_app() {
        return false;
    }
    // No display server → headless / SSH session, stay in CLI.
    if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
        return false;
    }
    let gui = std::path::PathBuf::from("/usr/bin/rssh");
    if !gui.exists() {
        return false;
    }
    // Don't loop when the user runs the GUI binary directly.
    if let Ok(me) = std::env::current_exe() {
        if me.canonicalize().ok() == gui.canonicalize().ok() {
            return false;
        }
    }
    Command::new(&gui)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .is_ok()
}

/// 把 lib 抛上来的 `AppError` 渲染成 CLI 可读的英文。
///
/// 业务变体（Ssh/Sftp/...）的 Display 是 `__rssh_err__|{json}` 协议字符串，
/// 给前端 `errMsg()` 翻译用。CLI 没有 catalog，脱壳显示成 `<code>(<params>)`，
/// 开发者一看就知道发生了什么；找不到协议前缀就原样输出（Database/Io 等模板）。
fn format_lib_error(e: &rssh_lib::error::AppError) -> String {
    let s = e.to_string();
    let Some(payload_json) = s.strip_prefix("__rssh_err__|") else {
        return s;
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(payload_json) else {
        return s;
    };
    let code = payload.get("code").and_then(|c| c.as_str()).unwrap_or("error");
    let params = payload.get("params").and_then(|p| p.as_object());
    match params {
        Some(o) if !o.is_empty() => {
            let parts: Vec<String> = o
                .iter()
                .map(|(k, v)| match v.as_str() {
                    Some(s) => format!("{k}={s}"),
                    None => format!("{k}={v}"),
                })
                .collect();
            format!("{code} ({})", parts.join(", "))
        }
        _ => code.to_string(),
    }
}

fn main() {
    let cli = Cli::parse();

    // No subcommand → try launching GUI on Linux.
    #[cfg(target_os = "linux")]
    if cli.command.is_none() && try_launch_gui() {
        return;
    }

    let data_dir = rssh_lib::db::data_dir();
    let db = Arc::new(Db::open(&data_dir).unwrap_or_else(|e| {
        eprintln!("Failed to open database: {e}");
        std::process::exit(1);
    }));
    let conn = CliCtx {
        db,
        secret_store: OnceLock::new(),
    };

    let result = match cli.command {
        None => commands::ls::cmd_ls(&conn, None),
        Some(Cmd::Ls { query }) => commands::ls::cmd_ls(&conn, query.as_deref()),
        Some(Cmd::Open { target, name }) => commands::open::cmd_open(&conn, &target, name.as_deref()),
        Some(Cmd::Add { kind }) => commands::add::cmd_add(&conn, &kind),
        Some(Cmd::Edit { kind, name }) => commands::edit::cmd_edit(&conn, &kind, &name),
        Some(Cmd::Rm { kind, name }) => commands::rm::cmd_rm(&conn, &kind, &name),
        Some(Cmd::Config { action }) => commands::config::cmd_config(&conn, action),
        Some(Cmd::Completions { shell }) => {
            commands::completions::print_completions(&shell);
            Ok(())
        }
        Some(Cmd::Names { kind }) => commands::ls::cmd_names(&conn, &kind),
    };

    if let Err(e) = result {
        eprintln!("error: {}", format_lib_error(&e));
        std::process::exit(1);
    }
}
