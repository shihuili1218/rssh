use rusqlite::params;

use super::Db;
use crate::error::{AppError, AppResult};
use crate::models::{validate_name, Profile};

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
    ).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            AppError::not_found("profile_not_found", serde_json::json!({ "id": id }))
        }
        other => other.into(),
    })
}

/// Profile 写入的 single source of truth。`models.rs::Profile.credential_id`
/// 文档里写的"应用层不变量"在这里 enforce —— UI/CLI 层的校验是 UX 早期提示，
/// 任何路径（包括 Tauri `create_profile`/`update_profile` 命令、外部脚本 invoke、
/// 旧前端、未来新增的写入入口）最终都得过这道关。
fn validate_for_write(p: &Profile) -> AppResult<()> {
    validate_name(&p.name)?;
    if p.credential_id.is_empty() {
        return Err(AppError::config(
            "profile_credential_id_required",
            serde_json::json!({ "id": p.id, "name": p.name }),
        ));
    }
    Ok(())
}

pub fn insert_tx(conn: &rusqlite::Connection, p: &Profile) -> AppResult<()> {
    validate_for_write(p)?;
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

pub fn insert(db: &Db, p: &Profile) -> AppResult<()> {
    let conn = db.lock()?;
    insert_tx(&conn, p)
}

pub fn update(db: &Db, p: &Profile) -> AppResult<()> {
    validate_for_write(p)?;
    let conn = db.lock()?;
    conn.execute(
        "UPDATE profiles SET name=?1, host=?2, port=?3, credential_id=?4, bastion_profile_id=?5, init_command=?6, group_id=?7 WHERE id=?8",
        params![p.name, p.host, p.port as u32, p.credential_id, p.bastion_profile_id, p.init_command, p.group_id, p.id],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    // Deleting an SSH profile must also purge its AI conversations (keyed
    // "ssh:<id>", holding UNREDACTED terminal output). Once the profile is gone
    // the AI panel — which only opens per live target — can't reach them, so
    // they'd linger in the DB forever with no way to clear them. Atomic so a
    // crash can't leave the profile deleted but its transcripts behind.
    let convo_key = crate::db::ai_conversation::ssh_target_key(id);
    db.with_transaction(|tx| {
        tx.execute("DELETE FROM profiles WHERE id = ?1", params![id])?;
        tx.execute(
            "DELETE FROM ai_conversations WHERE target_key = ?1",
            params![convo_key],
        )?;
        Ok(())
    })
}

pub fn clear_all_tx(conn: &rusqlite::Connection) -> AppResult<()> {
    conn.execute("DELETE FROM profiles", [])?;
    Ok(())
}

pub fn clear_all(db: &Db) -> AppResult<()> {
    let conn = db.lock()?;
    clear_all_tx(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, name: &str) -> Profile {
        Profile {
            id: id.into(),
            name: name.into(),
            host: "h.example".into(),
            port: 22,
            credential_id: "c1".into(),
            bastion_profile_id: None,
            init_command: None,
            group_id: None,
        }
    }

    #[test]
    fn insert_then_get_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let p = mk("p1", "alpha");
        insert(&db, &p).unwrap();
        let got = get(&db, "p1").unwrap();
        assert_eq!(got.name, "alpha");
        assert_eq!(got.host, "h.example");
        assert_eq!(got.port, 22);
    }

    #[test]
    fn get_missing_returns_not_found() {
        let db = Db::open_in_memory().unwrap();
        let err = get(&db, "ghost").unwrap_err();
        assert_eq!(err.code(), "profile_not_found");
    }

    #[test]
    fn upsert_overwrites_fields() {
        // 同 id 第二次 insert = UPDATE。host 字段必须被新值覆盖，
        // 否则前端"编辑 profile"功能会留下脏数据。
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("p1", "alpha")).unwrap();
        let mut updated = mk("p1", "alpha");
        updated.host = "new.example".into();
        updated.port = 2222;
        insert(&db, &updated).unwrap();
        let got = get(&db, "p1").unwrap();
        assert_eq!(got.host, "new.example");
        assert_eq!(got.port, 2222);
    }

    #[test]
    fn list_sorted_by_name_asc() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("p3", "charlie")).unwrap();
        insert(&db, &mk("p1", "alpha")).unwrap();
        insert(&db, &mk("p2", "bravo")).unwrap();
        let names: Vec<String> = list(&db).unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn delete_removes_row() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("p1", "alpha")).unwrap();
        delete(&db, "p1").unwrap();
        assert_eq!(get(&db, "p1").unwrap_err().code(), "profile_not_found");
    }

    /// #2: an SSH profile's AI conversations (target_key "ssh:<id>", holding
    /// UNREDACTED terminal output) must die with the profile — otherwise they
    /// linger unreachable in the DB, since the AI panel only opens per live
    /// target. Scoped: other profiles' and non-ssh conversations stay untouched.
    #[test]
    fn delete_purges_profile_ai_conversations() {
        use crate::db::ai_conversation;
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("p1", "web")).unwrap();
        insert(&db, &mk("p2", "db")).unwrap();
        ai_conversation::create(&db, "c1", &ai_conversation::ssh_target_key("p1")).unwrap();
        ai_conversation::create(&db, "c2", &ai_conversation::ssh_target_key("p2")).unwrap();
        ai_conversation::create(&db, "c3", "local").unwrap();

        delete(&db, "p1").unwrap();

        assert!(ai_conversation::get(&db, "c1").unwrap().is_none());
        assert!(ai_conversation::get(&db, "c2").unwrap().is_some());
        assert!(ai_conversation::get(&db, "c3").unwrap().is_some());
    }

    #[test]
    fn insert_rejects_name_with_control_char() {
        // validate_name 拦截 C0 控制符 — OSC 7337 注入防线
        let db = Db::open_in_memory().unwrap();
        let mut bad = mk("p1", "evil\x1b]52");
        bad.name = "evil\x1b]52".into();
        assert_eq!(
            insert(&db, &bad).unwrap_err().code(),
            "name_has_control_char"
        );
    }

    /// 应用层不变量：Profile.credential_id 永远非空。
    /// db layer 是 single source of truth —— UI/CLI 表层校验之外，任何路径
    /// （Tauri create_profile、外部脚本、未来新写入入口）走到这里都必须被拦下。
    #[test]
    fn insert_rejects_empty_credential_id() {
        let db = Db::open_in_memory().unwrap();
        let mut bad = mk("p1", "alpha");
        bad.credential_id = String::new();
        assert_eq!(
            insert(&db, &bad).unwrap_err().code(),
            "profile_credential_id_required"
        );
    }

    #[test]
    fn update_rejects_empty_credential_id() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("p1", "alpha")).unwrap();
        let mut bad = mk("p1", "alpha");
        bad.credential_id = String::new();
        assert_eq!(
            update(&db, &bad).unwrap_err().code(),
            "profile_credential_id_required"
        );
    }

    #[test]
    fn update_rejects_name_with_control_char() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("p1", "good")).unwrap();
        let mut bad = mk("p1", "good");
        bad.name = "bad\x07name".into();
        assert_eq!(
            update(&db, &bad).unwrap_err().code(),
            "name_has_control_char"
        );
    }
}
