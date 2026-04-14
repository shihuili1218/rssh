use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::models::{Forward, ForwardType};

fn parse_type(s: &str) -> ForwardType {
    match s {
        "remote" => ForwardType::Remote,
        "dynamic" => ForwardType::Dynamic,
        _ => ForwardType::Local,
    }
}
fn type_str(ft: ForwardType) -> &'static str {
    match ft {
        ForwardType::Local => "local",
        ForwardType::Remote => "remote",
        ForwardType::Dynamic => "dynamic",
    }
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Forward> {
    conn.query_row(
        "SELECT id, name, profile_id, type, local_port, remote_host, remote_port FROM forwards WHERE id = ?1",
        params![id],
        |row| Ok(Forward {
            id: row.get(0)?, name: row.get(1)?, profile_id: row.get(2)?,
            forward_type: parse_type(&row.get::<_, String>(3)?),
            local_port: row.get::<_, u32>(4)? as u16,
            remote_host: row.get(5)?, remote_port: row.get::<_, u32>(6)? as u16,
        }),
    ).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => crate::error::AppError::NotFound("转发规则不存在".into()),
        other => other.into(),
    })
}

pub fn list(conn: &Connection) -> AppResult<Vec<Forward>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, profile_id, type, local_port, remote_host, remote_port FROM forwards ORDER BY name ASC",
    )?;
    let rows = stmt.query_map([], |row| Ok(Forward {
        id: row.get(0)?, name: row.get(1)?, profile_id: row.get(2)?,
        forward_type: parse_type(&row.get::<_, String>(3)?),
        local_port: row.get::<_, u32>(4)? as u16,
        remote_host: row.get(5)?, remote_port: row.get::<_, u32>(6)? as u16,
    }))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn insert(conn: &Connection, f: &Forward) -> AppResult<()> {
    conn.execute(
        "INSERT INTO forwards (id, name, profile_id, type, local_port, remote_host, remote_port) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![f.id, f.name, f.profile_id, type_str(f.forward_type), f.local_port as u32, f.remote_host, f.remote_port as u32],
    )?;
    Ok(())
}

pub fn update(conn: &Connection, f: &Forward) -> AppResult<()> {
    conn.execute(
        "UPDATE forwards SET name=?1, profile_id=?2, type=?3, local_port=?4, remote_host=?5, remote_port=?6 WHERE id=?7",
        params![f.name, f.profile_id, type_str(f.forward_type), f.local_port as u32, f.remote_host, f.remote_port as u32, f.id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM forwards WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn clear_all(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM forwards", [])?;
    Ok(())
}
