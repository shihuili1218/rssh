use rusqlite::params;

use super::Db;
use crate::error::AppResult;
use crate::models::Profile;

fn row_to_profile(row: &rusqlite::Row) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        port: row.get::<_, u32>(3)? as u16,
        credential_id: row.get(4)?,
        bastion_profile_id: row.get(5)?,
        init_command: row.get(6)?,
        group_id: row.get(7)?,
    })
}

pub fn list(db: &Db) -> AppResult<Vec<Profile>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, name, host, port, credential_id, bastion_profile_id, init_command, group_id FROM profiles ORDER BY name ASC",
    )?;
    let rows = stmt.query_map([], |row| row_to_profile(row))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn get(db: &Db, id: &str) -> AppResult<Profile> {
    let conn = db.lock()?;
    conn.query_row(
        "SELECT id, name, host, port, credential_id, bastion_profile_id, init_command, group_id FROM profiles WHERE id = ?1",
        params![id],
        |row| row_to_profile(row),
    ).map_err(Into::into)
}

pub fn insert(db: &Db, p: &Profile) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute(
        "INSERT INTO profiles (id, name, host, port, credential_id, bastion_profile_id, init_command, group_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(id) DO UPDATE SET \
         name=excluded.name, host=excluded.host, port=excluded.port, \
         credential_id=excluded.credential_id, bastion_profile_id=excluded.bastion_profile_id, \
         init_command=excluded.init_command, group_id=excluded.group_id",
        params![p.id, p.name, p.host, p.port as u32, p.credential_id, p.bastion_profile_id, p.init_command, p.group_id],
    )?;
    Ok(())
}

pub fn update(db: &Db, p: &Profile) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute(
        "UPDATE profiles SET name=?1, host=?2, port=?3, credential_id=?4, bastion_profile_id=?5, init_command=?6, group_id=?7 WHERE id=?8",
        params![p.name, p.host, p.port as u32, p.credential_id, p.bastion_profile_id, p.init_command, p.group_id, p.id],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM profiles WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn clear_all(db: &Db) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM profiles", [])?;
    Ok(())
}
