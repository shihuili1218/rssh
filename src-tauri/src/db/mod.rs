pub mod credential;
pub mod forward;
pub mod highlight;
pub mod profile;
pub mod schema;
pub mod settings;
pub mod snippet;

use std::path::Path;

use rusqlite::Connection;

use crate::error::AppResult;

/// 数据目录：桌面用 ~/.rssh，Android 用 app_data_dir
pub fn data_dir() -> std::path::PathBuf {
    let mut p = dirs::home_dir().expect("无法获取 home 目录");
    p.push(".rssh");
    p
}

pub fn open(data_dir: &Path) -> AppResult<Connection> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("rssh.db");
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    schema::migrate(&conn)?;
    Ok(conn)
}
