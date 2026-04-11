use std::io::{self, Write};
use std::process::Command;

use clap::{Parser, Subcommand};
use rusqlite::Connection;

use rssh_lib::db;
use rssh_lib::error::AppResult;
use rssh_lib::models::*;

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
    Edit {
        kind: String,
        name: String,
    },
    /// Remove a profile, credential, or forward
    Rm {
        kind: String,
        name: String,
    },
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
    Names {
        kind: String,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Export encrypted backup
    Export { file: String },
    /// Import from encrypted backup
    Import { file: String },
    /// Set GitHub sync settings
    Set,
    /// Push config to GitHub
    Push,
    /// Pull config from GitHub
    Pull,
}

// ═══════════════════════════════════════════════════════════════════
// main
// ═══════════════════════════════════════════════════════════════════

fn main() {
    let cli = Cli::parse();
    let data_dir = db::data_dir();
    let conn = db::open(&data_dir).unwrap_or_else(|e| {
        eprintln!("Failed to open database: {e}");
        std::process::exit(1);
    });

    let result = match cli.command {
        None => cmd_ls(&conn, None),
        Some(Cmd::Ls { query }) => cmd_ls(&conn, query.as_deref()),
        Some(Cmd::Open { target, name }) => cmd_open(&conn, &target, name.as_deref()),
        Some(Cmd::Add { kind }) => cmd_add(&conn, &kind),
        Some(Cmd::Edit { kind, name }) => cmd_edit(&conn, &kind, &name),
        Some(Cmd::Rm { kind, name }) => cmd_rm(&conn, &kind, &name),
        Some(Cmd::Config { action }) => cmd_config(&conn, action),
        Some(Cmd::Completions { shell }) => {
            print_completions(&shell);
            Ok(())
        }
        Some(Cmd::Names { kind }) => cmd_names(&conn, &kind),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

// ═══════════════════════════════════════════════════════════════════
// ls
// ═══════════════════════════════════════════════════════════════════

fn cmd_ls(conn: &Connection, query: Option<&str>) -> AppResult<()> {
    match query {
        Some("cred") | Some("creds") => {
            let list = db::credential::list(conn)?;
            if list.is_empty() {
                println!("No credentials.");
                return Ok(());
            }
            println!("{:<20} {:<15} {:<10}", "NAME", "USER", "TYPE");
            println!("{}", "-".repeat(48));
            for c in &list {
                println!("{:<20} {:<15} {:<10}", c.name, c.username, c.credential_type.as_str());
            }
        }
        Some("fwd") => {
            let list = db::forward::list(conn)?;
            if list.is_empty() {
                println!("No forwards.");
                return Ok(());
            }
            println!("{:<18} {:<6} {:<8} {:<22} {}", "NAME", "TYPE", "LOCAL", "REMOTE", "PROFILE");
            println!("{}", "-".repeat(70));
            let profiles = db::profile::list(conn)?;
            for f in &list {
                let pname = profiles.iter().find(|p| p.id == f.profile_id).map(|p| p.name.as_str()).unwrap_or("?");
                let ft = match f.forward_type { ForwardType::Local => "L", ForwardType::Remote => "R" };
                println!("{:<18} {:<6} {:<8} {}:{:<17} {}", f.name, ft, f.local_port, f.remote_host, f.remote_port, pname);
            }
        }
        _ => {
            let list = db::profile::list(conn)?;
            let filtered: Vec<&Profile> = match query {
                Some(q) => {
                    let q = q.to_lowercase();
                    list.iter().filter(|p| p.name.to_lowercase().contains(&q) || p.host.to_lowercase().contains(&q)).collect()
                }
                None => list.iter().collect(),
            };
            if filtered.is_empty() {
                println!("No profiles.");
                return Ok(());
            }
            let creds = db::credential::list(conn)?;
            println!("{:<20} {:<25} {:<6} {:<15}", "NAME", "HOST", "PORT", "USER");
            println!("{}", "-".repeat(70));
            for p in &filtered {
                let user = p.credential_id.as_deref()
                    .and_then(|id| creds.iter().find(|c| c.id == id))
                    .map(|c| c.username.as_str())
                    .unwrap_or("-");
                println!("{:<20} {:<25} {:<6} {:<15}", p.name, p.host, p.port, user);
            }
        }
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// open
// ═══════════════════════════════════════════════════════════════════

fn in_rssh_app() -> bool {
    std::env::var("RSSH_APP").is_ok()
}

fn osc_open(kind: &str, name: &str) {
    // OSC 7337 ; <kind>:<name> ST
    print!("\x1b]7337;{}:{}\x07", kind, name);
}

fn cmd_open(conn: &Connection, target: &str, name: Option<&str>) -> AppResult<()> {
    if target == "fwd" {
        let fname = name.ok_or_else(|| rssh_lib::error::AppError::Config("Usage: rssh open fwd <name>".into()))?;
        if in_rssh_app() {
            osc_open("fwd", fname);
            return Ok(());
        }
        return cmd_open_fwd(conn, fname);
    }
    if in_rssh_app() {
        osc_open("open", target);
        return Ok(());
    }
    cmd_open_ssh(conn, target)
}

fn cmd_open_ssh(conn: &Connection, name: &str) -> AppResult<()> {
    let profiles = db::profile::list(conn)?;
    let profile = profiles.iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| rssh_lib::error::AppError::NotFound(format!("Profile '{}' not found", name)))?;

    let cred = profile.credential_id.as_deref()
        .filter(|id| !id.is_empty())
        .and_then(|id| db::credential::get(conn, id).ok());

    let bastion = profile.bastion_profile_id.as_deref()
        .and_then(|bid| db::profile::get(conn, bid).ok())
        .and_then(|bp| {
            let bc = bp.credential_id.as_deref()
                .filter(|id| !id.is_empty())
                .and_then(|id| db::credential::get(conn, id).ok());
            Some((bp, bc))
        });

    let mut cmd = Command::new("ssh");

    // Key temp files — kept alive until ssh exits
    let mut _key_files: Vec<tempfile::NamedTempFile> = Vec::new();

    if let Some((ref bp, ref bc)) = bastion {
        let mut jump = String::new();
        if let Some(ref c) = bc {
            jump.push_str(&c.username);
            jump.push('@');

            if c.credential_type == CredentialType::Key {
                if let Some(ref secret) = c.secret {
                    let f = write_temp_key(secret)?;
                    cmd.arg("-o").arg(format!("IdentityFile={}", f.path().display()));
                    _key_files.push(f);
                }
            }
        }
        jump.push_str(&bp.host);
        if bp.port != 22 {
            jump = format!("{}:{}", jump, bp.port);
        }
        cmd.arg("-J").arg(&jump);
    }

    if let Some(ref cred) = cred {
        cmd.arg("-l").arg(&cred.username);
        if cred.credential_type == CredentialType::Key {
            if let Some(ref secret) = cred.secret {
                let f = write_temp_key(secret)?;
                cmd.arg("-i").arg(f.path());
                _key_files.push(f);
            }
        }
    }

    if profile.port != 22 {
        cmd.arg("-p").arg(profile.port.to_string());
    }

    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");

    // init_command: run it then hand off to shell
    if let Some(ref init) = profile.init_command {
        if !init.is_empty() {
            cmd.arg("-t").arg(&profile.host).arg(format!("{}; exec $SHELL -l", init));
        } else {
            cmd.arg(&profile.host);
        }
    } else {
        cmd.arg(&profile.host);
    }

    let status = cmd.status().map_err(|e| rssh_lib::error::AppError::Ssh(format!("Failed to run ssh: {e}")))?;
    std::process::exit(status.code().unwrap_or(1));
}

fn cmd_open_fwd(conn: &Connection, name: &str) -> AppResult<()> {
    let forwards = db::forward::list(conn)?;
    let fwd = forwards.iter()
        .find(|f| f.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| rssh_lib::error::AppError::NotFound(format!("Forward '{}' not found", name)))?;

    let profile = db::profile::get(conn, &fwd.profile_id)?;
    let cred = profile.credential_id.as_deref()
        .filter(|id| !id.is_empty())
        .and_then(|id| db::credential::get(conn, id).ok());

    let mut cmd = Command::new("ssh");
    cmd.arg("-N");

    let fwd_arg = match fwd.forward_type {
        ForwardType::Local => format!("{}:{}:{}", fwd.local_port, fwd.remote_host, fwd.remote_port),
        ForwardType::Remote => format!("{}:{}:{}", fwd.remote_port, fwd.remote_host, fwd.local_port),
    };
    cmd.arg(if fwd.forward_type == ForwardType::Local { "-L" } else { "-R" }).arg(&fwd_arg);

    let mut _key_files: Vec<tempfile::NamedTempFile> = Vec::new();

    if let Some(ref cred) = cred {
        cmd.arg("-l").arg(&cred.username);
        if cred.credential_type == CredentialType::Key {
            if let Some(ref secret) = cred.secret {
                let f = write_temp_key(secret)?;
                cmd.arg("-i").arg(f.path());
                _key_files.push(f);
            }
        }
    }

    if profile.port != 22 {
        cmd.arg("-p").arg(profile.port.to_string());
    }

    cmd.arg(&profile.host);

    println!("Forwarding {} {} ...", if fwd.forward_type == ForwardType::Local { "-L" } else { "-R" }, fwd_arg);
    let status = cmd.status().map_err(|e| rssh_lib::error::AppError::Ssh(format!("{e}")))?;
    std::process::exit(status.code().unwrap_or(1));
}

fn write_temp_key(pem: &str) -> AppResult<tempfile::NamedTempFile> {
    use std::os::unix::fs::PermissionsExt;
    let mut f = tempfile::NamedTempFile::new()
        .map_err(|e| rssh_lib::error::AppError::Io(e))?;
    f.write_all(pem.as_bytes())
        .map_err(|e| rssh_lib::error::AppError::Io(e))?;
    if !pem.ends_with('\n') {
        f.write_all(b"\n")
            .map_err(|e| rssh_lib::error::AppError::Io(e))?;
    }
    f.flush()
        .map_err(|e| rssh_lib::error::AppError::Io(e))?;
    f.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|e| rssh_lib::error::AppError::Io(e))?;
    Ok(f)
}

// ═══════════════════════════════════════════════════════════════════
// add
// ═══════════════════════════════════════════════════════════════════

fn cmd_add(conn: &Connection, kind: &str) -> AppResult<()> {
    match kind {
        "profile" => add_profile(conn),
        "cred" | "creds" => add_credential(conn),
        "fwd" => add_forward(conn),
        _ => {
            eprintln!("Unknown kind: {kind}. Use: profile, cred, fwd");
            Ok(())
        }
    }
}

fn add_profile(conn: &Connection) -> AppResult<()> {
    let name = prompt("Name: ");
    let host = prompt("Host: ");
    let port: u16 = prompt_default("Port", "22").parse().unwrap_or(22);

    let creds = db::credential::list(conn)?;
    let credential_id = if creds.is_empty() {
        println!("(no credentials, use 'rssh add cred' first)");
        None
    } else {
        println!("Credentials:");
        for (i, c) in creds.iter().enumerate() {
            println!("  {} - {} ({})", i + 1, c.name, c.username);
        }
        let choice = prompt_default("Credential #", "0");
        choice.parse::<usize>().ok()
            .and_then(|n| creds.get(n.wrapping_sub(1)))
            .map(|c| c.id.clone())
    };

    let profiles = db::profile::list(conn)?;
    let bastion_profile_id = if profiles.is_empty() {
        None
    } else {
        println!("Bastion (optional):");
        println!("  0 - none");
        for (i, p) in profiles.iter().enumerate() {
            println!("  {} - {} ({})", i + 1, p.name, p.host);
        }
        let choice = prompt_default("Bastion #", "0");
        choice.parse::<usize>().ok()
            .and_then(|n| profiles.get(n.wrapping_sub(1)))
            .map(|p| p.id.clone())
    };

    let init_command = prompt_optional("Init command (optional): ");

    let p = Profile {
        id: uuid::Uuid::new_v4().to_string(),
        name, host, port,
        credential_id,
        bastion_profile_id,
        init_command,
    };
    db::profile::insert(conn, &p)?;
    println!("Profile '{}' created.", p.name);
    Ok(())
}

fn add_credential(conn: &Connection) -> AppResult<()> {
    let name = prompt("Name: ");
    let username = prompt("Username: ");

    println!("Auth type:");
    println!("  1 - password");
    println!("  2 - key (PEM)");
    println!("  3 - none");
    let choice = prompt_default("Type #", "1");
    let (credential_type, secret) = match choice.as_str() {
        "2" => {
            println!("Paste private key (end with empty line):");
            let key = read_multiline();
            (CredentialType::Key, Some(key))
        }
        "3" => (CredentialType::None, None),
        _ => {
            let pw = read_password("Password: ");
            (CredentialType::Password, Some(pw))
        }
    };

    let save_to_remote = confirm("Sync secret to GitHub?", false);

    let c = Credential {
        id: uuid::Uuid::new_v4().to_string(),
        name, username, credential_type, secret, save_to_remote,
    };
    db::credential::insert(conn, &c)?;
    println!("Credential '{}' created.", c.name);
    Ok(())
}

fn add_forward(conn: &Connection) -> AppResult<()> {
    let name = prompt("Name: ");

    println!("Type:");
    println!("  1 - local (-L)");
    println!("  2 - remote (-R)");
    let ft = if prompt_default("Type #", "1") == "2" { ForwardType::Remote } else { ForwardType::Local };

    let local_port: u16 = prompt("Local port: ").parse().unwrap_or(0);
    let remote_host = prompt_default("Remote host", "127.0.0.1");
    let remote_port: u16 = prompt("Remote port: ").parse().unwrap_or(0);

    let profiles = db::profile::list(conn)?;
    if profiles.is_empty() {
        eprintln!("No profiles. Create one first with 'rssh add profile'.");
        return Ok(());
    }
    println!("Profile:");
    for (i, p) in profiles.iter().enumerate() {
        println!("  {} - {} ({})", i + 1, p.name, p.host);
    }
    let pidx = prompt("Profile #: ").parse::<usize>().unwrap_or(0);
    let profile_id = profiles.get(pidx.wrapping_sub(1))
        .map(|p| p.id.clone())
        .unwrap_or_default();

    let f = Forward {
        id: uuid::Uuid::new_v4().to_string(),
        name, forward_type: ft, local_port, remote_host, remote_port, profile_id,
    };
    db::forward::insert(conn, &f)?;
    println!("Forward '{}' created.", f.name);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// edit
// ═══════════════════════════════════════════════════════════════════

fn cmd_edit(conn: &Connection, kind: &str, name: &str) -> AppResult<()> {
    match kind {
        "profile" => edit_profile(conn, name),
        "cred" | "creds" => edit_credential(conn, name),
        "fwd" => edit_forward(conn, name),
        _ => { eprintln!("Unknown kind: {kind}"); Ok(()) }
    }
}

fn edit_profile(conn: &Connection, name: &str) -> AppResult<()> {
    let profiles = db::profile::list(conn)?;
    let p = profiles.iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| rssh_lib::error::AppError::NotFound(format!("Profile '{name}' not found")))?;

    let mut updated = p.clone();
    updated.name = prompt_default("Name", &p.name);
    updated.host = prompt_default("Host", &p.host);
    updated.port = prompt_default("Port", &p.port.to_string()).parse().unwrap_or(p.port);

    let creds = db::credential::list(conn)?;
    if !creds.is_empty() {
        let cur = p.credential_id.as_deref()
            .and_then(|id| creds.iter().position(|c| c.id == id))
            .map(|i| (i + 1).to_string())
            .unwrap_or("0".into());
        println!("Credentials:");
        println!("  0 - none");
        for (i, c) in creds.iter().enumerate() {
            println!("  {} - {} ({})", i + 1, c.name, c.username);
        }
        let choice = prompt_default("Credential #", &cur);
        updated.credential_id = choice.parse::<usize>().ok()
            .and_then(|n| creds.get(n.wrapping_sub(1)))
            .map(|c| c.id.clone());
    }

    updated.init_command = {
        let cur = p.init_command.as_deref().unwrap_or("");
        let v = prompt_default("Init command", cur);
        if v.is_empty() { None } else { Some(v) }
    };

    db::profile::update(conn, &updated)?;
    println!("Profile '{}' updated.", updated.name);
    Ok(())
}

fn edit_credential(conn: &Connection, name: &str) -> AppResult<()> {
    let creds = db::credential::list(conn)?;
    let c = creds.iter()
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| rssh_lib::error::AppError::NotFound(format!("Credential '{name}' not found")))?;

    let mut updated = c.clone();
    updated.name = prompt_default("Name", &c.name);
    updated.username = prompt_default("Username", &c.username);

    println!("Auth type (current: {}):", c.credential_type.as_str());
    println!("  1 - password  2 - key  3 - none  Enter - keep");
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
            updated.credential_type = CredentialType::None;
            updated.secret = None;
        }
        _ => {} // keep current
    }

    updated.save_to_remote = confirm("Sync secret to GitHub?", c.save_to_remote);

    db::credential::update(conn, &updated)?;
    println!("Credential '{}' updated.", updated.name);
    Ok(())
}

