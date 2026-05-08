//! 配置同步：merge（增量合并）与 replace（事务替换）两种语义。
//!
//! - **merge_import**：文件 import / `rssh config import` —— 不清空本地，按 id
//!   upsert，已有同 id 实体被覆盖，本地独有实体保留。secret 为 None 或空时
//!   保留本地原 secret，避免推过去再拉回来时丢密码。错误逐项收集，不影响其他。
//!
//! - **replace_import**：`github_pull` / `rssh config pull` —— 事务包住
//!   clear+insert，任何步骤失败整体回滚。SecretStore 不在事务内，按
//!   原行为先清旧 secret 再写新的。
//!
//! 两者均接受同一 `serde_json::Value` 形态：
//! ```json
//! { "version": 1, "profiles": [..], "credentials": [..],
//!   "forwards": [..], "groups": [..], "skills": [..] }
//! ```

use serde_json::{json, Value};

use crate::db::{ai_skill, credential, forward, group, profile, Db};
use crate::error::{AppError, AppResult};
use crate::models::{Credential, Forward, Group, Profile};
use crate::secret::{cred_secret_key, SecretStore};

/// 失败项的结构化记录。前端只展示首条，但内部保留全量供日志诊断。
#[derive(Debug, Clone)]
pub struct ImportError {
    pub kind: &'static str,
    pub name: Option<String>,
    pub code: String,
}

impl ImportError {
    fn into_json(self) -> Value {
        json!({
            "kind": self.kind,
            "name": self.name.unwrap_or_default(),
            "code": self.code,
        })
    }
}

fn first_failure(errs: Vec<ImportError>) -> AppError {
    let count = errs.len();
    let first = errs
        .into_iter()
        .next()
        .map(|e| e.into_json())
        .unwrap_or(json!({}));
    AppError::other(
        "import_partial_failed",
        json!({
            "count": count,
            "first_kind": first.get("kind").cloned().unwrap_or(json!("?")),
            "first_name": first.get("name").cloned().unwrap_or(json!("?")),
            "first_code": first.get("code").cloned().unwrap_or(json!("?")),
        }),
    )
}

// ---------------------------------------------------------------------------
// merge_import — 增量合并
// ---------------------------------------------------------------------------

