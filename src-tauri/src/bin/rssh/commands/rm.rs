//! `rssh rm <profile|cred|fwd> <name>` —— 带二次确认的删除。

use rssh_lib::error::{AppError, AppResult};
use rssh_lib::secret::cred_secret_key;

use crate::ctx::CliCtx;
use crate::helpers::{confirm, find_id_by_name};

pub fn cmd_rm(conn: &CliCtx, kind: &str, name: &str) -> AppResult<()> {
    match kind {
        "profile" => {
            let profiles = rssh_lib::db::profile::list(conn)?;
            let id = find_id_by_name(&profiles, name, "Profile")?;
            if !confirm(&format!("Delete profile '{name}'?"), false) {
                return Ok(());
            }
            rssh_lib::db::profile::delete(conn, &id)?;
            println!("Deleted.");
        }
        "cred" | "creds" => {
            let creds = rssh_lib::db::credential::list(conn)?;
            let id = find_id_by_name(&creds, name, "Credential")?;
            if !confirm(&format!("Delete credential '{name}'?"), false) {
                return Ok(());
            }
            rssh_lib::db::credential::delete(conn, &id)?;
            let _ = conn.secret_store().delete(&cred_secret_key(&id));
            println!("Deleted.");
        }
        "fwd" => {
            let forwards = rssh_lib::db::forward::list(conn)?;
            let id = find_id_by_name(&forwards, name, "Forward")?;
            if !confirm(&format!("Delete forward '{name}'?"), false) {
                return Ok(());
            }
            rssh_lib::db::forward::delete(conn, &id)?;
            println!("Deleted.");
        }
        _ => {
            return Err(AppError::config(
                "cli_unknown_kind",
                serde_json::json!({ "kind": kind, "valid": "profile, cred, fwd" }),
            ))
        }
    }
    Ok(())
}