fn edit_forward(conn: &Connection, name: &str) -> AppResult<()> {
    let forwards = db::forward::list(conn)?;
    let f = forwards.iter()
        .find(|f| f.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| rssh_lib::error::AppError::NotFound(format!("Forward '{name}' not found")))?;

    let mut updated = f.clone();
    updated.name = prompt_default("Name", &f.name);
    updated.local_port = prompt_default("Local port", &f.local_port.to_string()).parse().unwrap_or(f.local_port);
    updated.remote_host = prompt_default("Remote host", &f.remote_host);
    updated.remote_port = prompt_default("Remote port", &f.remote_port.to_string()).parse().unwrap_or(f.remote_port);

    db::forward::update(conn, &updated)?;
    println!("Forward '{}' updated.", updated.name);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// rm
// ═══════════════════════════════════════════════════════════════════

fn cmd_rm(conn: &Connection, kind: &str, name: &str) -> AppResult<()> {
    match kind {
        "profile" => {
            let id = find_profile_id(conn, name)?;
            if !confirm(&format!("Delete profile '{name}'?"), false) { return Ok(()); }
            db::profile::delete(conn, &id)?;
            println!("Deleted.");
        }
        "cred" | "creds" => {
            let id = find_credential_id(conn, name)?;
            if !confirm(&format!("Delete credential '{name}'?"), false) { return Ok(()); }
            db::credential::delete(conn, &id)?;
            println!("Deleted.");
        }
        "fwd" => {
            let id = find_forward_id(conn, name)?;
            if !confirm(&format!("Delete forward '{name}'?"), false) { return Ok(()); }
            db::forward::delete(conn, &id)?;
            println!("Deleted.");
        }
        _ => eprintln!("Unknown kind: {kind}"),
    }
    Ok(())
}

fn find_profile_id(conn: &Connection, name: &str) -> AppResult<String> {
    db::profile::list(conn)?
        .iter().find(|p| p.name.eq_ignore_ascii_case(name))
        .map(|p| p.id.clone())
        .ok_or_else(|| rssh_lib::error::AppError::NotFound(format!("Profile '{name}' not found")))
}

fn find_credential_id(conn: &Connection, name: &str) -> AppResult<String> {
    db::credential::list(conn)?
        .iter().find(|c| c.name.eq_ignore_ascii_case(name))
        .map(|c| c.id.clone())
        .ok_or_else(|| rssh_lib::error::AppError::NotFound(format!("Credential '{name}' not found")))
}

fn find_forward_id(conn: &Connection, name: &str) -> AppResult<String> {
    db::forward::list(conn)?
        .iter().find(|f| f.name.eq_ignore_ascii_case(name))
        .map(|f| f.id.clone())
        .ok_or_else(|| rssh_lib::error::AppError::NotFound(format!("Forward '{name}' not found")))
}

// ═══════════════════════════════════════════════════════════════════
// config
// ═══════════════════════════════════════════════════════════════════

fn cmd_config(conn: &Connection, action: ConfigCmd) -> AppResult<()> {
    match action {
        ConfigCmd::Export { file } => config_export(conn, &file),
        ConfigCmd::Import { file } => config_import(conn, &file),
        ConfigCmd::Set => config_set(conn),
        ConfigCmd::Push => config_push(conn),
        ConfigCmd::Pull => config_pull(conn),
    }
}

fn build_config_json(conn: &Connection) -> AppResult<String> {
    let profiles = db::profile::list(conn)?;
    let credentials = db::credential::list(conn)?;
    let forwards = db::forward::list(conn)?;
    serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "profiles": profiles,
        "credentials": credentials,
        "forwards": forwards,
    })).map_err(|e| rssh_lib::error::AppError::Other(e.to_string()))
}

