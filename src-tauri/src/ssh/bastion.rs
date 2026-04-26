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
            return Err(AppError::Config(format!(
                "堡垒机链存在环: {}",
                path.join(" → ")
            )));
        }
        if chain.len() >= MAX_HOPS {
            return Err(AppError::Config(format!(
                "堡垒机链超过 {} 跳，疑似配置异常",
                MAX_HOPS
            )));
        }
        let bp = db::profile::get(db, &bid)
            .map_err(|_| AppError::NotFound(format!("堡垒机 Profile '{}' 不存在", bid)))?;
        visited.insert(bid);
        next_id = bp.bastion_profile_id.clone();
        chain.push(bp);
    }
    chain.reverse();
    Ok(chain)
}
