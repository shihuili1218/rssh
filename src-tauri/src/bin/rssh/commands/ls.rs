//! `rssh <profile|credential|forward> list` —— entity listing.

use rssh_lib::error::AppResult;
use rssh_lib::models::{ForwardType, Profile};

use crate::ctx::CliCtx;
use crate::helpers::hex_to_rgb;

pub fn cmd_list_profiles(conn: &CliCtx, query: Option<&str>) -> AppResult<()> {
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
        let user = creds
            .iter()
            .find(|c| c.id == p.credential_id)
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
    Ok(())
}

pub fn cmd_list_credentials(conn: &CliCtx) -> AppResult<()> {
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
    Ok(())
}

pub fn cmd_list_forwards(conn: &CliCtx) -> AppResult<()> {
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
    Ok(())
}
