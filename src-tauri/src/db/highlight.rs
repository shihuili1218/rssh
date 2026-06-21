use rusqlite::params;

use super::Db;
use crate::error::{AppError, AppResult};
use crate::models::HighlightRule;

fn validate_rule(rule: &HighlightRule) -> AppResult<()> {
    if rule.keyword.trim().is_empty() {
        return Err(AppError::config(
            "highlight_empty_keyword",
            serde_json::json!({}),
        ));
    }
    if !rule.is_regex && !rule.name.trim().is_empty() {
        return Err(AppError::config(
            "highlight_name_for_regex_only",
            serde_json::json!({}),
        ));
    }
    if rule.name.chars().count() > 100 {
        return Err(AppError::config(
            "highlight_name_too_long",
            serde_json::json!({ "max": 100 }),
        ));
    }
    Ok(())
}

pub fn list(db: &Db) -> AppResult<Vec<HighlightRule>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(
        "SELECT keyword, name, color, enabled, is_regex, is_case_sensitive FROM highlights",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(HighlightRule {
            keyword: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
            enabled: row.get::<_, bool>(3)?,
            is_regex: row.get::<_, bool>(4)?,
            is_case_sensitive: row.get::<_, bool>(5)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn insert(db: &Db, rule: &HighlightRule) -> AppResult<()> {
    validate_rule(rule)?;
    let conn = db.lock()?;
    // keyword is the rule's identity: the sync key AND the UI list key (see the
    // keyed `each` in HighlightManager). The schema has no UNIQUE constraint, so
    // a second row with an existing keyword would slip in and crash the settings
    // panel on its duplicate key. Reject it here, mirroring update()'s rename-
    // collision guard. (ERROR/WARN/INFO/DEBUG/IPv4 are seeded, so "add ERROR" is
    // the common trigger.)
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
    conn.execute(
        "INSERT INTO highlights (keyword, name, color, enabled, is_regex, is_case_sensitive) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            rule.keyword,
            rule.name,
            rule.color,
            rule.enabled,
            rule.is_regex,
            rule.is_case_sensitive
        ],
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
    validate_rule(rule)?;
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
        "UPDATE highlights SET keyword = ?1, name = ?2, color = ?3, enabled = ?4, is_regex = ?5, is_case_sensitive = ?6 WHERE keyword = ?7",
        params![
            rule.keyword,
            rule.name,
            rule.color,
            rule.enabled,
            rule.is_regex,
            rule.is_case_sensitive,
            old_keyword
        ],
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

/// Upsert addressed by keyword — the sync identity. The autoincrement `id` is
/// local-only and never synced (it would collide across devices). Updates all
/// columns when the keyword exists, inserts otherwise. Used by merge_import;
/// additive, never deletes.
pub fn upsert_by_keyword(db: &Db, rule: &HighlightRule) -> AppResult<()> {
    validate_rule(rule)?;
    let conn = db.lock()?;
    let affected = conn.execute(
        "UPDATE highlights SET name = ?2, color = ?3, enabled = ?4, is_regex = ?5, is_case_sensitive = ?6 WHERE keyword = ?1",
        params![
            rule.keyword,
            rule.name,
            rule.color,
            rule.enabled,
            rule.is_regex,
            rule.is_case_sensitive
        ],
    )?;
    if affected == 0 {
        conn.execute(
            "INSERT INTO highlights (keyword, name, color, enabled, is_regex, is_case_sensitive) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                rule.keyword,
                rule.name,
                rule.color,
                rule.enabled,
                rule.is_regex,
                rule.is_case_sensitive
            ],
        )?;
    }
    Ok(())
}

pub fn reset_defaults(db: &Db) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM highlights", [])?;
    const DEFAULTS: [(&str, &str, &str, bool, bool, bool); 5] = [
        ("ERROR", "", "#FF6B6B", true, false, false),
        ("WARN", "", "#FFD060", true, false, false),
        ("INFO", "", "#6EDAA0", true, false, false),
        ("DEBUG", "", "#40C8E0", true, false, false),
        (
            r"\b(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}\b",
            "IPv4",
            "#D86BFF",
            true,
            true,
            false,
        ),
    ];
    for (keyword, name, color, enabled, is_regex, is_case_sensitive) in &DEFAULTS {
        conn.execute(
            "INSERT INTO highlights (keyword, name, color, enabled, is_regex, is_case_sensitive) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![keyword, name, color, enabled, is_regex, is_case_sensitive],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(keyword: &str) -> HighlightRule {
        HighlightRule {
            keyword: keyword.into(),
            name: String::new(),
            color: "#FF0000".into(),
            enabled: true,
            is_regex: false,
            is_case_sensitive: false,
        }
    }

    #[test]
    fn insert_rejects_seeded_keyword() {
        // C1 repro: a fresh DB seeds ERROR/WARN/INFO/DEBUG/IPv4. Adding "ERROR"
        // via the New form used to INSERT a duplicate row; the keyword-keyed
        // each-block in HighlightManager then threw on the duplicate key and the
        // settings panel went blank on a routine action. insert now rejects it.
        let db = Db::open_in_memory().unwrap();
        assert_eq!(
            insert(&db, &rule("ERROR")).unwrap_err().code(),
            "highlight_keyword_conflict"
        );
    }

    #[test]
    fn insert_rejects_second_duplicate() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &rule("CUSTOM")).unwrap();
        assert_eq!(
            insert(&db, &rule("CUSTOM")).unwrap_err().code(),
            "highlight_keyword_conflict"
        );
        // Exactly one row survives — no duplicate to crash the keyed each block.
        assert_eq!(
            list(&db)
                .unwrap()
                .iter()
                .filter(|r| r.keyword == "CUSTOM")
                .count(),
            1
        );
    }
}
