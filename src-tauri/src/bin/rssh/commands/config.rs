//! `rssh config <export|import|github|webdav>` —— config backup & remote sync.
//!
//! Both `import` and `pull` go through `merge_import` (additive upsert by
//! identity, never destructive), sharing `rssh_lib::sync::config` with the GUI.

use clap::Subcommand;
use rssh_lib::error::{AppError, AppResult};
use rssh_lib::secret::{setting_key, SecretStore};

use crate::ctx::CliCtx;
use crate::helpers::{die, prompt_default, prompt_secret_default, read_password};

#[derive(Subcommand)]
pub enum ConfigCmd {
    /// Export encrypted backup
    Export { file: String },
    /// Import from encrypted backup
    Import { file: String },
    /// GitHub remote sync
    Github {
        #[command(subcommand)]
        action: RemoteSyncCmd,
    },
    /// WebDAV remote sync
    Webdav {
        #[command(subcommand)]
        action: RemoteSyncCmd,
    },
}

#[derive(Subcommand)]
pub enum RemoteSyncCmd {
    /// Set remote sync settings
    Set,
    /// Push config to remote
    Push,
    /// Pull config from remote
    Pull,
}

pub fn cmd_config(conn: &CliCtx, action: ConfigCmd) -> AppResult<()> {
    match action {
        ConfigCmd::Export { file } => config_export(conn, &file),
        ConfigCmd::Import { file } => config_import(conn, &file),
        ConfigCmd::Github { action } => match action {
            RemoteSyncCmd::Set => config_github_set(conn),
            RemoteSyncCmd::Push => config_github_push(conn),
            RemoteSyncCmd::Pull => config_github_pull(conn),
        },
        ConfigCmd::Webdav { action } => match action {
            RemoteSyncCmd::Set => config_webdav_set(conn),
            RemoteSyncCmd::Push => config_webdav_push(conn),
            RemoteSyncCmd::Pull => config_webdav_pull(conn),
        },
    }
}

/// Parse JSON then delegate to the shared sync logic (same path as the GUI).
fn import_config_json(conn: &CliCtx, json: &str) -> AppResult<()> {
    let data: serde_json::Value =
        serde_json::from_str(json).unwrap_or_else(|e| die(format!("JSON parse error: {e}")));
    let ss: &dyn SecretStore = conn.secret_store().as_ref();
    rssh_lib::sync::config::merge_import(conn, ss, &conn.data_dir, &data)
}

fn config_export(conn: &CliCtx, file: &str) -> AppResult<()> {
    // 本地 export：全量备份（每个类别 + 所有 secret），不看开关 —— 跟 GUI 本地导出
    // 用同一个 build_payload，CLI ↔ GUI 形态永不漂移。
    let ss: &dyn SecretStore = conn.secret_store().as_ref();
    let payload = rssh_lib::sync::config::build_payload(
        conn,
        ss,
        &conn.data_dir,
        &rssh_lib::sync::config::ExportMode::LocalBackup,
    )?;
    let json = serde_json::to_string_pretty(&payload)
        .unwrap_or_else(|e| die(format!("Serialization failed: {e}")));
    let pw = read_password("Encryption password: ");
    let pw2 = read_password("Confirm password: ");
    if pw != pw2 {
        return Err(AppError::config(
            "cli_password_mismatch",
            serde_json::json!({}),
        ));
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
    // File import: additive merge — local data kept; same-id entities overwritten.
    import_config_json(conn, &json)?;
    println!("Imported from {file}");
    Ok(())
}

fn remote_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| die(format!("Tokio runtime: {e}")))
}

fn push_remote_backup(
    conn: &CliCtx,
    remote: &dyn rssh_lib::sync::remote::RemoteBackup,
) -> AppResult<()> {
    let secrets = conn.secret_store();
    let mut prepared =
        rssh_lib::sync::remote::prepare_backup(conn, secrets.as_ref(), &conn.data_dir)?;
    let password = read_password("Encryption password: ");
    let encrypted = rssh_lib::crypto::encrypt(&prepared.json, &password)?;
    prepared.json.clear();

    remote_runtime().block_on(rssh_lib::sync::remote::publish(
        remote,
        &encrypted,
        &prepared.metadata,
    ))?;
    Ok(())
}

