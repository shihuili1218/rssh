//! `rssh open <profile|fwd <name>>` —— 直接 exec ssh，或在 GUI 内嵌终端里
//! emit OSC 7337 让宿主接管。

use std::process::Command;

use rssh_lib::error::AppResult;
use rssh_lib::models::ForwardType;

use crate::ctx::CliCtx;
use crate::helpers::{build_ssh_command, die, load_cred_secrets};

pub fn in_rssh_app() -> bool {
    std::env::var("RSSH_APP").is_ok()
}

/// 把 `kind:name` 编进 OSC 7337，让宿主 GUI 终端 catch 后接管。
/// 防御式校验 name —— DB 层（`db::profile/forward::insert/update`）已经拒绝
/// 控制符，理论上 osc_open 拿不到坏值。这里再校一次：万一 DB 被外部工具
/// 写了脏数据 / 旧版本残留，宁可拒绝打印也不让恶意 ESC/BEL 注入终端。
fn osc_open(kind: &str, name: &str) -> AppResult<()> {
    rssh_lib::models::validate_name(name)?;
    // OSC 7337 ; <kind>:<name> ST   （kind 永远是字面量 "open" / "fwd"，不需校）
    print!("\x1b]7337;{}:{}\x07", kind, name);
    Ok(())
}

pub fn cmd_open(conn: &CliCtx, target: &str, name: Option<&str>) -> AppResult<()> {
    if target == "fwd" {
        let fname = name.unwrap_or_else(|| die("Usage: rssh open fwd <name>"));
        if in_rssh_app() {
            return osc_open("fwd", fname);
        }
        return cmd_open_fwd(conn, fname);
    }
    if in_rssh_app() {
        return osc_open("open", target);
    }
    cmd_open_ssh(conn, target)
}

fn cmd_open_ssh(conn: &CliCtx, name: &str) -> AppResult<()> {
    let profiles = rssh_lib::db::profile::list(conn)?;
    let profile = profiles
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| die(format!("Profile '{name}' not found")));

    // credential_id 配错（如 DB 不一致 / 引用已删 cred）必须显式报错；旧版的
    // .ok() 会把这种情况降级为"无凭证"，导致 ssh 走默认 key fallback，错的离谱。
    let cred = match profile.credential_id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => Some(load_cred_secrets(conn, rssh_lib::db::credential::get(conn, id)?)?),
        None => None,
    };

    let mut cmd = Command::new("ssh");
    let key_files = build_ssh_command(&mut cmd, conn, profile, cred.as_ref())?;

    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");

    // init_command: run it then hand off to shell
    if let Some(ref init) = profile.init_command {
        if !init.is_empty() {
            cmd.arg("-t")
                .arg(&profile.host)
                .arg(format!("{}; exec $SHELL -l", init));
        } else {
            cmd.arg(&profile.host);
        }
    } else {
        cmd.arg(&profile.host);
    }

    let status = cmd
        .status()
        .unwrap_or_else(|e| die(format!("Failed to run ssh: {e}")));
    let code = status.code().unwrap_or(1);
    // 必须显式 drop —— `process::exit` 跳过 stack unwind，NamedTempFile 的
    // Drop 不会跑，私钥临时文件留在磁盘。
    drop(key_files);
    std::process::exit(code);
}

fn cmd_open_fwd(conn: &CliCtx, name: &str) -> AppResult<()> {
    let forwards = rssh_lib::db::forward::list(conn)?;
    let fwd = forwards
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| die(format!("Forward '{name}' not found")));

    let profile = rssh_lib::db::profile::get(conn, &fwd.profile_id)?;
    let cred = match profile.credential_id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => Some(load_cred_secrets(conn, rssh_lib::db::credential::get(conn, id)?)?),
        None => None,
    };

    let mut cmd = Command::new("ssh");
    cmd.arg("-N");

    let (flag, fwd_arg) = match fwd.forward_type {
        ForwardType::Local => (
            "-L",
            format!("{}:{}:{}", fwd.local_port, fwd.remote_host, fwd.remote_port),
        ),
        ForwardType::Remote => (
            "-R",
            format!("{}:{}:{}", fwd.remote_port, fwd.remote_host, fwd.local_port),
        ),
        ForwardType::Dynamic => ("-D", format!("{}", fwd.local_port)),
    };
    cmd.arg(flag).arg(&fwd_arg);

    let key_files = build_ssh_command(&mut cmd, conn, &profile, cred.as_ref())?;

    cmd.arg(&profile.host);

    println!("Forwarding {} {} ...", flag, fwd_arg);
    let status = cmd
        .status()
        .unwrap_or_else(|e| die(format!("Failed to run ssh: {e}")));
    let code = status.code().unwrap_or(1);
    // 同 cmd_open_ssh：process::exit 跳过 NamedTempFile 的 Drop，必须先显式 drop。
    drop(key_files);
    std::process::exit(code);
}
