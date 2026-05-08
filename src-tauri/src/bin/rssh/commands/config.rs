//! `rssh config <export|import|set|push|pull>` —— 配置备份与 GitHub 同步。
//!
//! `import` 走增量合并（merge_import），`pull` 走全量替换 + 事务（replace_import），
//! 共用 `rssh_lib::sync::config` —— GUI 同一份逻辑。

use clap::Subcommand;
use rssh_lib::error::AppResult;
use rssh_lib::secret::{cred_secret_key, setting_key, SecretStore};

use crate::ctx::CliCtx;
use crate::helpers::{die, prompt_default, read_password};

#[derive(Subcommand)]
pub enum ConfigCmd {
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

#[derive(Clone, Copy)]
enum ImportMode {
    Merge,
    Replace,
}

pub fn cmd_config(conn: &CliCtx, action: ConfigCmd) -> AppResult<()> {
    match action {
        ConfigCmd::Export { file } => config_export(conn, &file),
        ConfigCmd::Import { file } => config_import(conn, &file),
        ConfigCmd::Set => config_set(conn),
        ConfigCmd::Push => config_push(conn),
        ConfigCmd::Pull => config_pull(conn),
    }
}

fn build_config_json(conn: &CliCtx) -> AppResult<String> {
    let profiles = rssh_lib::db::profile::list(conn)?;
    let mut credentials = rssh_lib::db::credential::list(conn)?;
    for c in credentials.iter_mut() {
        c.secret = conn.secret_store().get(&cred_secret_key(&c.id))?;
    }
    let forwards = rssh_lib::db::forward::list(conn)?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "profiles": profiles,
        "credentials": credentials,
        "forwards": forwards,
    }))
    .unwrap_or_else(|e| die(format!("Serialization failed: {e}"))))
}

/// 解析 JSON 后委派给共享同步逻辑。CLI 与 GUI 共用 sync::config 这一份。
fn import_config_json(conn: &CliCtx, json: &str, mode: ImportMode) -> AppResult<()> {
    let data: serde_json::Value = serde_json::from_str(json)
        .unwrap_or_else(|e| die(format!("JSON parse error: {e}")));
    let ss: &dyn SecretStore = conn.secret_store().as_ref();
    match mode {
        ImportMode::Merge => rssh_lib::sync::config::merge_import(conn, ss, &data),
        ImportMode::Replace => rssh_lib::sync::config::replace_import(conn, ss, &data),
    }
}

fn config_export(conn: &CliCtx, file: &str) -> AppResult<()> {
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

fn config_import(conn: &CliCtx, file: &str) -> AppResult<()> {
    let encrypted = std::fs::read_to_string(file)?;
    let pw = read_password("Decryption password: ");
    let json = rssh_lib::crypto::decrypt(&encrypted, &pw)?;
    // 文件 import：增量合并，本地数据保留；同 id 实体被覆盖。
    import_config_json(conn, &json, ImportMode::Merge)?;
    println!("Imported from {file}");
    Ok(())
}

fn config_set(conn: &CliCtx) -> AppResult<()> {
    let cur_token = conn
        .secret_store()
        .get(&setting_key("github_token"))?
        .unwrap_or_default();
    let cur_repo = rssh_lib::db::settings::get(conn, "github_repo")?.unwrap_or_default();
    let cur_branch = rssh_lib::db::settings::get(conn, "github_branch")?.unwrap_or("main".into());

    let token = prompt_default(
        "GitHub PAT",
        if cur_token.is_empty() {
            "ghp_..."
        } else {
            &cur_token
        },
    );
    let repo = prompt_default("Repo (owner/repo)", &cur_repo);
    let branch = prompt_default("Branch", &cur_branch);

    if token.is_empty() {
        conn.secret_store().delete(&setting_key("github_token"))?;
    } else {
        conn.secret_store()
            .set(&setting_key("github_token"), &token)?;
    }
    rssh_lib::db::settings::set(conn, "github_repo", &repo)?;
    rssh_lib::db::settings::set(conn, "github_branch", &branch)?;
    println!(
        "GitHub settings saved (token in {}).",
        conn.secret_store().backend_name()
    );
    Ok(())
}

fn config_push(conn: &CliCtx) -> AppResult<()> {
    let token = conn
        .secret_store()
        .get(&setting_key("github_token"))?
        .unwrap_or_else(|| die("GitHub token not set. Run: rssh config set"));
    let repo = rssh_lib::db::settings::get(conn, "github_repo")?
        .unwrap_or_else(|| die("GitHub repo not set"));
    let branch = rssh_lib::db::settings::get(conn, "github_branch")?.unwrap_or("main".into());

    let mut json_data = {
        let profiles = rssh_lib::db::profile::list(conn)?;
        let mut credentials = rssh_lib::db::credential::list(conn)?;
        for c in credentials.iter_mut() {
            c.secret = conn.secret_store().get(&cred_secret_key(&c.id))?;
            if !c.save_to_remote {
                c.secret = None;
            }
        }
        let forwards = rssh_lib::db::forward::list(conn)?;
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 1, "exported_at": chrono::Utc::now().to_rfc3339(),
            "profiles": profiles, "credentials": credentials, "forwards": forwards,
        }))
        .unwrap_or_else(|e| die(format!("Serialization failed: {e}")))
    };

    let pw = read_password("Encryption password: ");
    let encrypted = rssh_lib::crypto::encrypt(&json_data, &pw)?;
    json_data.clear();

    let sync = rssh_lib::sync::github::GitHubSync::from_settings(&token, &repo, &branch)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| die(format!("Tokio runtime: {e}")));
    rt.block_on(sync.push(&encrypted))?;
    println!("Pushed to GitHub.");
    Ok(())
}

fn config_pull(conn: &CliCtx) -> AppResult<()> {
    let token = conn
        .secret_store()
        .get(&setting_key("github_token"))?
        .unwrap_or_else(|| die("GitHub token not set. Run: rssh config set"));
    let repo = rssh_lib::db::settings::get(conn, "github_repo")?
        .unwrap_or_else(|| die("GitHub repo not set"));
    let branch = rssh_lib::db::settings::get(conn, "github_branch")?.unwrap_or("main".into());

    let sync = rssh_lib::sync::github::GitHubSync::from_settings(&token, &repo, &branch)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| die(format!("Tokio runtime: {e}")));
    let encrypted = rt.block_on(sync.pull())?;

    let pw = read_password("Decryption password: ");
    let json = rssh_lib::crypto::decrypt(&encrypted, &pw)?;
    // pull：全量替换语义，clear+insert 包事务。
    import_config_json(conn, &json, ImportMode::Replace)?;
    println!("Pulled from GitHub.");
    Ok(())
}
