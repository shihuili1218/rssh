//! credentials 表 — 只存元数据（id/name/username/type/save_to_remote）。
//! 实际的 secret / passphrase 由 SecretStore 管理（系统 keychain 或 secrets 表）。

use rusqlite::params;

use super::Db;
use crate::error::AppResult;
use crate::models::{Credential, CredentialType};

fn row_to_credential(row: &rusqlite::Row) -> rusqlite::Result<Credential> {
    Ok(Credential {
        id: row.get(0)?,
        name: row.get(1)?,
        username: row.get(2)?,
        credential_type: CredentialType::from_str(&row.get::<_, String>(3)?),
        secret: None,
        save_to_remote: row.get::<_, i32>(4)? != 0,
        passphrase: None,
    })
}

pub fn list(db: &Db) -> AppResult<Vec<Credential>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, name, username, type, save_to_remote FROM credentials",
    )?;
    let rows = stmt.query_map([], |row| row_to_credential(row))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn get(db: &Db, id: &str) -> AppResult<Credential> {
    let conn = db.lock()?;
    conn.query_row(
        "SELECT id, name, username, type, save_to_remote FROM credentials WHERE id = ?1",
        params![id],
        |row| row_to_credential(row),
    )
    .map_err(Into::into)
}

pub fn insert(db: &Db, cred: &Credential) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute(
        "INSERT INTO credentials (id, name, username, type, save_to_remote) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            cred.id, cred.name, cred.username, cred.credential_type.as_str(),
            cred.save_to_remote as i32,
        ],
    )?;
    Ok(())
}

pub fn update(db: &Db, cred: &Credential) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute(
        "UPDATE credentials SET name=?1, username=?2, type=?3, save_to_remote=?4 WHERE id=?5",
        params![
            cred.name, cred.username, cred.credential_type.as_str(),
            cred.save_to_remote as i32, cred.id,
        ],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM credentials WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn clear_all(db: &Db) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM credentials", [])?;
    Ok(())
}
