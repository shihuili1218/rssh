//! `secrets` 表 — DB 后端的 SecretStore。
//! 当系统 keychain 不可用（Android、Linux headless 等）时使用。

use rusqlite::params;

use super::Db;
use crate::error::AppResult;

pub fn get(db: &Db, key: &str) -> AppResult<Option<String>> {
    let conn = db.lock()?;
    let result = conn.query_row(
        "SELECT value FROM secrets WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    );
    match result {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn set(db: &Db, key: &str, value: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute(
        "INSERT INTO secrets (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}

pub fn delete(db: &Db, key: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM secrets WHERE key = ?1", params![key])?;
    Ok(())
}
