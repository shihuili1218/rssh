//! AI 命令黑名单的 DB 访问。出厂默认首次建表时 seed 进表（见 schema.rs v14），
//! 之后无 builtin 概念，统一 CRUD —— 与 ai_redact_rule（v13）同模型。
//!
//! `name` 是 PRIMARY KEY：一个命令只可能属于一类（5 张表语义互斥），DB 层就把这条
//! 不变量钉死。某类无行 = 放行该类，整表皆空 = 全部放行 —— 都是用户显式删除的合法状态。

use rusqlite::params;

use crate::error::AppResult;

use super::Db;

#[derive(Debug, Clone)]
pub struct BlacklistRow {
    pub name: String,
    pub category: String,
}

/// 按 (category, name) 升序返回，顺序稳定（黑名单是 set 语义，顺序只为展示确定性）。
pub fn list(db: &Db) -> AppResult<Vec<BlacklistRow>> {
    let conn = db.lock()?;
    let mut stmt =
        conn.prepare("SELECT name, category FROM ai_command_blacklist ORDER BY category, name")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(BlacklistRow {
                name: r.get(0)?,
                category: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 整类原子替换：删掉该类全部行，再插入新集合。一个事务，避免中途失败留下半套名单。
/// `names` 必须由上层（`ai::command_blacklist`）校验 + 去重后传入。
///
/// `INSERT OR REPLACE`：若某命令名原本属于别的分类，会被移动到本类 —— 符合「一个命令
/// 只属一类」的不变量。
pub fn replace_category(db: &Db, category: &str, names: &[String]) -> AppResult<()> {
    let mut conn = db.lock()?;
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM ai_command_blacklist WHERE category = ?1",
        [category],
    )?;
    for name in names {
        tx.execute(
            "INSERT OR REPLACE INTO ai_command_blacklist (name, category) VALUES (?1, ?2)",
            params![name, category],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_then_replace_category_is_atomic() {
        let db = Db::open_in_memory().unwrap();
        // seed 后 destructive 类含 rm。
        let rows = list(&db).unwrap();
        assert!(rows
            .iter()
            .any(|r| r.name == "rm" && r.category == "destructive"));

        // 整类替换：destructive 只留 rm + 新增 frob。
        replace_category(&db, "destructive", &["rm".into(), "frob".into()]).unwrap();
        let d: Vec<_> = list(&db)
            .unwrap()
            .into_iter()
            .filter(|r| r.category == "destructive")
            .map(|r| r.name)
            .collect();
        assert_eq!(d, vec!["frob".to_string(), "rm".to_string()]);

        // 其它类不受影响（write_verb 仍在）。
        assert!(list(&db).unwrap().iter().any(|r| r.category == "write_verb"));
    }

    #[test]
    fn replace_with_empty_clears_category() {
        let db = Db::open_in_memory().unwrap();
        replace_category(&db, "destructive", &[]).unwrap();
        assert!(!list(&db)
            .unwrap()
            .iter()
            .any(|r| r.category == "destructive"));
    }
}