/// 不清空本地数据。每条尝试 upsert，单条失败不影响其他。
/// 返回 Ok 即全成功；返回 Err 只携带"首条失败"信息（与原 apply_import 风格一致）。
pub fn merge_import(db: &Db, ss: &dyn SecretStore, data: &Value) -> AppResult<()> {
    let mut errors: Vec<ImportError> = Vec::new();

    if let Some(arr) = data["credentials"].as_array() {
        for item in arr {
            match serde_json::from_value::<Credential>(item.clone()) {
                Ok(c) => {
                    if let Err(e) = credential::insert(db, &c) {
                        errors.push(ImportError {
                            kind: "credential",
                            name: Some(c.name.clone()),
                            code: e.code().to_string(),
                        });
                        continue;
                    }
                    // merge 语义：仅当 import 显式带非空 secret 才写入；
                    // 否则保留本地（避免 push 时被清空的 secret 覆盖回 None）。
                    if let Some(s) = c.secret.as_deref().filter(|s| !s.is_empty()) {
                        if let Err(e) = ss.set(&cred_secret_key(&c.id), s) {
                            errors.push(ImportError {
                                kind: "credential_secret",
                                name: Some(c.name),
                                code: e.code().to_string(),
                            });
                        }
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "credential",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    if let Some(arr) = data["profiles"].as_array() {
        for item in arr {
            match serde_json::from_value::<Profile>(item.clone()) {
                Ok(p) => {
                    if let Err(e) = profile::insert(db, &p) {
                        errors.push(ImportError {
                            kind: "profile",
                            name: Some(p.name),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "profile",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    if let Some(arr) = data["forwards"].as_array() {
        for item in arr {
            match serde_json::from_value::<Forward>(item.clone()) {
                Ok(f) => {
                    if let Err(e) = forward::insert(db, &f) {
                        errors.push(ImportError {
                            kind: "forward",
                            name: Some(f.name),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "forward",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    if let Some(arr) = data["groups"].as_array() {
        for item in arr {
            match serde_json::from_value::<Group>(item.clone()) {
                Ok(g) => {
                    if let Err(e) = group::insert(db, &g) {
                        errors.push(ImportError {
                            kind: "group",
                            name: Some(g.name),
                            code: e.code().to_string(),
                        });
                    }
                }
                Err(_) => errors.push(ImportError {
                    kind: "group",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }
    // skills：merge 语义同样按 id upsert；merge 不清空本地（即使 payload 带 skills:[]）
    if let Some(arr) = data
        .get("skills")
        .filter(|v| !v.is_null())
        .and_then(Value::as_array)
    {
        for item in arr {
            match parse_skill(item) {
                Ok(Some(s)) => {
                    if let Err(e) = ai_skill::upsert(db, &s) {
                        errors.push(ImportError {
                            kind: "skill",
                            name: Some(s.id),
                            code: e.code().to_string(),
                        });
                    }
                }
                Ok(None) => {} // builtin skip
                Err(_) => errors.push(ImportError {
                    kind: "skill",
                    name: None,
                    code: "parse_failed".into(),
                }),
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(first_failure(errors))
    }
}

// ---------------------------------------------------------------------------
// replace_import — 事务全量替换
// ---------------------------------------------------------------------------

/// 全量替换本地配置，DB 部分包在单一 transaction 里：任一插入失败整体回滚。
/// SecretStore 不属于 SQL 事务范围 —— 顺序：(1) parse all (2) DB tx (3) secret 同步。
/// **关键**：旧 secret 的清理在 DB tx **之后**进行——tx 失败时本地 secrets
/// 不动，与 DB 一起完整回滚到旧状态；tx 成功后才清理"被新配置淘汰"的旧 cred secret。
pub fn replace_import(db: &Db, ss: &dyn SecretStore, data: &Value) -> AppResult<()> {
    // (1) 先解析所有条目，全部解析成功才进事务。任何 parse 失败 → 早 fail，不动 DB。
    let creds: Vec<Credential> = parse_array(data, "credentials")?;
    let profiles: Vec<Profile> = parse_array(data, "profiles")?;
    let forwards: Vec<Forward> = parse_array(data, "forwards")?;
    let groups: Vec<Group> = parse_array(data, "groups")?;
    // skills 字段可缺省（v1 老 payload）。缺省 → 不动 ai_skills 表。
    let skills_present = data.get("skills").filter(|v| !v.is_null()).is_some();
    let skills: Vec<ai_skill::UserSkill> = if skills_present {
        parse_skills(data)?
    } else {
        Vec::new()
    };

    // 旧 cred id 列表 —— 必须在 tx 前抓快照（tx 后表已清空，list 就是新的了）。
    // 留作 tx 成功后清理被淘汰 cred secret 用。
    let old_cred_ids: Vec<String> = credential::list(db)
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.id)
        .collect();

    // (2) DB 事务：clear + insert 整体原子。失败回滚，secrets 不动。
    db.with_transaction(|tx| {
        credential::clear_all_tx(tx)?;
        profile::clear_all_tx(tx)?;
        forward::clear_all_tx(tx)?;
        group::clear_all_tx(tx)?;
        if skills_present {
            ai_skill::clear_all_tx(tx)?;
        }

        for c in &creds {
            credential::insert_tx(tx, c)?;
        }
        for p in &profiles {
            profile::insert_tx(tx, p)?;
        }
        for f in &forwards {
            forward::insert_tx(tx, f)?;
        }
        for g in &groups {
            group::insert_tx(tx, g)?;
        }
        for s in &skills {
            ai_skill::upsert_tx(tx, s)?;
        }
        Ok(())
    })?;

    // (3) DB 已 commit。处理 SecretStore（非事务范围）：
    //   a. 先删被淘汰的旧 cred secret（new 列表里没有的）
    //   b. 再写每条新 cred 的 secret（None / 空 → delete 该 key）
    // 顺序换一下也行，但先删再写更接近"全量替换"语义。
    let new_ids: std::collections::HashSet<&str> = creds.iter().map(|c| c.id.as_str()).collect();
    for old_id in &old_cred_ids {
        if !new_ids.contains(old_id.as_str()) {
            let _ = ss.delete(&cred_secret_key(old_id));
        }
    }

    let mut errors: Vec<ImportError> = Vec::new();
    for c in &creds {
        let sk = cred_secret_key(&c.id);
        let res = match c.secret.as_deref() {
            Some(s) if !s.is_empty() => ss.set(&sk, s),
            _ => ss.delete(&sk),
        };
        if let Err(e) = res {
            errors.push(ImportError {
                kind: "credential_secret",
                name: Some(c.name.clone()),
                code: e.code().to_string(),
            });
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(first_failure(errors))
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn parse_array<T: for<'de> serde::Deserialize<'de>>(data: &Value, key: &str) -> AppResult<Vec<T>> {
    let Some(arr) = data[key].as_array() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let parsed = serde_json::from_value::<T>(item.clone()).map_err(|e| {
            AppError::config(
                "import_parse_failed",
                json!({
                    "field": key,
                    "index": i,
                    "err": e.to_string(),
                }),
            )
        })?;
        out.push(parsed);
    }
    Ok(out)
}

fn parse_skill(item: &Value) -> AppResult<Option<ai_skill::UserSkill>> {
    use crate::ai::skills::SkillRecord;
    let s: SkillRecord = serde_json::from_value(item.clone()).map_err(|e| {
        AppError::config(
            "import_parse_failed",
            json!({ "field": "skills", "err": e.to_string() }),
        )
    })?;
    if s.builtin {
        return Ok(None);
    }
    Ok(Some(ai_skill::UserSkill {
        id: s.id,
        name: s.name,
        description: s.description,
        content: s.content,
    }))
}

fn parse_skills(data: &Value) -> AppResult<Vec<ai_skill::UserSkill>> {
    let Some(arr) = data
        .get("skills")
        .filter(|v| !v.is_null())
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in arr {
        if let Some(s) = parse_skill(item)? {
            out.push(s);
        }
    }
    Ok(out)
}

