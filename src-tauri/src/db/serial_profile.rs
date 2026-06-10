use rusqlite::params;

use super::Db;
use crate::error::AppResult;
use crate::models::{validate_name, SerialProfile};

const COLS: &str = "id, name, port, baud_rate, data_bits, parity, stop_bits, flow_control, \
     xany, input_newline, output_newline, local_echo, backspace, slow_send, input_mode, output_mode, login_script";

fn from_row(row: &rusqlite::Row) -> rusqlite::Result<SerialProfile> {
    Ok(SerialProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        port: row.get(2)?,
        baud_rate: row.get(3)?,
        // data_bits / stop_bits are small ints; stored as INTEGER, narrowed here.
        data_bits: row.get::<_, u32>(4)? as u8,
        parity: row.get(5)?,
        stop_bits: row.get::<_, u32>(6)? as u8,
        flow_control: row.get(7)?,
        xany: row.get(8)?,
        input_newline: row.get(9)?,
        output_newline: row.get(10)?,
        local_echo: row.get(11)?,
        backspace: row.get(12)?,
        slow_send: row.get(13)?,
        input_mode: row.get(14)?,
        output_mode: row.get(15)?,
        login_script: row.get(16)?,
    })
}

pub fn get(db: &Db, id: &str) -> AppResult<SerialProfile> {
    let conn = db.lock()?;
    conn.query_row(
        &format!("SELECT {COLS} FROM serial_profiles WHERE id = ?1"),
        params![id],
        from_row,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            crate::error::AppError::not_found("serial_profile_not_found", serde_json::json!({}))
        }
        other => other.into(),
    })
}

pub fn list(db: &Db) -> AppResult<Vec<SerialProfile>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM serial_profiles ORDER BY name ASC"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn insert_tx(conn: &rusqlite::Connection, s: &SerialProfile) -> AppResult<()> {
    validate_name(&s.name)?;
    conn.execute(
        "INSERT INTO serial_profiles \
         (id, name, port, baud_rate, data_bits, parity, stop_bits, flow_control, \
          xany, input_newline, output_newline, local_echo, backspace, slow_send, input_mode, output_mode, login_script) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17) \
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, port=excluded.port, baud_rate=excluded.baud_rate, \
          data_bits=excluded.data_bits, parity=excluded.parity, stop_bits=excluded.stop_bits, flow_control=excluded.flow_control, \
          xany=excluded.xany, input_newline=excluded.input_newline, output_newline=excluded.output_newline, \
          local_echo=excluded.local_echo, backspace=excluded.backspace, slow_send=excluded.slow_send, \
          input_mode=excluded.input_mode, output_mode=excluded.output_mode, login_script=excluded.login_script",
        params![
            s.id, s.name, s.port, s.baud_rate, s.data_bits as u32, s.parity, s.stop_bits as u32, s.flow_control,
            s.xany, s.input_newline, s.output_newline, s.local_echo, s.backspace, s.slow_send, s.input_mode, s.output_mode, s.login_script,
        ],
    )?;
    Ok(())
}

pub fn insert(db: &Db, s: &SerialProfile) -> AppResult<()> {
    let conn = db.lock()?;
    insert_tx(&conn, s)
}

pub fn update(db: &Db, s: &SerialProfile) -> AppResult<()> {
    validate_name(&s.name)?;
    let conn = db.lock()?;
    conn.execute(
        "UPDATE serial_profiles SET name=?1, port=?2, baud_rate=?3, data_bits=?4, parity=?5, stop_bits=?6, flow_control=?7, \
         xany=?8, input_newline=?9, output_newline=?10, local_echo=?11, backspace=?12, slow_send=?13, input_mode=?14, output_mode=?15, login_script=?16 \
         WHERE id=?17",
        params![
            s.name, s.port, s.baud_rate, s.data_bits as u32, s.parity, s.stop_bits as u32, s.flow_control,
            s.xany, s.input_newline, s.output_newline, s.local_echo, s.backspace, s.slow_send, s.input_mode, s.output_mode, s.login_script,
            s.id,
        ],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM serial_profiles WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn clear_all_tx(conn: &rusqlite::Connection) -> AppResult<()> {
    conn.execute("DELETE FROM serial_profiles", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, name: &str) -> SerialProfile {
        SerialProfile {
            id: id.into(),
            name: name.into(),
            port: "/dev/ttyUSB0".into(),
            baud_rate: 115200,
            data_bits: 8,
            parity: "none".into(),
            stop_bits: 1,
            flow_control: "none".into(),
            xany: false,
            input_newline: "cr".into(),
            output_newline: "raw".into(),
            local_echo: false,
            backspace: "del".into(),
            slow_send: false,
            input_mode: "normal".into(),
            output_mode: "text".into(),
            login_script: String::new(),
        }
    }

    #[test]
    fn insert_then_get_roundtrips_all_fields() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("s1", "router-console")).unwrap();
        let got = get(&db, "s1").unwrap();
        assert_eq!(got.name, "router-console");
        assert_eq!(got.baud_rate, 115200);
        assert_eq!(got.data_bits, 8);
        assert_eq!(got.input_newline, "cr");
        assert_eq!(got.output_newline, "raw");
        assert_eq!(got.backspace, "del");
        assert!(!got.xany);
        assert!(!got.local_echo);
    }

    #[test]
    fn upsert_overwrites_tabby_extras() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("s1", "router")).unwrap();
        let mut u = mk("s1", "router");
        u.xany = true;
        u.local_echo = true;
        u.slow_send = true;
        u.input_newline = "crlf".into();
        u.output_newline = "lf".into();
        u.backspace = "bs".into();
        u.input_mode = "hex".into();
        u.output_mode = "hex".into();
        u.login_script = "expect login:\nsend root".into();
        insert(&db, &u).unwrap();
        let got = get(&db, "s1").unwrap();
        assert!(got.xany);
        assert!(got.local_echo);
        assert!(got.slow_send);
        assert_eq!(got.input_newline, "crlf");
        assert_eq!(got.output_newline, "lf");
        assert_eq!(got.backspace, "bs");
        assert_eq!(got.input_mode, "hex");
        assert_eq!(got.output_mode, "hex");
        assert_eq!(got.login_script, "expect login:\nsend root");
    }

    #[test]
    fn list_sorted_by_name() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("s1", "zebra")).unwrap();
        insert(&db, &mk("s2", "apple")).unwrap();
        let names: Vec<String> = list(&db).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["apple", "zebra"]);
    }

    #[test]
    fn delete_removes_row() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("s1", "router")).unwrap();
        delete(&db, "s1").unwrap();
        assert_eq!(
            get(&db, "s1").unwrap_err().code(),
            "serial_profile_not_found"
        );
    }

    #[test]
    fn insert_rejects_name_with_control_char() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(
            insert(&db, &mk("s1", "bad\nname")).unwrap_err().code(),
            "name_has_control_char"
        );
    }
}
