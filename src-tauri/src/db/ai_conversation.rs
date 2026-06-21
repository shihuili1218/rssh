//! AI conversation persistence (schema v17). Two blobs per row — see the
//! migration comment in schema.rs for the data-fork rationale.
//!
//! Writers are split by ownership: the session actor owns history_json
//! (`save_history`, called at every consistent commit point), the front-end
//! owns timeline_json (`set_timeline`, called after chat-mutating events).
//! They never touch each other's column, so no lost-update hazard.

use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::error::AppResult;

use super::Db;

/// Listing entry — everything except the two blobs, so the picker UI never
/// drags megabytes of history across the IPC boundary.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationMeta {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Full row, fetched once on resume.
#[derive(Debug, Clone)]
pub struct ConversationRow {
    pub id: String,
    pub target_key: String,
    pub title: String,
    pub history_json: String,
    pub timeline_json: String,
}

/// Most recently active first — the picker shows "continue where I left off".
pub fn list(db: &Db, target_key: &str) -> AppResult<Vec<ConversationMeta>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, title, created_at, updated_at FROM ai_conversations
         WHERE target_key = ?1 ORDER BY updated_at DESC, id",
    )?;
    let rows = stmt
        .query_map([target_key], |r| {
            Ok(ConversationMeta {
                id: r.get(0)?,
                title: r.get(1)?,
                created_at: r.get(2)?,
                updated_at: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(db: &Db, id: &str) -> AppResult<Option<ConversationRow>> {
    let conn = db.lock()?;
    let row = conn
        .query_row(
            "SELECT id, target_key, title, history_json, timeline_json
             FROM ai_conversations WHERE id = ?1",
            [id],
            |r| {
                Ok(ConversationRow {
                    id: r.get(0)?,
                    target_key: r.get(1)?,
                    title: r.get(2)?,
                    history_json: r.get(3)?,
                    timeline_json: r.get(4)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Create the row — called exactly once, by session start, after it has won
/// the ai_sessions slot. Row creation is deliberately NOT part of
/// `save_history`: if autosaves could insert, a dying actor's last write
/// would resurrect a conversation the user just deleted. With UPDATE-only
/// autosaves that race is structurally impossible.
pub fn create(db: &Db, id: &str, target_key: &str) -> AppResult<()> {
    let conn = db.lock()?;
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT OR IGNORE INTO ai_conversations (id, target_key, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)",
        params![id, target_key, now],
    )?;
    Ok(())
}

/// Autosave from the session actor. UPDATE-only (see `create`); a miss means
/// the conversation was deleted out from under a stopping actor — the write
/// is intentionally dropped.
///
/// Title is recomputed by the caller from the current first user message, so
/// a cleared-then-restarted conversation re-titles itself naturally.
pub fn save_history(db: &Db, id: &str, title: &str, history_json: &str) -> AppResult<()> {
    let conn = db.lock()?;
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE ai_conversations
         SET title = ?2, history_json = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, title, history_json, now],
    )?;
    Ok(())
}

/// UPDATE-only, same rationale as `save_history`. A miss (0 rows) is silently
/// fine — the next chat event re-saves the full timeline, so the write is
/// self-healing.
pub fn set_timeline(db: &Db, id: &str, timeline_json: &str) -> AppResult<()> {
    let conn = db.lock()?;
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "UPDATE ai_conversations SET timeline_json = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, timeline_json, now],
    )?;
    Ok(())
}

/// The conversation `target_key` for an SSH profile — single source of truth for
/// the "ssh:<profile_id>" convention, shared by ai::commands (storing a
/// conversation) and profile::delete (purging one). Keeping the format in one
/// place means the purge can't silently desync from the store.
pub fn ssh_target_key(profile_id: &str) -> String {
    format!("ssh:{profile_id}")
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM ai_conversations WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(db: &Db, id: &str, key: &str, title: &str, history: &str) {
        create(db, id, key).unwrap();
        save_history(db, id, title, history).unwrap();
    }

    #[test]
    fn save_then_get_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        save(&db, "c1", "ssh:p1", "disk full", r#"[{"role":"user"}]"#);
        let row = get(&db, "c1").unwrap().unwrap();
        assert_eq!(row.target_key, "ssh:p1");
        assert_eq!(row.title, "disk full");
        assert_eq!(row.history_json, r#"[{"role":"user"}]"#);
        assert_eq!(row.timeline_json, "[]"); // column default until front-end writes
    }

    #[test]
    fn save_history_without_create_is_noop() {
        // The anti-resurrection invariant: a dying actor's autosave after the
        // user deleted the conversation must NOT recreate the row.
        let db = Db::open_in_memory().unwrap();
        save_history(&db, "ghost", "t", "[]").unwrap();
        assert!(get(&db, "ghost").unwrap().is_none());
    }

    #[test]
    fn list_filters_by_target_and_orders_recent_first() {
        let db = Db::open_in_memory().unwrap();
        save(&db, "c1", "ssh:p1", "first", "[]");
        save(&db, "c2", "ssh:p2", "other profile", "[]");
        save(&db, "c3", "ssh:p1", "second", "[]");
        // Bump c1 so it becomes the most recent on p1.
        save_history(&db, "c1", "first updated", "[1]").unwrap();
        let metas = list(&db, "ssh:p1").unwrap();
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].id, "c1");
        assert_eq!(metas[0].title, "first updated");
        assert_eq!(metas[1].id, "c3");
        assert!(list(&db, "local").unwrap().is_empty());
    }

    #[test]
    fn save_history_preserves_timeline_and_created_at() {
        let db = Db::open_in_memory().unwrap();
        save(&db, "c1", "local", "t", "[]");
        set_timeline(&db, "c1", r#"[{"kind":"user"}]"#).unwrap();
        let created = {
            let conn = db.lock().unwrap();
            conn.query_row(
                "SELECT created_at FROM ai_conversations WHERE id='c1'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
        };
        save(&db, "c1", "local", "t2", "[2]");
        let row = get(&db, "c1").unwrap().unwrap();
        assert_eq!(row.timeline_json, r#"[{"kind":"user"}]"#);
        assert_eq!(row.history_json, "[2]");
        assert_eq!(row.title, "t2");
        let created_after = {
            let conn = db.lock().unwrap();
            conn.query_row(
                "SELECT created_at FROM ai_conversations WHERE id='c1'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
        };
        assert_eq!(created, created_after);
    }

    #[test]
    fn set_timeline_on_missing_row_is_noop() {
        let db = Db::open_in_memory().unwrap();
        set_timeline(&db, "ghost", "[]").unwrap();
        assert!(get(&db, "ghost").unwrap().is_none());
    }

    #[test]
    fn delete_sticks() {
        let db = Db::open_in_memory().unwrap();
        save(&db, "c1", "local", "t", "[]");
        delete(&db, "c1").unwrap();
        assert!(get(&db, "c1").unwrap().is_none());
        assert!(list(&db, "local").unwrap().is_empty());
    }
}
