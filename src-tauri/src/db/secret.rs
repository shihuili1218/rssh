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

/// 罗列整表 (key, value) — 仅迁移代码用：v0.1.x 时代 keychain 不可用 fallback
/// 到 DbStore 走的是明文存储，新版本 HybridStore 要把这些明文 re-encrypt 一遍。
/// 普通运行时不应该用这个 API（SecretStore trait 也没暴露），避免误用让明文
/// 漏出 secrets 表边界。
///
/// `pub(crate)`：API 暴露面收窄到本 lib 内部，外部 binary 无法直接拿到所有
/// 密文/旧明文 blob —— 防止被误调进非迁移路径，把整表 secret 流出。
pub(crate) fn list_all(db: &Db) -> AppResult<Vec<(String, String)>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare("SELECT key, value FROM secrets")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
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
    fn set_then_get_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        set(&db, "cred:abc", "s3cret").unwrap();
        assert_eq!(get(&db, "cred:abc").unwrap().as_deref(), Some("s3cret"));
    }

    #[test]
    fn set_twice_overwrites() {
        let db = Db::open_in_memory().unwrap();
        set(&db, "k", "v1").unwrap();
        set(&db, "k", "v2").unwrap();
        assert_eq!(get(&db, "k").unwrap().as_deref(), Some("v2"));
    }

    #[test]
    fn delete_removes_key() {
        let db = Db::open_in_memory().unwrap();
        set(&db, "k", "v").unwrap();
        delete(&db, "k").unwrap();
        assert!(get(&db, "k").unwrap().is_none());
    }

    #[test]
    fn delete_missing_key_is_noop() {
        // 幂等：删一个不存在的 key 不应该报错（rm 语义）
        let db = Db::open_in_memory().unwrap();
        delete(&db, "ghost").unwrap();
    }
}
