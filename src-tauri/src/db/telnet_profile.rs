use rusqlite::params;

use super::Db;
use crate::error::AppResult;
use crate::models::{validate_name, TelnetProfile};

const COLS: &str =
    "id, name, host, port, input_newline, output_newline, local_echo, backspace, login_script, group_id";

fn from_row(row: &rusqlite::Row) -> rusqlite::Result<TelnetProfile> {
    Ok(TelnetProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        // rusqlite's u16 FromSql is range-checked: a corrupted out-of-range
        // INTEGER fails loudly instead of silently truncating.
        port: row.get(3)?,
        input_newline: row.get(4)?,
        output_newline: row.get(5)?,
        local_echo: row.get(6)?,
        backspace: row.get(7)?,
        login_script: row.get(8)?,
        group_id: row.get(9)?,
    })
}

pub fn get(db: &Db, id: &str) -> AppResult<TelnetProfile> {
    let conn = db.lock()?;
    conn.query_row(
        &format!("SELECT {COLS} FROM telnet_profiles WHERE id = ?1"),
        params![id],
        from_row,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            crate::error::AppError::not_found("telnet_profile_not_found", serde_json::json!({}))
        }
        other => other.into(),
    })
}

pub fn list(db: &Db) -> AppResult<Vec<TelnetProfile>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM telnet_profiles ORDER BY name ASC"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn insert_tx(conn: &rusqlite::Connection, t: &TelnetProfile) -> AppResult<()> {
    validate_name(&t.name)?;
    conn.execute(
        "INSERT INTO telnet_profiles \
         (id, name, host, port, input_newline, output_newline, local_echo, backspace, login_script, group_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, host=excluded.host, port=excluded.port, \
          input_newline=excluded.input_newline, output_newline=excluded.output_newline, \
          local_echo=excluded.local_echo, backspace=excluded.backspace, \
          login_script=excluded.login_script, group_id=excluded.group_id",
        params![
            t.id,
            t.name,
            t.host,
            t.port,
            t.input_newline,
            t.output_newline,
            t.local_echo,
            t.backspace,
            t.login_script,
            t.group_id,
        ],
    )?;
    Ok(())
}

pub fn insert(db: &Db, t: &TelnetProfile) -> AppResult<()> {
    let conn = db.lock()?;
    insert_tx(&conn, t)
}

pub fn update(db: &Db, t: &TelnetProfile) -> AppResult<()> {
    validate_name(&t.name)?;
    let conn = db.lock()?;
    conn.execute(
        "UPDATE telnet_profiles SET name=?1, host=?2, port=?3, input_newline=?4, output_newline=?5, \
         local_echo=?6, backspace=?7, login_script=?8, group_id=?9 WHERE id=?10",
        params![
            t.name,
            t.host,
            t.port,
            t.input_newline,
            t.output_newline,
            t.local_echo,
            t.backspace,
            t.login_script,
            t.group_id,
            t.id,
        ],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM telnet_profiles WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn clear_all_tx(conn: &rusqlite::Connection) -> AppResult<()> {
    conn.execute("DELETE FROM telnet_profiles", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, name: &str) -> TelnetProfile {
        TelnetProfile {
            id: id.into(),
            name: name.into(),
            host: "192.168.1.1".into(),
            port: 23,
            input_newline: "crlf".into(),
            output_newline: "raw".into(),
            local_echo: false,
            backspace: "del".into(),
            login_script: String::new(),
            group_id: None,
        }
    }

    #[test]
    fn insert_then_get_roundtrips_all_fields() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("t1", "core-switch")).unwrap();
        let got = get(&db, "t1").unwrap();
        assert_eq!(got.name, "core-switch");
        assert_eq!(got.host, "192.168.1.1");
        assert_eq!(got.port, 23);
        assert_eq!(got.input_newline, "crlf");
        assert_eq!(got.output_newline, "raw");
        assert_eq!(got.backspace, "del");
        assert!(!got.local_echo);
    }

    #[test]
    fn upsert_overwrites_line_discipline() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("t1", "switch")).unwrap();
        let mut u = mk("t1", "switch");
        u.port = 2323;
        u.local_echo = true;
        u.input_newline = "cr".into();
        u.backspace = "bs".into();
        u.login_script = "expect login:\nsend admin".into();
        insert(&db, &u).unwrap();
        let got = get(&db, "t1").unwrap();
        assert_eq!(got.port, 2323);
        assert!(got.local_echo);
        assert_eq!(got.input_newline, "cr");
        assert_eq!(got.backspace, "bs");
        assert_eq!(got.login_script, "expect login:\nsend admin");
    }

    #[test]
    fn update_changes_group_id() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("t1", "switch")).unwrap();
        let mut t = mk("t1", "switch");
        t.group_id = Some("g9".into());
        update(&db, &t).unwrap();
        assert_eq!(get(&db, "t1").unwrap().group_id.as_deref(), Some("g9"));
    }

    #[test]
    fn list_sorted_by_name() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("t1", "zebra")).unwrap();
        insert(&db, &mk("t2", "apple")).unwrap();
        let names: Vec<String> = list(&db).unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["apple", "zebra"]);
    }

    #[test]
    fn delete_removes_row() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("t1", "switch")).unwrap();
        delete(&db, "t1").unwrap();
        assert_eq!(
            get(&db, "t1").unwrap_err().code(),
            "telnet_profile_not_found"
        );
    }

    #[test]
    fn insert_rejects_name_with_control_char() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(
            insert(&db, &mk("t1", "bad\nname")).unwrap_err().code(),
            "name_has_control_char"
        );
    }

    #[test]
    fn high_port_roundtrips() {
        // u16 boundary through the INTEGER column.
        let db = Db::open_in_memory().unwrap();
        let mut t = mk("t1", "highport");
        t.port = 65535;
        insert(&db, &t).unwrap();
        assert_eq!(get(&db, "t1").unwrap().port, 65535);
    }

    #[test]
    fn out_of_range_port_in_db_fails_loudly() {
        // The typed API can't store >65535; plant it with raw SQL the way a
        // corrupted row would arrive. Reading back must error, not truncate.
        let db = Db::open_in_memory().unwrap();
        db.lock()
            .unwrap()
            .execute(
                "INSERT INTO telnet_profiles (id, name, host, port) VALUES ('x', 'x', 'h', 70000)",
                [],
            )
            .unwrap();
        assert!(get(&db, "x").is_err());
    }

    #[test]
    fn older_payload_without_new_keys_deserializes_with_defaults() {
        // Import path: a minimal JSON (id/name/host only) must parse — serde
        // defaults fill the rest. Guards the sync/export compatibility promise.
        let t: TelnetProfile =
            serde_json::from_str(r#"{"id":"t1","name":"sw","host":"10.0.0.1"}"#).unwrap();
        assert_eq!(t.port, 23);
        assert_eq!(t.input_newline, "crlf");
        assert_eq!(t.output_newline, "raw");
        assert_eq!(t.backspace, "del");
        assert!(!t.local_echo);
        assert_eq!(t.group_id, None);
    }
}
