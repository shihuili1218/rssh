//! `rssh config <export|import|set|push|pull>` —— config backup & GitHub sync.
//!
//! Both `import` and `pull` go through `merge_import` (additive upsert by
//! identity, never destructive), sharing `rssh_lib::sync::config` with the GUI.

use clap::Subcommand;
use rssh_lib::error::{AppError, AppResult};
use rssh_lib::secret::{cred_secret_key, setting_key, SecretStore};

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

/// 构造 export / push 共用的 JSON 形态。
///
/// The shape must cover the GUI export field set (profiles / credentials /
/// forwards / groups / serial_profiles / skills) so a CLI backup carries every
/// category. Under merge semantics a missing key is simply not synced (it never
/// wipes the other side), but parity keeps CLI ↔ GUI round-trips lossless.
///
/// `respect_save_to_remote = true` (push path) sets the secret of
/// `save_to_remote=false` credentials to None; the local export path passes
/// false so every secret lands in the encrypted file. `include_ai_keys` gates
/// plaintext AI provider keys the same way the GUI push honors the
/// `sync_include_ai_key` toggle — push reads the setting, local export passes true.
fn build_config_json(
    conn: &CliCtx,
    respect_save_to_remote: bool,
    include_ai_keys: bool,
) -> AppResult<String> {
    let profiles = rssh_lib::db::profile::list(conn)?;
    let mut credentials = rssh_lib::db::credential::list(conn)?;
    for c in credentials.iter_mut() {
        c.secret = conn.secret_store().get(&cred_secret_key(&c.id))?;
        if respect_save_to_remote && !c.save_to_remote {
            c.secret = None;
        }
    }
    let forwards = rssh_lib::db::forward::list(conn)?;
    let serial_profiles = rssh_lib::db::serial_profile::list(conn)?;
    let groups = rssh_lib::db::group::list(conn)?;
    // ai_skill 表只有 user 自定义条目，builtin "general" 不入表。
    // SkillRecord wire format 需要 builtin 字段，inline 拼出来避免依赖 ai 模块。
    let skills: Vec<serde_json::Value> = rssh_lib::db::ai_skill::list(conn)?
        .into_iter()
        .map(|u| {
            serde_json::json!({
                "id": u.id,
                "name": u.name,
                "description": u.description,
                "content": u.content,
                "builtin": false,
            })
        })
        .collect();
    let highlights = rssh_lib::db::highlight::list(conn)?;
    let snippets = rssh_lib::db::snippet::load(&conn.data_dir)?;
    let ai_redact_rules = rssh_lib::db::ai_redact_rule::list(conn)?;
    let ai_command_blacklist = rssh_lib::db::ai_command_blacklist::list(conn)?;
    let ss: &dyn SecretStore = conn.secret_store().as_ref();
    let ai = rssh_lib::ai::commands::export_ai_settings(conn, ss, include_ai_keys)?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "profiles": profiles,
        "credentials": credentials,
        "forwards": forwards,
        "serial_profiles": serial_profiles,
        "groups": groups,
        "skills": skills,
        "highlights": highlights,
        "snippets": snippets,
        "ai_redact_rules": ai_redact_rules,
        "ai_command_blacklist": ai_command_blacklist,
        "ai": ai,
    }))
    .unwrap_or_else(|e| die(format!("Serialization failed: {e}"))))
}

/// Parse JSON then delegate to the shared sync logic (same path as the GUI).
fn import_config_json(conn: &CliCtx, json: &str) -> AppResult<()> {
    let data: serde_json::Value =
        serde_json::from_str(json).unwrap_or_else(|e| die(format!("JSON parse error: {e}")));
    let ss: &dyn SecretStore = conn.secret_store().as_ref();
    rssh_lib::sync::config::merge_import(conn, ss, &conn.data_dir, &data)
}

fn config_export(conn: &CliCtx, file: &str) -> AppResult<()> {
    // 本地 export：所有 secret 都加密落盘，不看 save_to_remote；AI key 一并保留。
    let json = build_config_json(conn, false, true)?;
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

    // push 路径：尊重 save_to_remote（不同步的凭证 secret 置 None）+ sync_include_ai_key
    // 闸门（GUI 关了就别从 CLI 把 key 漏到同一个 repo）。absent / 非 "0" = 开。
    let include_ai_keys = rssh_lib::db::settings::get(conn, "sync_include_ai_key")?
        .is_none_or(|v| v != "0");
    let mut json_data = build_config_json(conn, true, include_ai_keys)?;

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
