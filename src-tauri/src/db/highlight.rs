use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::models::HighlightRule;

pub fn list(conn: &Connection) -> AppResult<Vec<HighlightRule>> {
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

pub fn insert(conn: &Connection, rule: &HighlightRule) -> AppResult<()> {
    conn.execute(
        "INSERT INTO highlights (keyword, color, enabled) VALUES (?1, ?2, ?3)",
        params![rule.keyword, rule.color, rule.enabled],
    )?;
    Ok(())
}

pub fn delete_by_keyword(conn: &Connection, keyword: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM highlights WHERE keyword = ?1",
        params![keyword],
    )?;
    Ok(())
}

pub fn reset_defaults(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM highlights", [])?;
    conn.execute_batch(
        "
        INSERT INTO highlights (keyword, color, enabled) VALUES ('ERROR', 'brightRed', 1);
        INSERT INTO highlights (keyword, color, enabled) VALUES ('WARN', 'brightYellow', 1);
        INSERT INTO highlights (keyword, color, enabled) VALUES ('INFO', 'brightGreen', 1);
        INSERT INTO highlights (keyword, color, enabled) VALUES ('DEBUG', 'brightCyan', 1);
        "
    )?;
    Ok(())
}
