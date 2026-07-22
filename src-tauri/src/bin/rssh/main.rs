//! `rssh` / `rssh-cli` CLI 入口。
//!
//! 所有具体子命令实现在 `commands::*`，IO/格式 helper 在 `helpers::*`。
//! 本文件只负责：
//! - clap 命令枚举定义
//! - main() 派发
//! - Linux 上 GUI shadow 启动（CLI 不带子命令 + 有 DISPLAY → fork GUI）
//! - lib AppError → CLI 用户可读字符串

use std::sync::{Arc, OnceLock};

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{ArgValueCompleter, CompleteEnv};

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
#[command(
    name = "rssh",
    version = rssh_lib::CLI_VERSION,
    about = "RSSH — SSH connection manager"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the CLI version
    Version,
    /// Manage SSH profiles
    Profile {
        #[command(subcommand)]
        action: ProfileCmd,
    },
    /// Manage credentials
    Credential {
        #[command(subcommand)]
        action: CredentialCmd,
    },
    /// Manage port forwards
    Forward {
        #[command(subcommand)]
        action: ForwardCmd,
    },
    /// Manage connection groups
    Group {
        #[command(subcommand)]
        action: GroupCmd,
    },
    /// Configuration: export, import, remote sync (GitHub / WebDAV)
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// Generate shell completion script
    Completions {
        /// "zsh", "bash", "fish", or "powershell" (alias: "pwsh")
        #[arg(value_parser = ["zsh", "bash", "fish", "powershell", "pwsh"])]
        shell: String,
    },
}

#[derive(Subcommand)]
enum ProfileCmd {
    /// List profiles, optionally filtered by name or host
    List { query: Option<String> },
    /// Open an SSH profile
    Open {
        #[arg(add = ArgValueCompleter::new(commands::completions::complete_profiles))]
        name: String,
    },
    /// Add a profile interactively
    Add,
    /// Edit a profile interactively
    Edit {
        #[arg(add = ArgValueCompleter::new(commands::completions::complete_profiles))]
        name: String,
    },
    /// Remove a profile
    Rm {
        #[arg(add = ArgValueCompleter::new(commands::completions::complete_profiles))]
        name: String,
    },
}

#[derive(Subcommand)]
enum CredentialCmd {
    /// List credentials
    List,
    /// Add a credential interactively
    Add,
    /// Edit a credential interactively
    Edit {
        #[arg(add = ArgValueCompleter::new(commands::completions::complete_credentials))]
        name: String,
    },
    /// Remove a credential
    Rm {
        #[arg(add = ArgValueCompleter::new(commands::completions::complete_credentials))]
        name: String,
    },
}

#[derive(Subcommand)]
enum ForwardCmd {
    /// List port forwards
    List,
    /// Open a port forward
    Open {
        #[arg(add = ArgValueCompleter::new(commands::completions::complete_forwards))]
        name: String,
    },
    /// Add a port forward interactively
    Add,
    /// Edit a port forward interactively
    Edit {
        #[arg(add = ArgValueCompleter::new(commands::completions::complete_forwards))]
        name: String,
    },
    /// Remove a port forward
    Rm {
        #[arg(add = ArgValueCompleter::new(commands::completions::complete_forwards))]
        name: String,
    },
}

#[derive(Subcommand)]
enum GroupCmd {
    /// List groups
    List,
    /// Add a group interactively
    Add,
    /// Edit a group interactively
    Edit {
        #[arg(add = ArgValueCompleter::new(commands::completions::complete_groups))]
        name: String,
    },
    /// Remove a group
    Rm {
        #[arg(add = ArgValueCompleter::new(commands::completions::complete_groups))]
        name: String,
    },
}

// ═══════════════════════════════════════════════════════════════════
// main
// ═══════════════════════════════════════════════════════════════════

