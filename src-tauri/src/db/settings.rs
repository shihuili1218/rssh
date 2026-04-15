use rusqlite::params;

use super::Db;
use crate::error::AppResult;

pub fn get(db: &Db, key: &str) -> AppResult<Option<String>> {
    let conn = db.lock()?;
    let result = conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    );
    match result {
        Ok(val) => Ok(Some(val)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn set(db: &Db, key: &str, value: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}
