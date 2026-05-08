//! 堡垒机链解析。
//!
//! `Profile.bastion_profile_id` 指向**上一跳**（更外层、更靠近入口的那一跳）。
//! `resolve_chain(target)` 返回从入口到目标前一跳的完整链——
//! `chain[0]` 必须先连，最后一跳通过它再连到 `target`。
//!
//! 与 OpenSSH `-J entry,mid,...,last target` 的顺序一致。
//!
//! 防御：访问集去环，硬上限 `MAX_HOPS` 拦异常深度。

use std::collections::HashSet;

use serde_json::json;

use crate::db::{self, Db};
use crate::error::{AppError, AppResult};
use crate::models::Profile;

/// 堡垒机链最大跳数。OpenSSH 默认 `ProxyJump` 没硬限制，
/// 8 跳已远超任何合理生产场景；超过这个值大概率是数据异常。
pub const MAX_HOPS: usize = 8;

/// 解析 `target` 的堡垒机链。返回顺序：入口 → 倒数第二跳。
/// 无堡垒机返回空 `Vec`。
pub fn resolve_chain(db: &Db, target: &Profile) -> AppResult<Vec<Profile>> {
    let mut chain: Vec<Profile> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    if !target.id.is_empty() {
        visited.insert(target.id.clone());
    }

    let mut next_id = target.bastion_profile_id.clone();
    while let Some(bid) = next_id {
        if visited.contains(&bid) {
            let mut path: Vec<&str> = std::iter::once(target.name.as_str())
                .chain(chain.iter().map(|p| p.name.as_str()))
                .collect();
            // 把闭环的那一跳也拼出来便于排查
            if let Some(loop_name) = chain.iter().find(|p| p.id == bid).map(|p| p.name.as_str()) {
                path.push(loop_name);
            }
            return Err(AppError::config(
                "bastion_cycle",
                json!({ "path": path.join(" → ") }),
            ));
        }
        if chain.len() >= MAX_HOPS {
            return Err(AppError::config(
                "bastion_too_many_hops",
                json!({ "max": MAX_HOPS }),
            ));
        }
        let bp = db::profile::get(db, &bid).map_err(|e| match e {
            AppError::NotFound(_) => {
                AppError::not_found("bastion_profile_not_found", json!({ "id": &bid }))
            }
            other => other,
        })?;
        visited.insert(bid);
        next_id = bp.bastion_profile_id.clone();
        chain.push(bp);
    }
    chain.reverse();
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// 持有 tempdir 让 db 文件直到测试结束才被清理。
    fn open_test_db() -> (Db, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        (db, dir)
    }

    fn make_profile(id: &str, name: &str, bastion_id: Option<&str>) -> Profile {
        Profile {
            id: id.to_string(),
            name: name.to_string(),
            host: format!("{name}.local"),
            port: 22,
            // DB 的 credential_id 列声明 NOT NULL DEFAULT ''；model 端是 Option<String>。
            // 给个空 string 兼容 schema，不动 schema/model 之间这层小不一致。
            credential_id: Some(String::new()),
            bastion_profile_id: bastion_id.map(|s| s.to_string()),
            init_command: None,
            group_id: None,
        }
    }

    fn insert(db: &Db, p: &Profile) {
        db::profile::insert(db, p).unwrap();
    }

    #[test]
    fn no_bastion_returns_empty_chain() {
        let (db, _g) = open_test_db();
        let target = make_profile("t", "target", None);
        // target 不需要插表（resolve_chain 只查 target.bastion_profile_id 后续的）
        let chain = resolve_chain(&db, &target).unwrap();
        assert!(chain.is_empty());
    }

    #[test]
    fn single_hop_bastion() {
        let (db, _g) = open_test_db();
        let bastion = make_profile("b1", "bastion1", None);
        insert(&db, &bastion);
        let target = make_profile("t", "target", Some("b1"));
        let chain = resolve_chain(&db, &target).unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].id, "b1");
    }

    #[test]
    fn multi_hop_chain_in_entry_to_last_order() {
        // entry → mid → target
        // target.bastion = mid; mid.bastion = entry; entry.bastion = None
        // 期望 chain = [entry, mid]（先连 entry，再走 mid）
        let (db, _g) = open_test_db();
        insert(&db, &make_profile("entry", "entry", None));
        insert(&db, &make_profile("mid", "mid", Some("entry")));
        let target = make_profile("t", "target", Some("mid"));
        let chain = resolve_chain(&db, &target).unwrap();
        assert_eq!(chain.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["entry", "mid"]);
    }

    #[test]
    fn cycle_two_node_loop_detected() {
        // a.bastion = b, b.bastion = a → 环
        let (db, _g) = open_test_db();
        insert(&db, &make_profile("a", "a", Some("b")));
        insert(&db, &make_profile("b", "b", Some("a")));
        let target = make_profile("t", "target", Some("a"));
        let err = resolve_chain(&db, &target).unwrap_err();
        assert_eq!(err.code(), "bastion_cycle");
    }

    #[test]
    fn cycle_self_loop_detected() {
        // 通过中间节点指回自己：mid.bastion = mid
        let (db, _g) = open_test_db();
        insert(&db, &make_profile("mid", "mid", Some("mid")));
        let target = make_profile("t", "target", Some("mid"));
        let err = resolve_chain(&db, &target).unwrap_err();
        assert_eq!(err.code(), "bastion_cycle");
    }

    #[test]
    fn too_many_hops_detected() {
        // 起 MAX_HOPS+1 长度的链
        let (db, _g) = open_test_db();
        let n = MAX_HOPS + 2;
        // 建链：h0 ← h1 ← h2 ← ... ← h{n-1}（每个的 bastion 指向前一个）
        for i in 0..n {
            let bastion_id = if i == 0 { None } else { Some(format!("h{}", i - 1)) };
            insert(
                &db,
                &make_profile(
                    &format!("h{i}"),
                    &format!("h{i}"),
                    bastion_id.as_deref(),
                ),
            );
        }
        let target = make_profile("t", "target", Some(&format!("h{}", n - 1)));
        let err = resolve_chain(&db, &target).unwrap_err();
        assert_eq!(err.code(), "bastion_too_many_hops");
    }

    #[test]
    fn missing_bastion_id_returns_not_found_code() {
        let (db, _g) = open_test_db();
        // target 引用一个不存在的 bastion id
        let target = make_profile("t", "target", Some("ghost"));
        let err = resolve_chain(&db, &target).unwrap_err();
        assert_eq!(err.code(), "bastion_profile_not_found");
    }

    #[test]
    fn target_not_required_in_db() {
        // resolve_chain 不读 target 自身行；只读 bastion 链。
        // target.id 在 visited 起步，但不查 DB——仅当链回到 target 时才视为环。
        let (db, _g) = open_test_db();
        insert(&db, &make_profile("only", "only", None));
        let target = make_profile("ghost-target", "ghost", Some("only"));
        let chain = resolve_chain(&db, &target).unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].id, "only");
    }
}
