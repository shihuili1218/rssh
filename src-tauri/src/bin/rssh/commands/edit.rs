//! `rssh edit <profile|cred|fwd> <name>` —— 交互式修改。

use rssh_lib::error::{AppError, AppResult};
use rssh_lib::models::CredentialType;

use crate::ctx::CliCtx;
use crate::helpers::{
    confirm, die, load_cred_secrets, prompt_default, read_multiline, read_password,
    update_cred_with_secrets,
};

pub fn cmd_edit(conn: &CliCtx, kind: &str, name: &str) -> AppResult<()> {
    match kind {
        "profile" => edit_profile(conn, name),
        "cred" | "creds" => edit_credential(conn, name),
        "fwd" => edit_forward(conn, name),
        _ => Err(AppError::config(
            "cli_unknown_kind",
            serde_json::json!({ "kind": kind, "valid": "profile, cred, fwd" }),
        )),
    }
}

fn edit_profile(conn: &CliCtx, name: &str) -> AppResult<()> {
    let profiles = rssh_lib::db::profile::list(conn)?;
    let p = profiles
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| die(format!("Profile '{name}' not found")));

    let mut updated = p.clone();
    updated.name = prompt_default("Name", &p.name);
    updated.host = prompt_default("Host", &p.host);
    updated.port = prompt_default("Port", &p.port.to_string())
        .parse()
        .unwrap_or(p.port);

    let creds = rssh_lib::db::credential::list(conn)?;
    if !creds.is_empty() {
        let cur = p
            .credential_id
            .as_deref()
            .and_then(|id| creds.iter().position(|c| c.id == id))
            .map(|i| (i + 1).to_string())
            .unwrap_or("0".into());
        println!("Credentials:");
        println!("  0 - none");
        for (i, c) in creds.iter().enumerate() {
            println!("  {} - {} ({})", i + 1, c.name, c.username);
        }
        let choice = prompt_default("Credential #", &cur);
        updated.credential_id = choice
            .parse::<usize>()
            .ok()
            .and_then(|n| creds.get(n.wrapping_sub(1)))
            .map(|c| c.id.clone());
    }

    updated.init_command = {
        let cur = p.init_command.as_deref().unwrap_or("");
        let v = prompt_default("Init command", cur);
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    };

    let groups = rssh_lib::db::group::list(conn)?;
    if !groups.is_empty() {
        let cur = p
            .group_id
            .as_deref()
            .and_then(|id| groups.iter().position(|g| g.id == id))
            .map(|i| (i + 1).to_string())
            .unwrap_or("0".into());
        println!("Group:");
        println!("  0 - none");
        for (i, g) in groups.iter().enumerate() {
            println!("  {} - {}", i + 1, g.name);
        }
        let choice = prompt_default("Group #", &cur);
        updated.group_id = choice
            .parse::<usize>()
            .ok()
            .and_then(|n| groups.get(n.wrapping_sub(1)))
            .map(|g| g.id.clone());
    }

    rssh_lib::db::profile::update(conn, &updated)?;
    println!("Profile '{}' updated.", updated.name);
    Ok(())
}

fn edit_credential(conn: &CliCtx, name: &str) -> AppResult<()> {
    let creds = rssh_lib::db::credential::list(conn)?;
    let c = creds
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| die(format!("Credential '{name}' not found")));

    // 把 SecretStore 里的 secret 灌进当前值，便于后面"保留"判定
    let mut updated = load_cred_secrets(conn, c.clone())?;
    updated.name = prompt_default("Name", &c.name);
    updated.username = prompt_default("Username", &c.username);

    println!("Auth type (current: {}):", c.credential_type.as_str());
    println!("  1 - password  2 - key  3 - agent  4 - none  Enter - keep");
    let choice = prompt_default("Type #", "");
    match choice.as_str() {
        "1" => {
            updated.credential_type = CredentialType::Password;
            updated.secret = Some(read_password("Password: "));
        }
        "2" => {
            updated.credential_type = CredentialType::Key;
            println!("Paste private key (end with empty line):");
            updated.secret = Some(read_multiline());
        }
        "3" => {
            updated.credential_type = CredentialType::Agent;
            updated.secret = None;
        }
        "4" => {
            updated.credential_type = CredentialType::None;
            updated.secret = None;
        }
        _ => {
            // Keep current type/secret; passphrase 已由 OpenSSH 在使用时索取
        }
    }

    updated.save_to_remote = confirm("Sync secret to GitHub?", c.save_to_remote);

    update_cred_with_secrets(conn, &updated)?;
    println!("Credential '{}' updated.", updated.name);
    Ok(())
}

fn edit_forward(conn: &CliCtx, name: &str) -> AppResult<()> {
    let forwards = rssh_lib::db::forward::list(conn)?;
    let f = forwards
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| die(format!("Forward '{name}' not found")));

    let mut updated = f.clone();
    updated.name = prompt_default("Name", &f.name);
    updated.local_port = prompt_default("Local port", &f.local_port.to_string())
        .parse()
        .unwrap_or(f.local_port);
    updated.remote_host = prompt_default("Remote host", &f.remote_host);
    updated.remote_port = prompt_default("Remote port", &f.remote_port.to_string())
        .parse()
        .unwrap_or(f.remote_port);

    rssh_lib::db::forward::update(conn, &updated)?;
    println!("Forward '{}' updated.", updated.name);
    Ok(())
}