fn import_config_json(conn: &Connection, json: &str) -> AppResult<()> {
    let data: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| rssh_lib::error::AppError::Config(format!("JSON parse error: {e}")))?;

    db::credential::clear_all(conn)?;
    db::profile::clear_all(conn)?;
    db::forward::clear_all(conn)?;

    if let Some(arr) = data["credentials"].as_array() {
        for item in arr {
            if let Ok(c) = serde_json::from_value::<Credential>(item.clone()) {
                let _ = db::credential::insert(conn, &c);
            }
        }
    }
    if let Some(arr) = data["profiles"].as_array() {
        for item in arr {
            if let Ok(p) = serde_json::from_value::<Profile>(item.clone()) {
                let _ = db::profile::insert(conn, &p);
            }
        }
    }
    if let Some(arr) = data["forwards"].as_array() {
        for item in arr {
            if let Ok(f) = serde_json::from_value::<Forward>(item.clone()) {
                let _ = db::forward::insert(conn, &f);
            }
        }
    }
    Ok(())
}

fn config_export(conn: &Connection, file: &str) -> AppResult<()> {
    let json = build_config_json(conn)?;
    let pw = read_password("Encryption password: ");
    let pw2 = read_password("Confirm password: ");
    if pw != pw2 {
        eprintln!("Passwords do not match.");
        std::process::exit(1);
    }
    let encrypted = rssh_lib::crypto::encrypt(&json, &pw)?;
    std::fs::write(file, &encrypted)?;
    println!("Exported to {file}");
    Ok(())
}

