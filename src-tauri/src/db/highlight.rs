use rusqlite::params;

use super::Db;
use crate::error::{AppError, AppResult};
use crate::models::HighlightRule;

pub fn list(db: &Db) -> AppResult<Vec<HighlightRule>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare("SELECT keyword, color, enabled FROM highlights")?;
    let rows = stmt.query_map([], |row| {
        Ok(HighlightRule {
            keyword: row.get(0)?,
            color: row.get(1)?,
            enabled: row.get::<_, bool>(2)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn insert(db: &Db, rule: &HighlightRule) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute(
        "INSERT INTO highlights (keyword, color, enabled) VALUES (?1, ?2, ?3)",
        params![rule.keyword, rule.color, rule.enabled],
    )?;
    Ok(())
}

pub fn delete_by_keyword(db: &Db, keyword: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute(
        "DELETE FROM highlights WHERE keyword = ?1",
        params![keyword],
    )?;
    Ok(())
}

/// Update an existing highlight rule, addressed by its current keyword.
/// Supports renaming (the new keyword may differ from old_keyword). The schema
/// has no UNIQUE constraint on `keyword`, so when renaming we explicitly check
/// for a collision against any other row and return a business error rather
/// than silently producing duplicate rows.
pub fn update(db: &Db, old_keyword: &str, rule: &HighlightRule) -> AppResult<()> {
    let conn = db.lock()?;
    if rule.keyword != old_keyword {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM highlights WHERE keyword = ?1",
            params![rule.keyword],
            |r| r.get(0),
        )?;
        if exists > 0 {
            return Err(AppError::other(
                "highlight_keyword_conflict",
                serde_json::json!({ "keyword": rule.keyword }),
            ));
        }
    }
    let affected = conn.execute(
        "UPDATE highlights SET keyword = ?1, color = ?2, enabled = ?3 WHERE keyword = ?4",
        params![rule.keyword, rule.color, rule.enabled, old_keyword],
    )?;
    if affected == 0 {
        // No row matched old_keyword — UI would otherwise show a fake success.
        return Err(AppError::other(
            "highlight_not_found",
            serde_json::json!({ "keyword": old_keyword }),
        ));
    }
    Ok(())
}

pub fn reset_defaults(db: &Db) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM highlights", [])?;
    conn.execute_batch(
        "
        INSERT INTO highlights (keyword, color, enabled) VALUES ('ERROR', '#FF6B6B', 1);
        INSERT INTO highlights (keyword, color, enabled) VALUES ('WARN', '#FFD060', 1);
        INSERT INTO highlights (keyword, color, enabled) VALUES ('INFO', '#6EDAA0', 1);
        INSERT INTO highlights (keyword, color, enabled) VALUES ('DEBUG', '#40C8E0', 1);
        ",
    )?;
    Ok(())
}
