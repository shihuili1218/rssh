//! 脱敏规则的策略层：DB ↔ 命令层之间的薄封装。
//!
//! 设计哲学：默认规则不再写死在代码里"永远生效"，而是首次建表时 seed 进 DB
//! （见 db::schema v13），之后与用户自定义规则**一视同仁** —— 没有 builtin 概念，
//! 统一增删改。空表 = 脱敏关闭，是用户的显式选择。
//!
//! 这里只放两件 db 层不该管的事：
//!   - save 时编译校验正则（坏正则 fail-fast，绝不入库）
//!   - 建会话时把 DB 规则编译成 sanitize::RedactRule

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::db::{ai_redact_rule, Db};
use crate::error::{AppError, AppResult};

use super::sanitize::RedactRule;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactRuleRecord {
    pub id: String,
    pub pattern: String,
    pub replacement: String,
}

pub fn list(db: &Db) -> AppResult<Vec<RedactRuleRecord>> {
    Ok(ai_redact_rule::list(db)?
        .into_iter()
        .map(|r| RedactRuleRecord {
            id: r.id,
            pattern: r.pattern,
            replacement: r.replacement,
        })
        .collect())
}

/// 一个正则能否**零宽匹配**（匹配空串，或只由 `^`/`$`/`\b` 等断言构成）。
/// 这类规则在 `replace_all` 时会在每个位置插入 replacement，造成灾难性的
/// over-replacement / 文本膨胀。`Regex::new` 不拦这些（它们语法合法），所以 save
/// 时单独用 regex-syntax 的 `minimum_len()` 判最小匹配长度，==0 即零宽。
fn matches_empty(pattern: &str) -> bool {
    regex_syntax::Parser::new()
        .parse(pattern)
        // parse 失败的分支走不到（上游 Regex::new 已校验过可编译）；兜底视为非零宽，
        // 不误拒一条引擎接受的规则。
        .map(|hir| hir.properties().minimum_len() == Some(0))
        .unwrap_or(false)
}

/// 保存（新增或编辑）。两道 fail-fast 校验，绝不让坏规则入库 —— 否则建会话时
/// `compiled()` 会静默跳过它，用户以为脱敏生效其实没生效，这才是真正危险的 false sense。
///   1. 正则可编译（坏语法 → redact_invalid_regex）
///   2. 非零宽匹配（`""`/`^`/`a*`/`\b` 等 → redact_zero_width_pattern）
pub fn save(db: &Db, rec: &RedactRuleRecord) -> AppResult<()> {
    RedactRule::new(&rec.pattern, &rec.replacement).map_err(|e| {
        AppError::config(
            "redact_invalid_regex",
            json!({ "pattern": rec.pattern, "error": e.to_string() }),
        )
    })?;
    if matches_empty(&rec.pattern) {
        return Err(AppError::config(
            "redact_zero_width_pattern",
            json!({ "pattern": rec.pattern }),
        ));
    }
    ai_redact_rule::upsert(
        db,
        &ai_redact_rule::RedactRuleRow {
            id: rec.id.clone(),
            pattern: rec.pattern.clone(),
            replacement: rec.replacement.clone(),
        },
    )
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    ai_redact_rule::delete(db, id)
}

