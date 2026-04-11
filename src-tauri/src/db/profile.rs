use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::models::Profile;

pub fn list(conn: &Connection) -> AppResult<Vec<Profile>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, host, port, credential_id, bastion_profile_id, init_command FROM profiles ORDER BY name ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Profile {
            id: row.get(0)?,
            name: row.get(1)?,
            host: row.get(2)?,
            port: row.get::<_, u32>(3)? as u16,
            credential_id: row.get(4)?,
            bastion_profile_id: row.get(5)?,
            init_command: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Profile> {
    conn.query_row(
        "SELECT id, name, host, port, credential_id, bastion_profile_id, init_command FROM profiles WHERE id = ?1",
        params![id],
        |row| {
            Ok(Profile {
                id: row.get(0)?,
                name: row.get(1)?,
                host: row.get(2)?,
                port: row.get::<_, u32>(3)? as u16,
                credential_id: row.get(4)?,
                bastion_profile_id: row.get(5)?,
                init_command: row.get(6)?,
            })
        },
    ).map_err(Into::into)
}

pub fn insert(conn: &Connection, p: &Profile) -> AppResult<()> {
    conn.execute(
        "INSERT INTO profiles (id, name, host, port, credential_id, bastion_profile_id, init_command) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(id) DO UPDATE SET \
         name=excluded.name, host=excluded.host, port=excluded.port, \
         credential_id=excluded.credential_id, bastion_profile_id=excluded.bastion_profile_id, \
         init_command=excluded.init_command",
        params![p.id, p.name, p.host, p.port as u32, p.credential_id, p.bastion_profile_id, p.init_command],
    )?;
    Ok(())
}

pub fn update(conn: &Connection, p: &Profile) -> AppResult<()> {
    conn.execute(
        "UPDATE profiles SET name=?1, host=?2, port=?3, credential_id=?4, bastion_profile_id=?5, init_command=?6 WHERE id=?7",
        params![p.name, p.host, p.port as u32, p.credential_id, p.bastion_profile_id, p.init_command, p.id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM profiles WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn clear_all(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM profiles", [])?;
    Ok(())
}
