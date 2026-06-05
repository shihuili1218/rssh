//! AI 脱敏规则的 DB 访问。默认规则首次建表时 seed 进表（见 schema.rs v13），
//! 之后与用户自定义规则一视同仁 —— 没有 builtin 概念，统一增删改。

use rusqlite::params;

use crate::error::AppResult;

use super::Db;

#[derive(Debug, Clone)]
pub struct RedactRuleRow {
    pub id: String,
    pub pattern: String,
    pub replacement: String,
}

/// 按应用顺序返回（created_at 升序，id 作 tiebreak）。redact 顺序执行规则，所以
/// 顺序必须稳定且不受编辑影响 —— 用 created_at 而非 updated_at。
pub fn list(db: &Db) -> AppResult<Vec<RedactRuleRow>> {
    let conn = db.lock()?;
    let mut stmt = conn
        .prepare("SELECT id, pattern, replacement FROM ai_redact_rules ORDER BY created_at, id")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(RedactRuleRow {
                id: r.get(0)?,
                pattern: r.get(1)?,
                replacement: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 插入或更新。ON CONFLICT 只改 pattern/replacement/updated_at，**保留 created_at**
/// （= 应用顺序），所以编辑一条规则不会让它在列表里跳位。新规则用当前时间戳，
/// 必然排在 seed 默认（created_at 1..8）之后。
pub fn upsert(db: &Db, rule: &RedactRuleRow) -> AppResult<()> {
    let conn = db.lock()?;
    // 毫秒分辨率：避免同一秒内连续新增两条规则 created_at 撞值、退化成按随机 id 排序
    // （会改变重叠规则的应用顺序）。seed 默认用 1..8，新规则用 ~1.7e12 ms，必排其后。
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO ai_redact_rules (id, pattern, replacement, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(id) DO UPDATE SET
            pattern = excluded.pattern,
            replacement = excluded.replacement,
            updated_at = excluded.updated_at",
        params![rule.id, rule.pattern, rule.replacement, now],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM ai_redact_rules WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_with_eight_defaults_in_order() {
        let db = Db::open_in_memory().unwrap();
        let rules = list(&db).unwrap();
        assert_eq!(rules.len(), 8);
        // created_at 1..8 保证原始顺序：第一条 ip-10，最后一条 hex。
        assert_eq!(rules[0].id, "ip-10");
        assert_eq!(rules[7].id, "hex");
        // seed 的反斜杠原样入库（没被 Rust / SQLite 吞掉）。
        assert_eq!(rules[7].pattern, r"\b[0-9a-fA-F]{32,}\b");
        assert_eq!(rules[7].replacement, "<REDACTED:hex>");
        // AWS access key 预置规则入库正确。
        let aws = rules.iter().find(|r| r.id == "aws-key").unwrap();
        assert_eq!(aws.pattern, r"AKIA[0-9A-Z]{16}");
        assert_eq!(aws.replacement, "<REDACTED:aws-key>");
    }

    #[test]
    fn upsert_new_rule_appends_after_defaults() {
        let db = Db::open_in_memory().unwrap();
        upsert(
            &db,
            &RedactRuleRow {
                id: "user-test".into(),
                pattern: r"secret\d+".into(),
                replacement: "<X>".into(),
            },
        )
        .unwrap();
        let rules = list(&db).unwrap();
        assert_eq!(rules.len(), 9);
        assert_eq!(rules.last().unwrap().id, "user-test");
    }

    #[test]
    fn edit_default_preserves_position() {
        let db = Db::open_in_memory().unwrap();
        // 编辑 ip-10 的 replacement —— created_at 不变，仍排第一。
        upsert(
            &db,
            &RedactRuleRow {
                id: "ip-10".into(),
                pattern: r"\b10\.\d{1,3}\.\d{1,3}\.\d{1,3}\b".into(),
                replacement: "<changed>".into(),
            },
        )
        .unwrap();
        let rules = list(&db).unwrap();
        assert_eq!(rules.len(), 8);
        assert_eq!(rules[0].id, "ip-10");
        assert_eq!(rules[0].replacement, "<changed>");
    }

    #[test]
    fn delete_sticks() {
        let db = Db::open_in_memory().unwrap();
        delete(&db, "hex").unwrap();
        let rules = list(&db).unwrap();
        assert_eq!(rules.len(), 7);
        assert!(!rules.iter().any(|r| r.id == "hex"));
    }
}