/// 建会话用：把 DB 里的规则编译成 `RedactRule`，保留 list 的应用顺序。
///
/// **fail-closed**：任一规则编译失败直接向上抛。save 时已校验正则，正常路径到不了
/// 这里；走到这里 = DB 被外部改坏。这种情况**绝不静默跳过**那条规则 —— 那等于悄悄
/// 少套一条脱敏、用一套比用户配置更弱的策略。报错让会话起步失败、用户可见，再去
/// 设置页（走 list，不经本函数）删掉坏规则即可恢复。
pub fn compiled(db: &Db) -> AppResult<Vec<RedactRule>> {
    ai_redact_rule::list(db)?
        .into_iter()
        .map(|row| {
            RedactRule::new(&row.pattern, &row.replacement).map_err(|e| {
                AppError::config(
                    "redact_invalid_regex",
                    json!({ "id": row.id, "pattern": row.pattern, "error": e.to_string() }),
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::sanitize;
    use std::collections::HashSet;

    /// 漂移守卫：seed 进 DB 的 8 条默认规则必须与 sanitize::default_rules() 完全一致。
    /// 谁改了一处忘了另一处（schema.rs 的 seed SQL vs sanitize.rs 的 default_rules），
    /// 这个测试就会红 —— merge 不进去。这是处理"不可避免的重复"的正确姿势：把漂移
    /// 变成编译期/测试期的硬错误。
    #[test]
    fn seed_matches_default_rules() {
        let db = Db::open_in_memory().unwrap();
        let seeded: HashSet<(String, String)> = list(&db)
            .unwrap()
            .into_iter()
            .map(|r| (r.pattern, r.replacement))
            .collect();
        let defaults: HashSet<(String, String)> = sanitize::default_rules()
            .iter()
            .map(|r| (r.pattern.as_str().to_string(), r.replacement.clone()))
            .collect();
        assert_eq!(
            seeded, defaults,
            "seeded redact rules drifted from sanitize::default_rules()"
        );
    }

    #[test]
    fn save_rejects_invalid_regex() {
        let db = Db::open_in_memory().unwrap();
        let err = save(
            &db,
            &RedactRuleRecord {
                id: "user-bad".into(),
                pattern: "(unclosed".into(),
                replacement: "<X>".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "redact_invalid_regex");
        // 坏规则绝不入库
        assert!(!list(&db).unwrap().iter().any(|r| r.id == "user-bad"));
    }

    #[test]
    fn save_rejects_zero_width_patterns() {
        let db = Db::open_in_memory().unwrap();
        // 空串、纯锚点、可匹配空的星号、纯断言 —— 全是零宽，会灾难性 over-replace。
        for (i, pat) in ["", "^", "$", "a*", r"\b", r"x*y*"].iter().enumerate() {
            let err = save(
                &db,
                &RedactRuleRecord {
                    id: format!("user-zw-{i}"),
                    pattern: (*pat).into(),
                    replacement: "<X>".into(),
                },
            )
            .unwrap_err();
            assert_eq!(err.code(), "redact_zero_width_pattern", "pattern {pat:?}");
        }
        // 一条都没入库（仍是 8 条默认）。
        assert_eq!(list(&db).unwrap().len(), 8);
    }

    #[test]
    fn save_accepts_normal_nonempty_pattern() {
        let db = Db::open_in_memory().unwrap();
        // `a+` 最小匹配长度 1，非零宽，放行。
        save(
            &db,
            &RedactRuleRecord {
                id: "user-ok".into(),
                pattern: "a+".into(),
                replacement: "<X>".into(),
            },
        )
        .unwrap();
        assert!(list(&db).unwrap().iter().any(|r| r.id == "user-ok"));
    }

    #[test]
    fn save_then_compiled_includes_user_rule() {
        let db = Db::open_in_memory().unwrap();
        save(
            &db,
            &RedactRuleRecord {
                id: "user-emp".into(),
                pattern: r"EMP-\d{4}".into(),
                replacement: "<REDACTED:emp>".into(),
            },
        )
        .unwrap();
        let rules = compiled(&db).unwrap();
        // 8 默认 + 1 用户
        assert_eq!(rules.len(), 9);
        // 用户规则确实生效
        assert_eq!(
            sanitize::redact("staff EMP-1234 here", &rules),
            "staff <REDACTED:emp> here"
        );
    }

    #[test]
    fn empty_table_means_redaction_off() {
        let db = Db::open_in_memory().unwrap();
        for id in ["ip-10", "ip-172", "ip-192", "bearer", "sk-key", "aws-key", "jwt", "hex"] {
            delete(&db, id).unwrap();
        }
        let rules = compiled(&db).unwrap();
        assert!(rules.is_empty());
        // 空规则集 = 原样返回（passthrough）
        assert_eq!(sanitize::redact("ssh 10.0.0.1", &rules), "ssh 10.0.0.1");
    }

    /// fail-closed：DB 里若有一条编译不了的规则（只可能来自外部改坏 DB，因为 save 已
    /// 校验），compiled() 必须报错，绝不静默跳过 —— 否则会悄悄少套一条脱敏。
    #[test]
    fn compiled_fails_closed_on_uncompilable_rule() {
        let db = Db::open_in_memory().unwrap();
        // 绕过 save 校验，直接 upsert 一条坏正则。
        ai_redact_rule::upsert(
            &db,
            &ai_redact_rule::RedactRuleRow {
                id: "user-broken".into(),
                pattern: "(unclosed".into(),
                replacement: "<X>".into(),
            },
        )
        .unwrap();
        let err = compiled(&db).unwrap_err();
        assert_eq!(err.code(), "redact_invalid_regex");
    }
}
