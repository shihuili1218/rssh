//! 命令黑名单的策略层：DB ↔ sanitize 之间的薄封装，与 redact_rules 同结构。
//!
//! C 模型：const 仅作 seed 真值 + 出厂兜底（见 db::schema v14）；运行时以 DB 为准，
//! 某类无行 = 放行该类，整表皆空 = 全部放行 —— 都是用户显式删除的结果。
//! 这里只放两件 db 层不该管的事：
//!   - save 时校验命令名（坏名 fail-fast，绝不入库）
//!   - 建会话时把 DB 行物化成 sanitize::Blacklist（fail-closed）

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::{ai_command_blacklist, Db};
use crate::error::{AppError, AppResult};

use super::sanitize::{BlCategory, Blacklist};

/// 一个分类一行（前端展示单位）。`category` = `BlCategory::as_str`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryGroup {
    pub category: String,
    pub commands: Vec<String>,
}

/// 按 5 个分类分组列出，**空类也返回**（commands 为空）—— 保证前端永远渲染 5 行、
/// 顺序稳定（`BlCategory::ALL` 顺序）。
pub fn list_grouped(db: &Db) -> AppResult<Vec<CategoryGroup>> {
    let rows = ai_command_blacklist::list(db)?;
    Ok(BlCategory::ALL
        .iter()
        .map(|cat| {
            let mut commands: Vec<String> = rows
                .iter()
                .filter(|r| r.category == cat.as_str())
                .map(|r| r.name.clone())
                .collect();
            commands.sort();
            CategoryGroup {
                category: cat.as_str().to_string(),
                commands,
            }
        })
        .collect())
}

/// 整类替换：用户编辑某类的命令集合后保存。**先全量校验再写库** —— 任一命令名非法则
/// 整批拒绝、DB 不动（旧名单完好），不留半套。
pub fn replace_category(db: &Db, category: &str, names: &[String]) -> AppResult<()> {
    let cat = BlCategory::from_db_str(category).ok_or_else(|| {
        AppError::config("blacklist_unknown_category", json!({ "category": category }))
    })?;
    let mut cleaned: Vec<String> = Vec::with_capacity(names.len());
    for raw in names {
        cleaned.push(normalize_command_name(raw)?);
    }
    cleaned.sort();
    cleaned.dedup();
    ai_command_blacklist::replace_category(db, cat.as_str(), &cleaned)
}

/// 命令名校验：黑名单查的是 canonical 裸名（`rm`），不是路径 / 带参 / 带元字符的串。
///
/// **白名单字符集** `[A-Za-z0-9._+-]`：覆盖正常命令名 + `.`（source 的别名 `.`、
/// `mkfs.ext4` 这类带点命令）。任何其它字符（空白 / `/` / shell 元字符）都意味着这不是
/// 一个会被 `check_head` 命中的 bare 命令名，进黑名单等于死条目 → fail-fast 拒绝，
/// 比静默存废条目好（避免「我加了它怎么没拦住」的假象）。
fn normalize_command_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::config("blacklist_empty_name", json!({})));
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'));
    if !ok {
        return Err(AppError::config(
            "blacklist_invalid_name",
            json!({ "name": name }),
        ));
    }
    Ok(name.to_string())
}

