//! 一次性数据迁移编排。
//!
//! 每个 migration 是不可变文件，永远保留。启动时按顺序检查 `settings` 表里的
//! marker，已跑过则函数体直接跳过；没跑过则跑一次，跑完写 marker。
//!
//! 这跟"hot path 兼容代码"不同：
//!   - hot path 兼容：每次 get/set 都判断旧/新格式 → 性能负担、维护负担
//!   - 这里：startup 一次 `SELECT value FROM settings WHERE key=...`，已完成的用户
//!     等价于零成本（cmp + je）跳过
//!
//! 设计取自 Alembic / Diesel / sqitch / Rails ActiveRecord migration 运行模型。
//! 用户跨多个大版本跳升无破坏 (v1 → v5 仍能按链路依次跑完中间各 migration)。
//!
//! 调用方：lib.rs setup 闭包（GUI）+ bin/rssh ctx.secret_store() 首次访问（CLI），
//! 各调一次。Marker 在 DB 共享，所以两入口一致。

use crate::db::Db;
use crate::error::AppResult;
use crate::secret::SecretStore;

mod v1_unified_secret_storage;

/// 跑所有未完成的迁移，按顺序串行。每条独立 marker，跑过的下次直接跳过。
///
/// `raw_keyring`：老 keychain 入口（可能为 None — 当前平台没 keychain），
/// 给迁移函数读旧数据用。
/// `new_store`：新版统一 SecretStore（HybridStore，加密 DB），迁移函数写入用。
pub fn run_migrations(
    db: &Db,
    raw_keyring: Option<&dyn SecretStore>,
    new_store: &dyn SecretStore,
) -> AppResult<()> {
    v1_unified_secret_storage::run(db, raw_keyring, new_store)?;
    // 未来新 migration 在此追加：v2_*::run(...)?; ...
    Ok(())
}
