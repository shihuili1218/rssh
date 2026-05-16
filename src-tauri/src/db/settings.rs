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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_key_returns_none() {
        let db = Db::open_in_memory().unwrap();
        assert!(get(&db, "ghost").unwrap().is_none());
    }

    #[test]
    fn set_then_get_returns_value() {
        let db = Db::open_in_memory().unwrap();
        set(&db, "theme", "dark").unwrap();
        assert_eq!(get(&db, "theme").unwrap().as_deref(), Some("dark"));
    }

    #[test]
    fn set_twice_overwrites() {
        // UPSERT 必须真覆盖：用户切主题不能留旧值
        let db = Db::open_in_memory().unwrap();
        set(&db, "theme", "dark").unwrap();
        set(&db, "theme", "light").unwrap();
        assert_eq!(get(&db, "theme").unwrap().as_deref(), Some("light"));
    }

    #[test]
    fn empty_string_value_persists() {
        // 空串 ≠ 缺失：用户主动清空设置时要能拿到 Some("")，不是 None
        let db = Db::open_in_memory().unwrap();
        set(&db, "k", "").unwrap();
        assert_eq!(get(&db, "k").unwrap().as_deref(), Some(""));
    }
}