fn config_import(conn: &Connection, file: &str) -> AppResult<()> {
    let encrypted = std::fs::read_to_string(file)?;
    let pw = read_password("Decryption password: ");
    let json = rssh_lib::crypto::decrypt(&encrypted, &pw)?;
    import_config_json(conn, &json)?;
    println!("Imported from {file}");
    Ok(())
}

fn config_set(conn: &Connection) -> AppResult<()> {
    let cur_token = db::settings::get(conn, "github_token")?.unwrap_or_default();
    let cur_repo = db::settings::get(conn, "github_repo")?.unwrap_or_default();
    let cur_branch = db::settings::get(conn, "github_branch")?.unwrap_or("main".into());

    let token = prompt_default("GitHub PAT", if cur_token.is_empty() { "ghp_..." } else { &cur_token });
    let repo = prompt_default("Repo (owner/repo)", &cur_repo);
    let branch = prompt_default("Branch", &cur_branch);

    db::settings::set(conn, "github_token", &token)?;
    db::settings::set(conn, "github_repo", &repo)?;
    db::settings::set(conn, "github_branch", &branch)?;
    println!("GitHub settings saved.");
    Ok(())
}

fn config_push(conn: &Connection) -> AppResult<()> {
    let token = db::settings::get(conn, "github_token")?.ok_or_else(|| rssh_lib::error::AppError::Config("GitHub token not set. Run: rssh config set".into()))?;
    let repo = db::settings::get(conn, "github_repo")?.ok_or_else(|| rssh_lib::error::AppError::Config("GitHub repo not set".into()))?;
    let branch = db::settings::get(conn, "github_branch")?.unwrap_or("main".into());

    let mut json_data = {
        let profiles = db::profile::list(conn)?;
        let mut credentials = db::credential::list(conn)?;
        let forwards = db::forward::list(conn)?;
        for c in credentials.iter_mut() {
            if !c.save_to_remote { c.secret = None; }
        }
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 1, "exported_at": chrono::Utc::now().to_rfc3339(),
            "profiles": profiles, "credentials": credentials, "forwards": forwards,
        })).map_err(|e| rssh_lib::error::AppError::Other(e.to_string()))?
    };

    let pw = read_password("Encryption password: ");
    let encrypted = rssh_lib::crypto::encrypt(&json_data, &pw)?;
    json_data.clear();

    let sync = rssh_lib::sync::github::GitHubSync::from_settings(&token, &repo, &branch)?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()
        .map_err(|e| rssh_lib::error::AppError::Other(e.to_string()))?;
    rt.block_on(sync.push(&encrypted))?;
    println!("Pushed to GitHub.");
    Ok(())
}

