use rusqlite::params;

use super::Db;
use crate::error::AppResult;
use crate::models::{validate_name, Forward, ForwardType};

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

pub fn get(db: &Db, id: &str) -> AppResult<Forward> {
    let conn = db.lock()?;
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
        rusqlite::Error::QueryReturnedNoRows => crate::error::AppError::not_found("fwd_rule_not_found", serde_json::json!({})),
        other => other.into(),
    })
}

pub fn list(db: &Db) -> AppResult<Vec<Forward>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, name, profile_id, type, local_port, remote_host, remote_port FROM forwards ORDER BY name ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Forward {
            id: row.get(0)?,
            name: row.get(1)?,
            profile_id: row.get(2)?,
            forward_type: parse_type(&row.get::<_, String>(3)?),
            local_port: row.get::<_, u32>(4)? as u16,
            remote_host: row.get(5)?,
            remote_port: row.get::<_, u32>(6)? as u16,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn insert_tx(conn: &rusqlite::Connection, f: &Forward) -> AppResult<()> {
    validate_name(&f.name)?;
    conn.execute(
        "INSERT INTO forwards (id, name, profile_id, type, local_port, remote_host, remote_port) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, profile_id=excluded.profile_id, type=excluded.type, local_port=excluded.local_port, remote_host=excluded.remote_host, remote_port=excluded.remote_port",
        params![f.id, f.name, f.profile_id, type_str(f.forward_type), f.local_port as u32, f.remote_host, f.remote_port as u32],
    )?;
    Ok(())
}

pub fn insert(db: &Db, f: &Forward) -> AppResult<()> {
    let conn = db.lock()?;
    insert_tx(&conn, f)
}

pub fn update(db: &Db, f: &Forward) -> AppResult<()> {
    validate_name(&f.name)?;
    let conn = db.lock()?;
    conn.execute(
        "UPDATE forwards SET name=?1, profile_id=?2, type=?3, local_port=?4, remote_host=?5, remote_port=?6 WHERE id=?7",
        params![f.name, f.profile_id, type_str(f.forward_type), f.local_port as u32, f.remote_host, f.remote_port as u32, f.id],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM forwards WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn clear_all_tx(conn: &rusqlite::Connection) -> AppResult<()> {
    conn.execute("DELETE FROM forwards", [])?;
    Ok(())
}

pub fn clear_all(db: &Db) -> AppResult<()> {
    let conn = db.lock()?;
    clear_all_tx(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn mk(id: &str, name: &str, ft: ForwardType) -> Forward {
        Forward {
            id: id.into(),
            name: name.into(),
            forward_type: ft,
            local_port: 8080,
            remote_host: "127.0.0.1".into(),
            remote_port: 80,
            profile_id: "p1".into(),
        }
    }

    #[test]
    fn insert_then_get_for_all_types() {
        let db = Db::open_in_memory().unwrap();
        for ft in [ForwardType::Local, ForwardType::Remote, ForwardType::Dynamic] {
            let id = format!("f-{}", type_str(ft));
            insert(&db, &mk(&id, &id, ft)).unwrap();
            assert_eq!(get(&db, &id).unwrap().forward_type, ft);
        }
    }

    #[test]
    fn upsert_overwrites_ports() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("f1", "alpha", ForwardType::Local)).unwrap();
        let mut updated = mk("f1", "alpha", ForwardType::Remote);
        updated.local_port = 9999;
        updated.remote_port = 3306;
        insert(&db, &updated).unwrap();
        let got = get(&db, "f1").unwrap();
        assert_eq!(got.forward_type, ForwardType::Remote);
        assert_eq!(got.local_port, 9999);
        assert_eq!(got.remote_port, 3306);
    }

    #[test]
    fn list_sorted_by_name() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("f1", "zebra", ForwardType::Local)).unwrap();
        insert(&db, &mk("f2", "apple", ForwardType::Local)).unwrap();
        let names: Vec<String> = list(&db).unwrap().into_iter().map(|f| f.name).collect();
        assert_eq!(names, vec!["apple", "zebra"]);
    }

    #[test]
    fn delete_removes_row() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("f1", "alpha", ForwardType::Local)).unwrap();
        delete(&db, "f1").unwrap();
        assert_eq!(get(&db, "f1").unwrap_err().code(), "fwd_rule_not_found");
    }

    /// 防御 schema 漂移：DB 里出现未知 type 字符串时不能 panic，应退回 Local。
    /// 通过 raw SQL 注入一个 type='garbage' 的行模拟。
    #[test]
    fn unknown_type_string_falls_back_to_local() {
        let db = Db::open_in_memory().unwrap();
        {
            let conn = db.lock().unwrap();
            conn.execute(
                "INSERT INTO forwards (id, name, profile_id, type, local_port, remote_host, remote_port) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params!["fx", "weird", "p1", "garbage_type", 1u32, "127.0.0.1", 80u32],
            )
            .unwrap();
        }
        assert_eq!(get(&db, "fx").unwrap().forward_type, ForwardType::Local);
    }

    #[test]
    fn insert_rejects_name_with_control_char() {
        let db = Db::open_in_memory().unwrap();
        let bad = mk("f1", "bad\nname", ForwardType::Local);
        assert_eq!(
            insert(&db, &bad).unwrap_err().code(),
            "name_has_control_char"
        );
    }
}
