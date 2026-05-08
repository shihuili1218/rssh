//! `rssh config <export|import|set|push|pull>` —— 配置备份与 GitHub 同步。
//!
//! `import` 走增量合并（merge_import），`pull` 走全量替换 + 事务（replace_import），
//! 共用 `rssh_lib::sync::config` —— GUI 同一份逻辑。

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

/// 构造 export / push 共用的 JSON 形态。
///
/// 形态必须**完整覆盖** GUI export 的字段集（profiles / credentials / forwards /
/// groups / skills），否则 CLI 产出的 backup 在 GUI replace_pull 时会把缺失的
/// 表清空。CLI ↔ GUI 互导互拉是默认场景，不能在格式上漂移。
///
/// `respect_save_to_remote = true`（push 路径）时把 `save_to_remote=false`
/// 的凭证 secret 置 None；本地 export 路径传 false，所有 secret 都进加密文件。
fn build_config_json(conn: &CliCtx, respect_save_to_remote: bool) -> AppResult<String> {
    let profiles = rssh_lib::db::profile::list(conn)?;
    let mut credentials = rssh_lib::db::credential::list(conn)?;
    for c in credentials.iter_mut() {
        c.secret = conn.secret_store().get(&cred_secret_key(&c.id))?;
        if respect_save_to_remote && !c.save_to_remote {
            c.secret = None;
        }
    }
    let forwards = rssh_lib::db::forward::list(conn)?;
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
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "profiles": profiles,
        "credentials": credentials,
        "forwards": forwards,
        "groups": groups,
        "skills": skills,
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
    // 本地 export：所有 secret 都加密落盘，不看 save_to_remote。
    let json = build_config_json(conn, false)?;
    let pw = read_password("Encryption password: ");
    let pw2 = read_password("Confirm password: ");
    if pw != pw2 {
        return Err(AppError::config("cli_password_mismatch", serde_json::json!({})));
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

    // push 路径：尊重 save_to_remote — 不同步的凭证 secret 置 None。
    let mut json_data = build_config_json(conn, true)?;

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
