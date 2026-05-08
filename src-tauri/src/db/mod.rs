pub mod ai_skill;
pub mod credential;
pub mod forward;
pub mod group;
pub mod highlight;
pub mod profile;
pub mod schema;
pub mod secret;
pub mod settings;
pub mod snippet;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;

use crate::error::{locked, AppResult};

/// 数据库句柄。封装 Mutex<Connection>，对外只暴露领域方法（在子模块里），
/// `lock()` 仅对 `crate::db` 内部可见，禁止泄漏到 command 层。
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(data_dir: &Path) -> AppResult<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("rssh.db");
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        schema::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 仅 db 模块内部使用，用于锁住 Connection。
    pub(in crate::db) fn lock(&self) -> AppResult<MutexGuard<'_, Connection>> {
        locked(&self.conn)
    }

    /// 把一组写操作包进单个事务。闭包里调 `*_tx(&Connection, ...)` 系列，
    /// 任何错误自动回滚（tx 不 commit 即 drop = ROLLBACK）。
    /// 成功才 commit。用于"全量替换"语义（github_pull、未来 import-replace）。
    ///
    /// `pub(crate)`：只让同 crate 的 `sync::config` 等模块用，不对外暴露
    /// `rusqlite::Transaction`，避免 commands 层绕过 db 子模块的校验/不变量。
    pub(crate) fn with_transaction<F, T>(&self, f: F) -> AppResult<T>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> AppResult<T>,
    {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let result = f(&tx)?;
        tx.commit()?;
        Ok(result)
    }
}

/// 数据目录：桌面用 ~/.rssh，Android 用 app_data_dir
pub fn data_dir() -> PathBuf {
    let mut p = dirs::home_dir().expect("home directory unavailable");
    p.push(".rssh");
    p
}
