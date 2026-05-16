//! credentials 表 — 只存元数据（id/name/username/type/save_to_remote）。
//! 实际的 secret / passphrase 由 SecretStore 管理（系统 keychain 或 secrets 表）。

use rusqlite::params;

use super::Db;
use crate::error::{AppError, AppResult};
use crate::models::{validate_name, Credential, CredentialType};

fn row_to_credential(row: &rusqlite::Row) -> rusqlite::Result<Credential> {
    Ok(Credential {
        id: row.get(0)?,
        name: row.get(1)?,
        username: row.get(2)?,
        credential_type: CredentialType::from_str(&row.get::<_, String>(3)?),
        secret: None,
        save_to_remote: row.get::<_, i32>(4)? != 0,
    })
}

pub fn list(db: &Db) -> AppResult<Vec<Credential>> {
    let conn = db.lock()?;
    let mut stmt =
        conn.prepare("SELECT id, name, username, type, save_to_remote FROM credentials")?;
    let rows = stmt.query_map([], |row| row_to_credential(row))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn get(db: &Db, id: &str) -> AppResult<Credential> {
    let conn = db.lock()?;
    conn.query_row(
        "SELECT id, name, username, type, save_to_remote FROM credentials WHERE id = ?1",
        params![id],
        |row| row_to_credential(row),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            AppError::not_found("credential_not_found", serde_json::json!({ "id": id }))
        }
        other => other.into(),
    })
}

pub fn insert_tx(conn: &rusqlite::Connection, cred: &Credential) -> AppResult<()> {
    validate_name(&cred.name)?;
    conn.execute(
        "INSERT INTO credentials (id, name, username, type, save_to_remote) VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, username=excluded.username, type=excluded.type, save_to_remote=excluded.save_to_remote",
        params![
            cred.id, cred.name, cred.username, cred.credential_type.as_str(),
            cred.save_to_remote as i32,
        ],
    )?;
    Ok(())
}

pub fn insert(db: &Db, cred: &Credential) -> AppResult<()> {
    let conn = db.lock()?;
    insert_tx(&conn, cred)
}

pub fn update(db: &Db, cred: &Credential) -> AppResult<()> {
    validate_name(&cred.name)?;
    let conn = db.lock()?;
    conn.execute(
        "UPDATE credentials SET name=?1, username=?2, type=?3, save_to_remote=?4 WHERE id=?5",
        params![
            cred.name,
            cred.username,
            cred.credential_type.as_str(),
            cred.save_to_remote as i32,
            cred.id,
        ],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM credentials WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn clear_all_tx(conn: &rusqlite::Connection) -> AppResult<()> {
    conn.execute("DELETE FROM credentials", [])?;
    Ok(())
}

pub fn clear_all(db: &Db) -> AppResult<()> {
    let conn = db.lock()?;
    clear_all_tx(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, name: &str, kind: CredentialType) -> Credential {
        Credential {
            id: id.into(),
            name: name.into(),
            username: "root".into(),
            credential_type: kind,
            secret: None,
            save_to_remote: false,
        }
    }

    #[test]
    fn insert_then_get_roundtrip_with_type() {
        // 关键不变量：credential_type 字符串 ↔ enum 必须 roundtrip。
        // schema v9 之后 DB 里存 lowercase 字符串。
        let db = Db::open_in_memory().unwrap();
        for kind in [
            CredentialType::Password,
            CredentialType::Key,
            CredentialType::Interactive,
            CredentialType::Agent,
            CredentialType::None,
        ] {
            let id = format!("c-{}", kind.as_str());
            insert(&db, &mk(&id, kind.as_str(), kind)).unwrap();
            let got = get(&db, &id).unwrap();
            assert_eq!(got.credential_type, kind);
            // schema v11 删了 secret 列；row_to_credential 永远返回 None
            assert!(got.secret.is_none());
        }
    }

    #[test]
    fn get_missing_returns_not_found() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(
            get(&db, "ghost").unwrap_err().code(),
            "credential_not_found"
        );
    }

    #[test]
    fn upsert_overwrites_username_and_type() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("c1", "primary", CredentialType::Password)).unwrap();
        let mut updated = mk("c1", "primary", CredentialType::Key);
        updated.username = "deploy".into();
        insert(&db, &updated).unwrap();
        let got = get(&db, "c1").unwrap();
        assert_eq!(got.username, "deploy");
        assert_eq!(got.credential_type, CredentialType::Key);
    }

    #[test]
    fn save_to_remote_persists_as_bool() {
        let db = Db::open_in_memory().unwrap();
        let mut c = mk("c1", "primary", CredentialType::Password);
        c.save_to_remote = true;
        insert(&db, &c).unwrap();
        assert!(get(&db, "c1").unwrap().save_to_remote);
    }

    #[test]
    fn list_returns_all_inserted() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("c1", "one", CredentialType::Password)).unwrap();
        insert(&db, &mk("c2", "two", CredentialType::Key)).unwrap();
        assert_eq!(list(&db).unwrap().len(), 2);
    }

    #[test]
    fn delete_removes_row() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("c1", "one", CredentialType::Password)).unwrap();
        delete(&db, "c1").unwrap();
        assert_eq!(
            get(&db, "c1").unwrap_err().code(),
            "credential_not_found"
        );
    }

    #[test]
    fn insert_rejects_name_with_control_char() {
        let db = Db::open_in_memory().unwrap();
        let bad = mk("c1", "bad\x1bname", CredentialType::Password);
        assert_eq!(
            insert(&db, &bad).unwrap_err().code(),
            "name_has_control_char"
        );
    }
}
