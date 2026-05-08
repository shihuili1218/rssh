//! `rssh ls [query|cred|fwd]` 与 `rssh _names <kind>`（tab completion 用）。

use rssh_lib::error::AppResult;
use rssh_lib::models::{ForwardType, Profile};

use crate::ctx::CliCtx;
use crate::helpers::hex_to_rgb;

pub fn cmd_ls(conn: &CliCtx, query: Option<&str>) -> AppResult<()> {
    match query {
        Some("cred") | Some("creds") => {
            let list = rssh_lib::db::credential::list(conn)?;
            if list.is_empty() {
                println!("No credentials.");
                return Ok(());
            }
            println!("{:<20} {:<15} {:<10}", "NAME", "USER", "TYPE");
            println!("{}", "-".repeat(48));
            for c in &list {
                println!(
                    "{:<20} {:<15} {:<10}",
                    c.name,
                    c.username,
                    c.credential_type.as_str()
                );
            }
        }
        Some("fwd") => {
            let list = rssh_lib::db::forward::list(conn)?;
            if list.is_empty() {
                println!("No forwards.");
                return Ok(());
            }
            let profiles = rssh_lib::db::profile::list(conn)?;
            for f in &list {
                let pname = profiles
                    .iter()
                    .find(|p| p.id == f.profile_id)
                    .map(|p| p.name.as_str())
                    .unwrap_or("?");
                let arrow = match f.forward_type {
                    ForwardType::Local => {
                        format!("-L {} → {}:{}", f.local_port, f.remote_host, f.remote_port)
                    }
                    ForwardType::Remote => {
                        format!("-R {} → {}:{}", f.remote_port, f.remote_host, f.local_port)
                    }
                    ForwardType::Dynamic => format!("-D {}", f.local_port),
                };
                println!("{} ({}) via {}", f.name, arrow, pname);
            }
        }
        _ => {
            let list = rssh_lib::db::profile::list(conn)?;
            let filtered: Vec<&Profile> = match query {
                Some(q) => {
                    let q = q.to_lowercase();
                    list.iter()
                        .filter(|p| {
                            p.name.to_lowercase().contains(&q) || p.host.to_lowercase().contains(&q)
                        })
                        .collect()
                }
                None => list.iter().collect(),
            };
            if filtered.is_empty() {
                println!("No profiles.");
                return Ok(());
            }
            let creds = rssh_lib::db::credential::list(conn)?;
            let groups = rssh_lib::db::group::list(conn)?;
            for p in &filtered {
                let user = p
                    .credential_id
                    .as_deref()
                    .and_then(|id| creds.iter().find(|c| c.id == id))
                    .map(|c| c.username.as_str())
                    .unwrap_or("?");
                let label = format!("{} ({}@{}:{})", p.name, user, p.host, p.port);
                if let Some(g) = p
                    .group_id
                    .as_deref()
                    .and_then(|gid| groups.iter().find(|g| g.id == gid))
                {
                    let (r, gv, b) = hex_to_rgb(&g.color);
                    println!("\x1b[38;2;{};{};{}m{}\x1b[0m", r, gv, b, label);
                } else {
                    println!("{}", label);
                }
            }
        }
    }
    Ok(())
}

/// 隐藏命令 `rssh _names <kind>` —— 给 zsh/bash/fish/powershell completion 拉名字列表。
pub fn cmd_names(conn: &CliCtx, kind: &str) -> AppResult<()> {
    match kind {
        "profiles" | "profile" => {
            for p in rssh_lib::db::profile::list(conn)? {
                println!("{}", p.name);
            }
        }
        "cred" | "creds" => {
            for c in rssh_lib::db::credential::list(conn)? {
                println!("{}", c.name);
            }
        }
        "fwd" => {
            for f in rssh_lib::db::forward::list(conn)? {
                println!("{}", f.name);
            }
        }
        _ => {}
    }
    Ok(())
}