fn config_pull(conn: &Connection) -> AppResult<()> {
    let token = db::settings::get(conn, "github_token")?.ok_or_else(|| rssh_lib::error::AppError::Config("GitHub token not set. Run: rssh config set".into()))?;
    let repo = db::settings::get(conn, "github_repo")?.ok_or_else(|| rssh_lib::error::AppError::Config("GitHub repo not set".into()))?;
    let branch = db::settings::get(conn, "github_branch")?.unwrap_or("main".into());

    let sync = rssh_lib::sync::github::GitHubSync::from_settings(&token, &repo, &branch)?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()
        .map_err(|e| rssh_lib::error::AppError::Other(e.to_string()))?;
    let encrypted = rt.block_on(sync.pull())?;

    let pw = read_password("Decryption password: ");
    let json = rssh_lib::crypto::decrypt(&encrypted, &pw)?;
    import_config_json(conn, &json)?;
    println!("Pulled from GitHub.");
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// _names (for tab completion)
// ═══════════════════════════════════════════════════════════════════

fn cmd_names(conn: &Connection, kind: &str) -> AppResult<()> {
    match kind {
        "profiles" | "profile" => {
            for p in db::profile::list(conn)? { println!("{}", p.name); }
        }
        "cred" | "creds" => {
            for c in db::credential::list(conn)? { println!("{}", c.name); }
        }
        "fwd" => {
            for f in db::forward::list(conn)? { println!("{}", f.name); }
        }
        _ => {}
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
// Shell completions
// ═══════════════════════════════════════════════════════════════════

fn print_completions(shell: &str) {
    match shell {
        "zsh" => print!("{}", ZSH_COMPLETIONS),
        "bash" => print!("{}", BASH_COMPLETIONS),
        "powershell" | "pwsh" => print!("{}", POWERSHELL_COMPLETIONS),
        "fish" => print!("{}", FISH_COMPLETIONS),
        _ => eprintln!("Supported shells: zsh, bash, powershell, fish"),
    }
}

const ZSH_COMPLETIONS: &str = r#"#compdef rssh

_rssh() {
    local -a commands
    commands=(
        'ls:List profiles, credentials, or forwards'
        'open:Connect via SSH or start port forward'
        'add:Add profile, credential, or forward'
        'edit:Edit profile, credential, or forward'
        'rm:Delete profile, credential, or forward'
        'config:Configuration management'
        'completions:Generate shell completions'
    )

    _arguments -C \
        '1:command:->command' \
        '*::arg:->args'

    case $state in
        command)
            _describe 'command' commands
            ;;
        args)
            case $words[1] in
                ls)
                    local -a ls_opts=('cred:List credentials' 'fwd:List forwards')
                    _describe 'type' ls_opts
                    ;;
                open)
                    if [[ $CURRENT -eq 2 ]]; then
                        compadd fwd $(rssh _names profiles 2>/dev/null)
                    elif [[ $words[2] == "fwd" && $CURRENT -eq 3 ]]; then
                        compadd $(rssh _names fwd 2>/dev/null)
                    fi
                    ;;
                add)
                    compadd profile cred fwd
                    ;;
                edit|rm)
                    if [[ $CURRENT -eq 2 ]]; then
                        compadd profile cred fwd
                    elif [[ $CURRENT -eq 3 ]]; then
                        case $words[2] in
                            profile) compadd $(rssh _names profiles 2>/dev/null) ;;
                            cred)    compadd $(rssh _names creds 2>/dev/null) ;;
                            fwd)     compadd $(rssh _names fwd 2>/dev/null) ;;
                        esac
                    fi
                    ;;
                config)
                    if [[ $CURRENT -eq 2 ]]; then
                        local -a cfg_cmds=('export:Export encrypted backup' 'import:Import backup' 'set:Set GitHub settings' 'push:Push to GitHub' 'pull:Pull from GitHub')
                        _describe 'action' cfg_cmds
                    elif [[ $CURRENT -eq 3 && ($words[2] == "export" || $words[2] == "import") ]]; then
                        _files
                    fi
                    ;;
                completions)
                    compadd zsh bash powershell fish
                    ;;
            esac
            ;;
    esac
}

