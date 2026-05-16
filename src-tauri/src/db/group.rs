use rusqlite::params;

use super::Db;
use crate::error::AppResult;
use crate::models::{validate_name, Group};

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

pub fn insert_tx(conn: &rusqlite::Connection, g: &Group) -> AppResult<()> {
    validate_name(&g.name)?;
    conn.execute(
        "INSERT INTO groups (id, name, color, sort_order) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, color=excluded.color, sort_order=excluded.sort_order",
        params![g.id, g.name, g.color, g.sort_order],
    )?;
    Ok(())
}

pub fn insert(db: &Db, g: &Group) -> AppResult<()> {
    let conn = db.lock()?;
    insert_tx(&conn, g)
}

pub fn update(db: &Db, g: &Group) -> AppResult<()> {
    validate_name(&g.name)?;
    let conn = db.lock()?;
    conn.execute(
        "UPDATE groups SET name=?1, color=?2, sort_order=?3 WHERE id=?4",
        params![g.name, g.color, g.sort_order, g.id],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    // 删 group + 清 profiles.group_id 必须原子。中途崩 = 残留 profile 指向已删 group。
    db.with_transaction(|tx| {
        tx.execute("DELETE FROM groups WHERE id = ?1", params![id])?;
        tx.execute(
            "UPDATE profiles SET group_id = NULL WHERE group_id = ?1",
            params![id],
        )?;
        Ok(())
    })
}

pub fn clear_all_tx(conn: &rusqlite::Connection) -> AppResult<()> {
    conn.execute("DELETE FROM groups", [])?;
    Ok(())
}

pub fn clear_all(db: &Db) -> AppResult<()> {
    let conn = db.lock()?;
    clear_all_tx(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::profile;
    use crate::models::Profile;

    fn mk_group(id: &str, name: &str) -> Group {
        Group {
            id: id.into(),
            name: name.into(),
            color: "#FF0000".into(),
            sort_order: 0,
        }
    }

    fn mk_profile(id: &str, name: &str, group_id: Option<&str>) -> Profile {
        Profile {
            id: id.into(),
            name: name.into(),
            host: "h".into(),
            port: 22,
            credential_id: String::new(),
            bastion_profile_id: None,
            init_command: None,
            group_id: group_id.map(String::from),
        }
    }

    #[test]
    fn insert_then_get_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk_group("g1", "production")).unwrap();
        let got = get(&db, "g1").unwrap();
        assert_eq!(got.name, "production");
        assert_eq!(got.color, "#FF0000");
    }

    #[test]
    fn list_sorted_by_sort_order_then_name() {
        let db = Db::open_in_memory().unwrap();
        let mut g1 = mk_group("g1", "zebra");
        g1.sort_order = 10;
        let mut g2 = mk_group("g2", "apple");
        g2.sort_order = 10;
        let mut g3 = mk_group("g3", "manual");
        g3.sort_order = 1;
        insert(&db, &g1).unwrap();
        insert(&db, &g2).unwrap();
        insert(&db, &g3).unwrap();
        let names: Vec<String> = list(&db).unwrap().into_iter().map(|g| g.name).collect();
        // sort_order 升序优先，同 sort_order 内按 name
        assert_eq!(names, vec!["manual", "apple", "zebra"]);
    }

    #[test]
    fn update_changes_fields() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk_group("g1", "old")).unwrap();
        let mut g = mk_group("g1", "old");
        g.color = "#00FF00".into();
        g.sort_order = 99;
        update(&db, &g).unwrap();
        let got = get(&db, "g1").unwrap();
        assert_eq!(got.color, "#00FF00");
        assert_eq!(got.sort_order, 99);
    }

    /// 关键不变量：删 group 时必须清掉所有指向它的 profiles.group_id，
    /// 否则会留残留 profile 指向已删 group，前端列表渲染报"未知 group"。
    #[test]
    fn delete_clears_dependent_profile_group_ids() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk_group("g1", "prod")).unwrap();
        profile::insert(&db, &mk_profile("p1", "web1", Some("g1"))).unwrap();
        profile::insert(&db, &mk_profile("p2", "web2", Some("g1"))).unwrap();
        profile::insert(&db, &mk_profile("p3", "web3", None)).unwrap();

        delete(&db, "g1").unwrap();

        assert!(profile::get(&db, "p1").unwrap().group_id.is_none());
        assert!(profile::get(&db, "p2").unwrap().group_id.is_none());
        assert!(profile::get(&db, "p3").unwrap().group_id.is_none());
    }

    #[test]
    fn insert_rejects_name_with_control_char() {
        let db = Db::open_in_memory().unwrap();
        let bad = mk_group("g1", "bad\x1bname");
        assert_eq!(
            insert(&db, &bad).unwrap_err().code(),
            "name_has_control_char"
        );
    }
}
