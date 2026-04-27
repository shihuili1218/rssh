//! Skill 管理：内置 5 个（include_str! 内嵌）+ 用户自定义（DB ai_skills 表）。
//! 内置 skill 不可改不可删；用户自定义完全可控。

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

/// 内置 skill 元数据 + 内容来源。
const BUILTIN: &[(&str, &str, &str, &str)] = &[
    (
        "general",
        "通用对话",
        "默认 skill，LLM 自己根据用户问题路由到 CPU/内存场景",
        super::prompts::GENERAL,
    ),
    (
        "cpu-java",
        "CPU 高 — Java",
        "Java 进程 CPU 排障：jstack/jstat/async-profiler",
        super::prompts::CPU_JAVA,
    ),
    (
        "cpu-go",
        "CPU 高 — Go",
        "Go 进程 CPU 排障：pprof endpoint + perf 兜底",
        super::prompts::CPU_GO,
    ),
    (
        "mem-java",
        "内存高 — Java",
        "Java 进程内存排障：jstat/jmap -histo/heap dump",
        super::prompts::MEM_JAVA,
    ),
    (
        "mem-go",
        "内存高 — Go",
        "Go 进程内存排障：pprof heap inuse_space",
        super::prompts::MEM_GO,
    ),
];

pub fn list_all(db: &Db) -> AppResult<Vec<SkillRecord>> {
    let mut out: Vec<SkillRecord> = BUILTIN
        .iter()
        .map(|(id, name, desc, content)| SkillRecord {
            id: (*id).to_string(),
            name: (*name).to_string(),
            description: (*desc).to_string(),
            content: (*content).to_string(),
            builtin: true,
        })
        .collect();
    for u in ai_skill::list(db)? {
        out.push(SkillRecord {
            id: u.id,
            name: u.name,
            description: u.description,
            content: u.content,
            builtin: false,
        });
    }
    Ok(out)
}

pub fn get(db: &Db, id: &str) -> AppResult<Option<SkillRecord>> {
    if let Some(b) = BUILTIN.iter().find(|t| t.0 == id) {
        return Ok(Some(SkillRecord {
            id: b.0.to_string(),
            name: b.1.to_string(),
            description: b.2.to_string(),
            content: b.3.to_string(),
            builtin: true,
        }));
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
    BUILTIN.iter().any(|t| t.0 == id)
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

/// 构造会话启动用的 catalog prompt：通用规则 + 各 skill 的 description（不含详细 content）。
/// LLM 看到 skill 目录，按需用 `load_skill` 工具加载具体场景的完整工作流。
/// 模式参考 Anthropic Skills —— lazy load 节省启动 token。
pub fn build_catalog_prompt(db: &Db) -> AppResult<String> {
    let all = list_all(db)?;
    let mut s = String::new();
    s.push_str("你是运维排障助手，跑在 Linux / macOS / *BSD 上都行。具体场景的完整工作流通过 `load_skill` 工具按需加载。\n\n");

    s.push_str("## 通用边界（任何场景都适用）\n\n");
    s.push_str("- **只诊断不修复**。绝不 propose 破坏性命令（kill / rm / dd / mkfs / iptables / shutdown / reboot / chmod -R 等）；rssh 的 shape validator 也会兜底拦截。\n");
    s.push_str("- **每条命令都会先经用户点击确认才执行**。`explain`（含义）和 `side_effect`（副作用）必须诚实。\n");
    s.push_str("- **状态歧义就问用户**：多 PID 让用户选；不确定哪个端口跑了 pprof 让用户协助；不替用户猜。\n");
    s.push_str("- **第一步永远是探查环境**：`uname -s` / `cat /etc/os-release` / `which <工具>`。\n");
    s.push_str("- **不用刷屏命令**：不用 `top`（用 `top -bn1` 或 `top -l 1`）、`htop`、`watch`、`tail -f`。\n");
    s.push_str("- **重复采样必须显式带次数**：`vmstat 1 5` 而非 `vmstat 1`。\n");
    s.push_str("- **重数据本地预聚合**：火焰图必须用 folded format；jstack 多次采样自己聚合 top-20。\n");
    s.push_str("- **工具未安装时引导用户安装**，不替用户装。\n\n");

    s.push_str("## 可用工具\n\n");
    s.push_str("- `run_command(cmd, explain, side_effect, timeout_s)` —— 在用户的终端执行一条命令\n");
    s.push_str("- `download_file(remote_path, max_mb)` —— SFTP 拉远端文件到用户本机（dump / pprof / perf.data 等）。经跳板手动 ssh 进去时可能失败，会让你引导用户手动 scp。\n");
    s.push_str("- `analyze_locally(local_path, task)` —— 开新窗口 + 本地 shell + 独立 AI 会话分析下载的文件。本会话拿不到结果，只在远端跑分析会和被诊断进程抢资源时才用。\n");
    s.push_str("- `load_skill(id)` —— 加载某个 skill 的完整工作流文档；判断场景后**第一时间**调用。\n\n");

    s.push_str("## 可用 skill 目录\n\n");
    s.push_str("用户描述问题后，你判断场景 → 用 `load_skill(<id>)` 拉取对应详细工作流 → 按工作流执行。\n\n");
    for r in &all {
        let tag = if r.builtin { "" } else { " [用户自定义]" };
        s.push_str(&format!(
            "- **{}** (id: `{}`){} — {}\n",
            r.name, r.id, tag,
            if r.description.is_empty() { "（无描述）" } else { &r.description }
        ));
    }
    s.push_str("\n如果场景不明朗（用户问题模糊），先问用户澄清；如果确实只是通用聊天，加载 `general`。\n");
    Ok(s)
}
