//! `rssh config <export|import|set|push|pull>` —— config backup & GitHub sync.
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
    /// Set GitHub sync settings
    Set,
    /// Push config to GitHub
    Push,
    /// Pull config from GitHub
    Pull,
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

fn config_set(conn: &CliCtx) -> AppResult<()> {
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

fn config_push(conn: &CliCtx) -> AppResult<()> {
    let token = conn
        .secret_store()
        .get(&setting_key("github_token"))?
        .unwrap_or_else(|| die("GitHub token not set. Run: rssh config set"));
    let repo = rssh_lib::db::settings::get(conn, "github_repo")?
        .unwrap_or_else(|| die("GitHub repo not set"));
    let branch = rssh_lib::db::settings::get(conn, "github_branch")?.unwrap_or("main".into());

    // push 路径：跟 GUI push 用同一个 build_payload —— 尊重所有同步开关 + group 过滤
    // + save_to_remote scrub + AI-key 闸门。GUI 里关掉的类别，CLI 也不会漏到同一 repo。
    let ss: &dyn SecretStore = conn.secret_store().as_ref();
    let prefs = rssh_lib::sync::config::read_sync_prefs(conn)?;
    let payload = rssh_lib::sync::config::build_payload(
        conn,
        ss,
        &conn.data_dir,
        &rssh_lib::sync::config::ExportMode::GitHubPush(prefs),
    )?;
    let mut json_data = serde_json::to_string_pretty(&payload)
        .unwrap_or_else(|e| die(format!("Serialization failed: {e}")));

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
    // pull: additive merge (no destructive clear) — same as the GUI.
    import_config_json(conn, &json)?;
    println!("Pulled from GitHub.");
    Ok(())
}