_rssh "$@"
"#;

const BASH_COMPLETIONS: &str = r#"_rssh() {
    local cur prev words cword
    _init_completion || return

    if [[ $cword -eq 1 ]]; then
        COMPREPLY=($(compgen -W "ls open add edit rm config completions" -- "$cur"))
        return
    fi

    case ${words[1]} in
        ls)
            COMPREPLY=($(compgen -W "cred fwd" -- "$cur"))
            ;;
        open)
            if [[ $cword -eq 2 ]]; then
                local profiles=$(rssh _names profiles 2>/dev/null)
                COMPREPLY=($(compgen -W "fwd $profiles" -- "$cur"))
            elif [[ ${words[2]} == "fwd" && $cword -eq 3 ]]; then
                local fwds=$(rssh _names fwd 2>/dev/null)
                COMPREPLY=($(compgen -W "$fwds" -- "$cur"))
            fi
            ;;
        add)
            COMPREPLY=($(compgen -W "profile cred fwd" -- "$cur"))
            ;;
        edit|rm)
            if [[ $cword -eq 2 ]]; then
                COMPREPLY=($(compgen -W "profile cred fwd" -- "$cur"))
            elif [[ $cword -eq 3 ]]; then
                case ${words[2]} in
                    profile) COMPREPLY=($(compgen -W "$(rssh _names profiles 2>/dev/null)" -- "$cur")) ;;
                    cred)    COMPREPLY=($(compgen -W "$(rssh _names creds 2>/dev/null)" -- "$cur")) ;;
                    fwd)     COMPREPLY=($(compgen -W "$(rssh _names fwd 2>/dev/null)" -- "$cur")) ;;
                esac
            fi
            ;;
        config)
            if [[ $cword -eq 2 ]]; then
                COMPREPLY=($(compgen -W "export import set push pull" -- "$cur"))
            elif [[ $cword -eq 3 && (${words[2]} == "export" || ${words[2]} == "import") ]]; then
                _filedir
            fi
            ;;
        completions)
            COMPREPLY=($(compgen -W "zsh bash powershell fish" -- "$cur"))
            ;;
    esac
}

