use rusqlite::params;

use super::Db;
use crate::error::AppResult;
use crate::models::Group;

pub fn list(db: &Db) -> AppResult<Vec<Group>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, name, color, sort_order FROM groups ORDER BY sort_order ASC, name ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Group {
            id: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
            sort_order: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn get(db: &Db, id: &str) -> AppResult<Group> {
    let conn = db.lock()?;
    conn.query_row(
        "SELECT id, name, color, sort_order FROM groups WHERE id = ?1",
        params![id],
        |row| {
            Ok(Group {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                sort_order: row.get(3)?,
            })
        },
    )
    .map_err(Into::into)
}

pub fn insert(db: &Db, g: &Group) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute(
        "INSERT INTO groups (id, name, color, sort_order) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, color=excluded.color, sort_order=excluded.sort_order",
        params![g.id, g.name, g.color, g.sort_order],
    )?;
    Ok(())
}

pub fn update(db: &Db, g: &Group) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute(
        "UPDATE groups SET name=?1, color=?2, sort_order=?3 WHERE id=?4",
        params![g.name, g.color, g.sort_order, g.id],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM groups WHERE id = ?1", params![id])?;
    // Clear group_id references in profiles
    conn.execute(
        "UPDATE profiles SET group_id = NULL WHERE group_id = ?1",
        params![id],
    )?;
    Ok(())
}

pub fn clear_all(db: &Db) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM groups", [])?;
    Ok(())
}
