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
}

/// 数据目录：桌面用 ~/.rssh，Android 用 app_data_dir
pub fn data_dir() -> PathBuf {
    let mut p = dirs::home_dir().expect("无法获取 home 目录");
    p.push(".rssh");
    p
}