complete -F _rssh rssh
"#;

const POWERSHELL_COMPLETIONS: &str = r#"Register-ArgumentCompleter -Native -CommandName rssh -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $words = $commandAst.ToString().Split(' ')
    $cmd = if ($words.Length -gt 1) { $words[1] } else { '' }
    $pos = $words.Length

    if ($pos -le 1 -or ($pos -eq 2 -and $wordToComplete)) {
        @('ls','open','add','edit','rm','config','completions') | Where-Object { $_ -like "$wordToComplete*" } | ForEach-Object {
            [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
        }
        return
    }

    switch ($cmd) {
        'ls' { @('cred','fwd') | Where-Object { $_ -like "$wordToComplete*" } }
        'open' {
            if ($pos -eq 2 -or ($pos -eq 3 -and $wordToComplete -and $words[2] -ne 'fwd')) {
                $names = @('fwd') + @(rssh _names profiles 2>$null)
                $names | Where-Object { $_ -like "$wordToComplete*" }
            } elseif ($words[2] -eq 'fwd') {
                rssh _names fwd 2>$null | Where-Object { $_ -like "$wordToComplete*" }
            }
        }
        'add' { @('profile','cred','fwd') | Where-Object { $_ -like "$wordToComplete*" } }
        { $_ -in 'edit','rm' } {
            if ($pos -eq 2 -or ($pos -eq 3 -and $wordToComplete -and $words[2] -notin @('profile','cred','fwd'))) {
                @('profile','cred','fwd') | Where-Object { $_ -like "$wordToComplete*" }
            } elseif ($pos -ge 3) {
                $kind = $words[2]
                $n = switch ($kind) { 'profile' { 'profiles' } 'cred' { 'creds' } default { $kind } }
                rssh _names $n 2>$null | Where-Object { $_ -like "$wordToComplete*" }
            }
        }
        'config' { @('export','import','set','push','pull') | Where-Object { $_ -like "$wordToComplete*" } }
        'completions' { @('zsh','bash','powershell','fish') | Where-Object { $_ -like "$wordToComplete*" } }
    } | ForEach-Object {
        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)
    }
}
"#;