/// 建会话用：把 DB 黑名单物化成 `Blacklist`。
///
/// **fail-closed**：DB 读失败 / 分类字符串非法 → 向上抛，让会话起步失败、用户可见，
/// 绝不退化成空 `Blacklist` 放行一切。空表（用户删空）是合法的空 `Blacklist`，与失败
/// 严格区分 —— 这是 C 模型下唯一的硬不变量。
pub fn load(db: &Db) -> AppResult<Blacklist> {
    let rows = ai_command_blacklist::list(db)?;
    let mut entries = Vec::with_capacity(rows.len());
    for r in rows {
        let cat = BlCategory::from_db_str(&r.category).ok_or_else(|| {
            AppError::config(
                "blacklist_unknown_category",
                json!({ "category": r.category, "name": r.name }),
            )
        })?;
        entries.push((r.name, cat));
    }
    Ok(Blacklist::from_entries(entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 漂移守卫：schema.rs v14 seed 进 DB 的 49 条必须与 sanitize 的 5 张 const 表
    /// （= `Blacklist::builtin()`）完全一致。改了一处忘了另一处就红 —— 把不可避免的
    /// 重复变成测试期硬错误。
    #[test]
    fn seed_matches_builtin() {
        let db = Db::open_in_memory().unwrap();
        let seeded: HashSet<(String, String)> = ai_command_blacklist::list(&db)
            .unwrap()
            .into_iter()
            .map(|r| (r.name, r.category))
            .collect();
        let builtin: HashSet<(String, String)> = Blacklist::builtin()
            .iter()
            .map(|(name, cat)| (name.to_string(), cat.as_str().to_string()))
            .collect();
        assert_eq!(
            seeded, builtin,
            "schema.rs v14 seed drifted from sanitize const tables"
        );
    }

    /// seed 后 load == builtin；删空某类后该类从 Blacklist 消失（C 模型放行）。
    #[test]
    fn load_reflects_db_then_empty_category_drops_out() {
        let db = Db::open_in_memory().unwrap();
        let bl = load(&db).unwrap();
        assert!(!bl.is_empty());
        assert_eq!(bl.len(), Blacklist::builtin().len());

        replace_category(&db, "destructive", &[]).unwrap();
        let bl = load(&db).unwrap();
        // destructive 全删 → load 出来的 Blacklist 里不再有任何 destructive 条目。
        assert!(!bl.iter().any(|(_, cat)| cat == BlCategory::Destructive));
        // 但 write_verb 等其它类仍在。
        assert!(bl.iter().any(|(_, cat)| cat == BlCategory::WriteVerb));
    }

    #[test]
    fn replace_category_validates_before_write() {
        let db = Db::open_in_memory().unwrap();
        replace_category(&db, "destructive", &["rm".into(), "frob".into()]).unwrap();

        // 批里有非法名（含空格）→ 整批拒，DB 不动。
        let err = replace_category(&db, "destructive", &["ok".into(), "rm -rf".into()])
            .unwrap_err();
        assert_eq!(err.code(), "blacklist_invalid_name");
        let d = list_grouped(&db)
            .unwrap()
            .into_iter()
            .find(|g| g.category == "destructive")
            .unwrap();
        assert_eq!(d.commands, vec!["frob".to_string(), "rm".to_string()]);

        // 路径名（含 `/`）也拒。
        assert_eq!(
            replace_category(&db, "destructive", &["/bin/rm".into()])
                .unwrap_err()
                .code(),
            "blacklist_invalid_name"
        );
        // 未知分类拒。
        assert_eq!(
            replace_category(&db, "bogus", &[]).unwrap_err().code(),
            "blacklist_unknown_category"
        );
        // 带点命令名（source 别名 `.`、mkfs.ext4）放行。
        replace_category(&db, "destructive", &[".".into(), "mkfs.ext4".into()]).unwrap();
    }

    /// 端到端（load ↔ validate_with 接缝，= session.rs:1022 的真实路径）：删空某类后
    /// 该类命令放行，其它类不受影响。session 只是把 `&cfg.blacklist` 透传给 validate_with，
    /// 所以这条覆盖了接线后的实际行为。
    #[test]
    fn end_to_end_emptying_category_allows_those_commands() {
        use super::super::sanitize::validate_with;
        let db = Db::open_in_memory().unwrap();
        // 出厂：rm 被拦。
        let bl = load(&db).unwrap();
        assert!(validate_with("rm -rf /tmp/x", &bl).is_err());
        // 用户删空 destructive → 重新 load → rm 放行（C 模型生效）。
        replace_category(&db, "destructive", &[]).unwrap();
        let bl = load(&db).unwrap();
        assert!(validate_with("rm -rf /tmp/x", &bl).is_ok());
        // write_verb 未动 → cp 仍被拦，证明只放行了被删的那一类。
        assert!(validate_with("cp a b", &bl).is_err());
    }

    /// `load` 在分类串非法时 fail-closed 上抛，绝不静默吞掉那条 / 退化成空名单。
    /// db 层的 `replace_category` 不校验分类（校验在本层），正好用它注入坏分类，
    /// 模拟外部改坏 DB 的场景。
    #[test]
    fn load_fails_closed_on_bad_category() {
        let db = Db::open_in_memory().unwrap();
        ai_command_blacklist::replace_category(&db, "bogus_cat", &["weird".to_string()])
            .unwrap();
        let err = load(&db).unwrap_err();
        assert_eq!(err.code(), "blacklist_unknown_category");
    }
}