/// On Linux the CLI is installed as `/usr/local/bin/rssh`, which shadows the
/// GUI binary at `/usr/bin/rssh`.  When invoked without a subcommand, detect
/// the GUI binary and launch it instead — so `rssh` opens the app, while
/// `rssh profile list`, `rssh profile open …` etc. still use the CLI path.
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
    let code = payload
        .get("code")
        .and_then(|c| c.as_str())
        .unwrap_or("error");
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
    CompleteEnv::with_factory(Cli::command)
        .var("_RSSH_COMPLETE")
        .bin("rssh")
        .completer("rssh")
        .complete();

    let cli = Cli::parse();

    // No subcommand → try launching GUI on Linux.
    #[cfg(target_os = "linux")]
    if cli.command.is_none() && try_launch_gui() {
        return;
    }

    // Script generation is independent from HOME, the database, and secret storage.
    // The installed script calls back into the runtime completers on each Tab.
    if let Some(Cmd::Completions { shell }) = cli.command.as_ref() {
        commands::completions::print_completions(shell);
        return;
    }
    if matches!(cli.command, Some(Cmd::Version)) {
        println!("{}", rssh_lib::CLI_VERSION);
        return;
    }

    let data_dir = rssh_lib::db::data_dir().unwrap_or_else(|e| {
        eprintln!("error: {}", format_lib_error(&e));
        std::process::exit(1);
    });
    let db = Arc::new(Db::open(&data_dir).unwrap_or_else(|e| {
        eprintln!("error: {}", format_lib_error(&e));
        std::process::exit(1);
    }));
    let conn = CliCtx {
        db,
        data_dir,
        secret_store: OnceLock::new(),
    };

    let result = match cli.command {
        None => commands::ls::cmd_list_profiles(&conn, None),
        Some(Cmd::Version) => unreachable!("handled before database initialization"),
        Some(Cmd::Profile { action }) => match action {
            ProfileCmd::List { query } => commands::ls::cmd_list_profiles(&conn, query.as_deref()),
            ProfileCmd::Open { name } => commands::open::cmd_open_profile(&conn, &name),
            ProfileCmd::Add => commands::add::cmd_add_profile(&conn),
            ProfileCmd::Edit { name } => commands::edit::cmd_edit_profile(&conn, &name),
            ProfileCmd::Rm { name } => commands::rm::cmd_rm_profile(&conn, &name),
        },
        Some(Cmd::Credential { action }) => match action {
            CredentialCmd::List => commands::ls::cmd_list_credentials(&conn),
            CredentialCmd::Add => commands::add::cmd_add_credential(&conn),
            CredentialCmd::Edit { name } => commands::edit::cmd_edit_credential(&conn, &name),
            CredentialCmd::Rm { name } => commands::rm::cmd_rm_credential(&conn, &name),
        },
        Some(Cmd::Forward { action }) => match action {
            ForwardCmd::List => commands::ls::cmd_list_forwards(&conn),
            ForwardCmd::Open { name } => commands::open::cmd_open_forward(&conn, &name),
            ForwardCmd::Add => commands::add::cmd_add_forward(&conn),
            ForwardCmd::Edit { name } => commands::edit::cmd_edit_forward(&conn, &name),
            ForwardCmd::Rm { name } => commands::rm::cmd_rm_forward(&conn, &name),
        },
        Some(Cmd::Group { action }) => match action {
            GroupCmd::List => commands::group::cmd_list_groups(&conn),
            GroupCmd::Add => commands::group::cmd_add_group(&conn),
            GroupCmd::Edit { name } => commands::group::cmd_edit_group(&conn, &name),
            GroupCmd::Rm { name } => commands::group::cmd_rm_group(&conn, &name),
        },
        Some(Cmd::Config { action }) => commands::config::cmd_config(&conn, action),
        Some(Cmd::Completions { .. }) => unreachable!("handled before database initialization"),
    };

    if let Err(e) = result {
        eprintln!("error: {}", format_lib_error(&e));
        std::process::exit(1);
    }
}
