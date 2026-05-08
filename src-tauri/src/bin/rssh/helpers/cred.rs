//! Credential 写盘 + 名字查 id 的通用模板。
//!
//! `Named` trait 把 Profile/Credential/Forward 的 (name, id) 抽象成统一形态，
//! `find_id_by_name` 走泛型一份代替三份重复函数。

use rssh_lib::error::AppResult;
use rssh_lib::models::{Credential, Forward, Profile};
use rssh_lib::secret::cred_secret_key;

use super::tui::die;
use crate::ctx::CliCtx;

// ─── Named trait —————————————————————————————————————————————

pub trait Named {
    fn name(&self) -> &str;
    fn id(&self) -> &str;
}

impl Named for Profile {
    fn name(&self) -> &str {
        &self.name
    }
    fn id(&self) -> &str {
        &self.id
    }
}

impl Named for Credential {
    fn name(&self) -> &str {
        &self.name
    }
    fn id(&self) -> &str {
        &self.id
    }
}

impl Named for Forward {
    fn name(&self) -> &str {
        &self.name
    }
    fn id(&self) -> &str {
        &self.id
    }
}

/// 名字（大小写不敏感）查 id；找不到 die。`kind_label` 用于错误信息（"Profile" / "Credential" / "Forward"）。
/// 返回类型是 `AppResult<String>` 仅因为调用方的签名约定 `?`；die 已 exit(1)，不会真正走 Err 分支。
pub fn find_id_by_name<T: Named>(items: &[T], name: &str, kind_label: &str) -> AppResult<String> {
    items
        .iter()
        .find(|x| x.name().eq_ignore_ascii_case(name))
        .map(|x| x.id().to_string())
        .ok_or_else(|| die(format!("{kind_label} '{name}' not found")))
}

// ─── Credential helpers ————————————————————————————————————

/// 从 SecretStore 把 secret 灌到 Credential 上。
/// keychain 后端报错（系统锁定 / 权限）会传播出来——把它当成"没 secret"会
/// 让 ssh 走错认证路径，且 update 写回时还可能误删一条尚有效的 secret。
pub fn load_cred_secrets(conn: &CliCtx, mut c: Credential) -> AppResult<Credential> {
    if !c.id.is_empty() {
        c.secret = conn.secret_store().get(&cred_secret_key(&c.id))?;
    }
    Ok(c)
}

/// 把 Credential 完整写入（DB INSERT + SecretStore secret）。
/// 私钥 passphrase 不再持久化 — OpenSSH 会在使用 -i 时自行交互索取。
pub fn upsert_cred_with_secrets(conn: &CliCtx, c: &Credential) -> AppResult<()> {
    rssh_lib::db::credential::insert(conn, c)?;
    write_secret(conn, c)
}

/// update 版本（DB UPDATE 而非 INSERT）。
pub fn update_cred_with_secrets(conn: &CliCtx, c: &Credential) -> AppResult<()> {
    rssh_lib::db::credential::update(conn, c)?;
    write_secret(conn, c)
}

fn write_secret(conn: &CliCtx, c: &Credential) -> AppResult<()> {
    let sk = cred_secret_key(&c.id);
    match c.secret.as_deref() {
        Some(s) if !s.is_empty() => conn.secret_store().set(&sk, s)?,
        _ => conn.secret_store().delete(&sk)?,
    }
    Ok(())
}