fn pull_remote_backup(
    conn: &CliCtx,
    remote: &dyn rssh_lib::sync::remote::RemoteBackup,
) -> AppResult<()> {
    let fetched = remote_runtime().block_on(rssh_lib::sync::remote::fetch(remote))?;
    let password = read_password("Decryption password: ");
    let secrets = conn.secret_store();
    rssh_lib::sync::remote::apply_fetched_backup(
        conn,
        secrets.as_ref(),
        &conn.data_dir,
        fetched,
        &password,
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// GitHub sync
// ---------------------------------------------------------------------------

fn read_github_settings(conn: &CliCtx) -> AppResult<(String, String, String)> {
    let token = conn
        .secret_store()
        .get(&setting_key("github_token"))?
        .unwrap_or_else(|| die("GitHub token not set. Run: rssh config github set"));
    let repo = rssh_lib::db::settings::get(conn, "github_repo")?
        .unwrap_or_else(|| die("GitHub repo not set"));
    let branch = rssh_lib::db::settings::get(conn, "github_branch")?.unwrap_or("main".into());
    Ok((token, repo, branch))
}

fn config_github_set(conn: &CliCtx) -> AppResult<()> {
    let cur_token = conn
        .secret_store()
        .get(&setting_key("github_token"))?
        .unwrap_or_default();
    let cur_repo = rssh_lib::db::settings::get(conn, "github_repo")?.unwrap_or_default();
    let cur_branch = rssh_lib::db::settings::get(conn, "github_branch")?.unwrap_or("main".into());

    // PAT 是 secret —— 不能在 prompt 默认值里 echo 出来（屏幕录制 / 终端历史
    // 都会抓到）。走 prompt_secret_default：占位显示 `(stored)`，输入不回显。
    let token = prompt_secret_default("GitHub PAT", &cur_token);
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

fn config_github_push(conn: &CliCtx) -> AppResult<()> {
    let (token, repo, branch) = read_github_settings(conn)?;
    let sync = rssh_lib::sync::github::GitHubSync::from_settings(&token, &repo, &branch)?;
    push_remote_backup(conn, &sync)?;
    println!("Pushed to GitHub.");
    Ok(())
}

fn config_github_pull(conn: &CliCtx) -> AppResult<()> {
    let (token, repo, branch) = read_github_settings(conn)?;
    let sync = rssh_lib::sync::github::GitHubSync::from_settings(&token, &repo, &branch)?;
    pull_remote_backup(conn, &sync)?;
    println!("Pulled from GitHub.");
    Ok(())
}

// ---------------------------------------------------------------------------
// WebDAV sync
// ---------------------------------------------------------------------------

fn read_webdav_settings(conn: &CliCtx) -> AppResult<(String, String, String)> {
    let url = rssh_lib::db::settings::get(conn, "webdav_url")?
        .unwrap_or_else(|| die("WebDAV URL not set. Run: rssh config webdav set"));
    let username = rssh_lib::db::settings::get(conn, "webdav_username")?.unwrap_or_default();
    let password = conn
        .secret_store()
        .get(&setting_key("webdav_password"))?
        .unwrap_or_else(|| die("WebDAV password not set. Run: rssh config webdav set"));
    Ok((url, username, password))
}

fn config_webdav_set(conn: &CliCtx) -> AppResult<()> {
    let cur_url = rssh_lib::db::settings::get(conn, "webdav_url")?.unwrap_or_default();
    let cur_username = rssh_lib::db::settings::get(conn, "webdav_username")?.unwrap_or_default();
    let cur_password = conn
        .secret_store()
        .get(&setting_key("webdav_password"))?
        .unwrap_or_default();

    let url = prompt_default("WebDAV URL (https://...)", &cur_url);
    let username = prompt_default("Username", &cur_username);
    let password = prompt_secret_default("Password", &cur_password);

    // Validate early so the user doesn't have to run a network command to
    // discover a typo in the URL.
    rssh_lib::sync::webdav::WebDavSync::from_settings(&url, &username, &password)?;

    rssh_lib::db::settings::set(conn, "webdav_url", &url)?;
    rssh_lib::db::settings::set(conn, "webdav_username", &username)?;
    if password.is_empty() {
        conn.secret_store()
            .delete(&setting_key("webdav_password"))?;
    } else {
        conn.secret_store()
            .set(&setting_key("webdav_password"), &password)?;
    }
    println!(
        "WebDAV settings saved (password in {}).",
        conn.secret_store().backend_name()
    );
    Ok(())
}

fn config_webdav_push(conn: &CliCtx) -> AppResult<()> {
    let (url, username, password) = read_webdav_settings(conn)?;
    let sync = rssh_lib::sync::webdav::WebDavSync::from_settings(&url, &username, &password)?;
    push_remote_backup(conn, &sync)?;
    println!("Pushed to WebDAV.");
    Ok(())
}

fn config_webdav_pull(conn: &CliCtx) -> AppResult<()> {
    let (url, username, password) = read_webdav_settings(conn)?;
    let sync = rssh_lib::sync::webdav::WebDavSync::from_settings(&url, &username, &password)?;
    pull_remote_backup(conn, &sync)?;
    println!("Pulled from WebDAV.");
    Ok(())
}
