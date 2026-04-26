//! AI 自定义 skill 的 DB 访问。内置 skill 不入表，从 ai::prompts 常量读。

use rusqlite::params;

use crate::error::AppResult;

use super::Db;

#[derive(Debug, Clone)]
pub struct UserSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub content: String,
}

pub fn list(db: &Db) -> AppResult<Vec<UserSkill>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, name, description, content FROM ai_skills ORDER BY updated_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(UserSkill {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                content: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(db: &Db, id: &str) -> AppResult<Option<UserSkill>> {
    let conn = db.lock()?;
    let res = conn
        .query_row(
            "SELECT id, name, description, content FROM ai_skills WHERE id = ?1",
            [id],
            |r| {
                Ok(UserSkill {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    content: r.get(3)?,
                })
            },
        )
        .ok();
    Ok(res)
}

pub fn upsert(db: &Db, skill: &UserSkill) -> AppResult<()> {
    let conn = db.lock()?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO ai_skills (id, name, description, content, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            description = excluded.description,
            content = excluded.content,
            updated_at = excluded.updated_at",
        params![skill.id, skill.name, skill.description, skill.content, now],
    )?;
    Ok(())
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    conn.execute("DELETE FROM ai_skills WHERE id = ?1", [id])?;
    Ok(())
}
