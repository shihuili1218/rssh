use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::models::{Credential, CredentialType};

fn row_to_credential(row: &rusqlite::Row) -> rusqlite::Result<Credential> {
    let pp: String = row.get(6)?;
    Ok(Credential {
        id: row.get(0)?,
        name: row.get(1)?,
        username: row.get(2)?,
        credential_type: CredentialType::from_str(&row.get::<_, String>(3)?),
        secret: row.get(4)?,
        save_to_remote: row.get::<_, i32>(5)? != 0,
        passphrase: if pp.is_empty() { None } else { Some(pp) },
    })
}

pub fn list(conn: &Connection) -> AppResult<Vec<Credential>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, username, type, secret, save_to_remote, passphrase FROM credentials",
    )?;
    let rows = stmt.query_map([], |row| row_to_credential(row))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<Credential> {
    conn.query_row(
        "SELECT id, name, username, type, secret, save_to_remote, passphrase FROM credentials WHERE id = ?1",
        params![id],
        |row| row_to_credential(row),
    )
    .map_err(Into::into)
}

pub fn insert(conn: &Connection, cred: &Credential) -> AppResult<()> {
    conn.execute(
        "INSERT INTO credentials (id, name, username, type, secret, save_to_remote, passphrase) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            cred.id, cred.name, cred.username, cred.credential_type.as_str(),
            cred.secret.as_deref().unwrap_or(""), cred.save_to_remote as i32,
            cred.passphrase.as_deref().unwrap_or(""),
        ],
    )?;
    Ok(())
}

pub fn update(conn: &Connection, cred: &Credential) -> AppResult<()> {
    conn.execute(
        "UPDATE credentials SET name=?1, username=?2, type=?3, secret=?4, save_to_remote=?5, passphrase=?6 WHERE id=?7",
        params![
            cred.name, cred.username, cred.credential_type.as_str(),
            cred.secret.as_deref().unwrap_or(""), cred.save_to_remote as i32,
            cred.passphrase.as_deref().unwrap_or(""), cred.id,
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM credentials WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn clear_all(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM credentials", [])?;
    Ok(())
}
