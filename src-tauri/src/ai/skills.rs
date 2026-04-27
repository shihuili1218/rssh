//! Skill 管理：1 个内置（`general`，include_str! 内嵌）+ 用户自定义（DB ai_skills 表）。
//! 内置不可改不可删；用户自定义完全可控。
//!
//! 设计哲学：skill 是规则集，不是命令脚本——LLM 自己挑命令。所以一份 general 装下所有场景，
//! 按场景路由 + lazy-load 已经被砍掉。用户加 user-skill 时直接拼到 system prompt 末尾。

use serde::{Deserialize, Serialize};

use crate::db::{ai_skill, Db};
use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub content: String,
    pub builtin: bool,
}

const BUILTIN_ID: &str = "general";
const BUILTIN_NAME: &str = "General Ops diagnosis";
const BUILTIN_DESC: &str =
    "Default rule set + workflow reference for CPU / memory / general triage. The LLM picks commands itself.";

fn builtin_record() -> SkillRecord {
    SkillRecord {
        id: BUILTIN_ID.into(),
        name: BUILTIN_NAME.into(),
        description: BUILTIN_DESC.into(),
        content: super::prompts::GENERAL.into(),
        builtin: true,
    }
}

pub fn list_all(db: &Db) -> AppResult<Vec<SkillRecord>> {
    let mut out = vec![builtin_record()];
    for u in list_user(db)? {
        out.push(u);
    }
    Ok(out)
}

/// 仅返回用户自定义 skill（不含 builtin general）。给会话启动时 snapshot user-skill cache 用。
pub fn list_user(db: &Db) -> AppResult<Vec<SkillRecord>> {
    Ok(ai_skill::list(db)?
        .into_iter()
        .map(|u| SkillRecord {
            id: u.id,
            name: u.name,
            description: u.description,
            content: u.content,
            builtin: false,
        })
        .collect())
}

pub fn get(db: &Db, id: &str) -> AppResult<Option<SkillRecord>> {
    if id == BUILTIN_ID {
        return Ok(Some(builtin_record()));
    }
    Ok(ai_skill::get(db, id)?.map(|u| SkillRecord {
        id: u.id,
        name: u.name,
        description: u.description,
        content: u.content,
        builtin: false,
    }))
}

pub fn is_builtin(id: &str) -> bool {
    id == BUILTIN_ID
}

pub fn save_user(db: &Db, rec: &SkillRecord) -> AppResult<()> {
    if is_builtin(&rec.id) {
        return Err(crate::error::AppError::coded(
            "skill_builtin_readonly",
            serde_json::json!({ "id": rec.id }),
        ));
    }
    ai_skill::upsert(
        db,
        &ai_skill::UserSkill {
            id: rec.id.clone(),
            name: rec.name.clone(),
            description: rec.description.clone(),
            content: rec.content.clone(),
        },
    )
}

pub fn delete_user(db: &Db, id: &str) -> AppResult<()> {
    if is_builtin(id) {
        return Err(crate::error::AppError::coded(
            "skill_builtin_undeletable",
            serde_json::json!({ "id": id }),
        ));
    }
    ai_skill::delete(db, id)
}

/// 构造会话启动用的 system prompt：
/// - builtin general 规则集 **直接展开**（永远在 prompt 里）
/// - user-skill 列表 **只放 id + description**（catalog 形态），LLM 用 `load_skill(<id>)`
///   工具按需加载详细内容——claude skills 模式，用户写多个 skill 启动 prompt 不爆炸。
///
/// `user_locale_label` 是给 LLM 的回复语言提示（如 "English"、"Chinese (Simplified)"），
/// 由 commands 层根据前端 UI locale 解析后传入。
pub fn build_catalog_prompt(db: &Db, user_locale_label: &str) -> AppResult<String> {
    let mut s = String::new();
    s.push_str(super::prompts::GENERAL);

    let user_skills = ai_skill::list(db)?;
    if !user_skills.is_empty() {
        s.push_str("\n\n---\n\n# User-defined skills (catalog)\n\n");
        s.push_str(
            "The user has defined the following extra skills. \
             Each entry is just an id + one-line description; \
             when a user-skill matches the current problem, call `load_skill(<id>)` to pull its full content, then follow it.\n\n",
        );
        for u in user_skills {
            let desc = if u.description.is_empty() {
                "(no description)"
            } else {
                &u.description
            };
            s.push_str(&format!("- **{}** (id: `{}`) — {}\n", u.name, u.id, desc));
        }
    }

    s.push_str(&format!(
        "\n---\n\n# Response language\n\nRespond to the user in {user_locale_label}. Keep tool-call arguments (cmd, explain, side_effect, etc.) consistent with the user's language too — those are also user-facing.\n"
    ));

    Ok(s)
}
