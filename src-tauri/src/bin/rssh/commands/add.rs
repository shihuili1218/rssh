//! `rssh add <profile|cred|fwd>` —— 交互式新增。

use rssh_lib::error::{AppError, AppResult};
use rssh_lib::models::{Credential, CredentialType, Forward, ForwardType, Profile};

use crate::ctx::CliCtx;
use crate::helpers::{
    confirm, menu_select, prompt, prompt_default, prompt_optional, read_multiline, read_password,
    upsert_cred_with_secrets,
};

pub fn cmd_add(conn: &CliCtx, kind: &str) -> AppResult<()> {
    match kind {
        "profile" => add_profile(conn),
        "cred" | "creds" => add_credential(conn),
        "fwd" => add_forward(conn),
        // 返回 Err 让 main 走 exit(1)；返回 Ok = 退出码 0 把无效输入伪装成成功，
        // 脚本调用看不出问题。
        _ => Err(AppError::config(
            "cli_unknown_kind",
            serde_json::json!({ "kind": kind, "valid": "profile, cred, fwd" }),
        )),
    }
}

fn add_profile(conn: &CliCtx) -> AppResult<()> {
    let name = prompt("Name: ");
    let host = prompt("Host: ");
    let port: u16 = prompt_default("Port", "22").parse().unwrap_or(22);

    let creds = rssh_lib::db::credential::list(conn)?;
    let credential_id = menu_select(
        "Credentials:",
        "Credential",
        &creds,
        "(no credentials, use 'rssh add cred' first)",
        |c| format!("{} ({})", c.name, c.username),
    )
    .map(|c| c.id.clone());

    let profiles = rssh_lib::db::profile::list(conn)?;
    let bastion_profile_id = menu_select("Bastion (optional):", "Bastion", &profiles, "", |p| {
        format!("{} ({})", p.name, p.host)
    })
    .map(|p| p.id.clone());

    let init_command = prompt_optional("Init command (optional): ");

    let groups = rssh_lib::db::group::list(conn)?;
    let group_id = menu_select("Group (optional):", "Group", &groups, "", |g| g.name.clone())
        .map(|g| g.id.clone());

    let p = Profile {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        host,
        port,
        credential_id,
        bastion_profile_id,
        init_command,
        group_id,
    };
    rssh_lib::db::profile::insert(conn, &p)?;
    println!("Profile '{}' created.", p.name);
    Ok(())
}

fn add_credential(conn: &CliCtx) -> AppResult<()> {
    let name = prompt("Name: ");
    let username = prompt("Username: ");

    println!("Auth type:");
    println!("  1 - password");
    println!("  2 - key (PEM)");
    println!("  3 - SSH agent (use $SSH_AUTH_SOCK / Pageant)");
    println!("  4 - none");
    let choice = prompt_default("Type #", "1");
    let (credential_type, secret) = match choice.as_str() {
        "2" => {
            println!("Paste private key (end with empty line):");
            let key = read_multiline();
            (CredentialType::Key, Some(key))
        }
        "3" => (CredentialType::Agent, None),
        "4" => (CredentialType::None, None),
        _ => {
            let pw = read_password("Password: ");
            (CredentialType::Password, Some(pw))
        }
    };

    let save_to_remote = confirm("Sync secret to GitHub?", false);

    let c = Credential {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        username,
        credential_type,
        secret,
        save_to_remote,
    };
    upsert_cred_with_secrets(conn, &c)?;
    println!("Credential '{}' created.", c.name);
    Ok(())
}

fn add_forward(conn: &CliCtx) -> AppResult<()> {
    let name = prompt("Name: ");

    println!("Type:");
    println!("  1 - local (-L)");
    println!("  2 - remote (-R)");
    println!("  3 - dynamic (-D, SOCKS5)");
    let ft = match prompt_default("Type #", "1").as_str() {
        "2" => ForwardType::Remote,
        "3" => ForwardType::Dynamic,
        _ => ForwardType::Local,
    };

    let local_port: u16 = prompt("Local port: ").parse().unwrap_or(0);
    let (remote_host, remote_port) = if ft == ForwardType::Dynamic {
        ("127.0.0.1".to_string(), 0u16)
    } else {
        (
            prompt_default("Remote host", "127.0.0.1"),
            prompt("Remote port: ").parse().unwrap_or(0),
        )
    };

    let profiles = rssh_lib::db::profile::list(conn)?;
    if profiles.is_empty() {
        return Err(AppError::config(
            "cli_no_profiles",
            serde_json::json!({}),
        ));
    }
    println!("Profile:");
    for (i, p) in profiles.iter().enumerate() {
        println!("  {} - {} ({})", i + 1, p.name, p.host);
    }
    // 必须真选中一项 —— wrapping_sub(0) 会绕成 usize::MAX，再 unwrap_or_default
    // 写入空 profile_id，DB 里残留孤儿 forward。直接 Err 让 main 输出错误码。
    let pidx = prompt("Profile #: ")
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .ok_or_else(|| AppError::config("cli_invalid_profile_index", serde_json::json!({})))?;
    let profile_id = profiles
        .get(pidx)
        .map(|p| p.id.clone())
        .ok_or_else(|| AppError::config("cli_invalid_profile_index", serde_json::json!({})))?;

    let f = Forward {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        forward_type: ft,
        local_port,
        remote_host,
        remote_port,
        profile_id,
    };
    rssh_lib::db::forward::insert(conn, &f)?;
    println!("Forward '{}' created.", f.name);
    Ok(())
}