const FISH_COMPLETIONS: &str = r#"# rssh fish completions
complete -c rssh -n '__fish_use_subcommand' -a 'ls' -d 'List profiles/credentials/forwards'
complete -c rssh -n '__fish_use_subcommand' -a 'open' -d 'Connect via SSH'
complete -c rssh -n '__fish_use_subcommand' -a 'add' -d 'Add profile/credential/forward'
complete -c rssh -n '__fish_use_subcommand' -a 'edit' -d 'Edit profile/credential/forward'
complete -c rssh -n '__fish_use_subcommand' -a 'rm' -d 'Delete profile/credential/forward'
complete -c rssh -n '__fish_use_subcommand' -a 'config' -d 'Configuration management'
complete -c rssh -n '__fish_use_subcommand' -a 'completions' -d 'Generate shell completions'

complete -c rssh -n '__fish_seen_subcommand_from ls' -a 'cred fwd'
complete -c rssh -n '__fish_seen_subcommand_from open' -a '(rssh _names profiles 2>/dev/null)' -a 'fwd'
complete -c rssh -n '__fish_seen_subcommand_from add' -a 'profile cred fwd'
complete -c rssh -n '__fish_seen_subcommand_from edit' -a 'profile cred fwd'
complete -c rssh -n '__fish_seen_subcommand_from rm' -a 'profile cred fwd'
complete -c rssh -n '__fish_seen_subcommand_from config' -a 'export import set push pull'
complete -c rssh -n '__fish_seen_subcommand_from completions' -a 'zsh bash powershell fish'
"#;

// ═══════════════════════════════════════════════════════════════════
// IO helpers
// ═══════════════════════════════════════════════════════════════════

fn prompt(label: &str) -> String {
    eprint!("{}", label);
    io::stderr().flush().unwrap();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).unwrap();
    buf.trim().to_string()
}

fn prompt_default(label: &str, default: &str) -> String {
    eprint!("{} [{}]: ", label, default);
    io::stderr().flush().unwrap();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).unwrap();
    let val = buf.trim();
    if val.is_empty() { default.to_string() } else { val.to_string() }
}

fn prompt_optional(label: &str) -> Option<String> {
    let val = prompt(label);
    if val.is_empty() { None } else { Some(val) }
}

fn read_password(label: &str) -> String {
    eprint!("{}", label);
    io::stderr().flush().unwrap();
    rpassword::read_password().unwrap_or_default()
}

fn read_multiline() -> String {
    let mut lines = Vec::new();
    loop {
        let mut buf = String::new();
        io::stdin().read_line(&mut buf).unwrap();
        if buf.trim().is_empty() { break; }
        lines.push(buf);
    }
    lines.concat().trim_end().to_string()
}

fn confirm(label: &str, default: bool) -> bool {
    let hint = if default { "Y/n" } else { "y/N" };
    eprint!("{} [{}]: ", label, hint);
    io::stderr().flush().unwrap();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).unwrap();
    let val = buf.trim().to_lowercase();
    if val.is_empty() { default } else { val == "y" || val == "yes" }
}
