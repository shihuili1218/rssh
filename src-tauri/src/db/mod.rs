pub mod ai_command_blacklist;
pub mod ai_conversation;
pub mod ai_redact_rule;
pub mod ai_skill;
pub mod credential;
pub mod forward;
pub mod group;
pub mod highlight;
pub mod profile;
pub mod schema;
pub mod secret;
pub mod serial_profile;
pub mod settings;
pub mod snippet;
pub mod telnet_profile;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;

use crate::error::{locked, AppError, AppResult};

/// 数据库句柄。封装 Mutex<Connection>，对外只暴露领域方法（在子模块里），
/// `lock()` 仅对 `crate::db` 内部可见，禁止泄漏到 command 层。
pub struct Db {
    conn: Mutex<Connection>,
}

const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn configure_connection(conn: &Connection) -> AppResult<()> {
    conn.busy_timeout(BUSY_TIMEOUT)?;

    // SQLite does not invoke the busy handler for every journal-mode locking
    // conflict (notably SQLITE_LOCKED during two first-time openers), so the
    // timeout must also cover a small explicit retry loop around WAL setup.
    let deadline = std::time::Instant::now() + BUSY_TIMEOUT;
    loop {
        match conn.execute_batch("PRAGMA journal_mode=WAL;") {
            Ok(()) => break,
            Err(e)
                if matches!(
                    e.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => return Err(e.into()),
        }
    }
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    Ok(())
}

impl Db {
    pub fn open(data_dir: &Path) -> AppResult<Self> {
        std::fs::create_dir_all(data_dir)?;
        let path = data_dir.join("rssh.db");
        let mut conn = Connection::open(path)?;
        // WAL: readers never block the writer. busy_timeout: three processes write
        // this same ~/.rssh/rssh.db (GUI, headless server, `rssh` CLI). Without it,
        // a write that meets a peer's lock fails *instantly* with SQLITE_BUSY
        // instead of waiting — e.g. GUI autosaving AI history while the CLI runs
        // `config pull`. 5s covers any real contention window.
        // Install the busy handler before journal_mode itself takes locks. On
        // the first concurrent open, setting WAL can otherwise fail instantly
        // before the later timeout pragma ever runs.
        configure_connection(&conn)?;
        // Schema inspection and mutation are one serialized operation across
        // GUI, server and CLI processes. Taking the write reservation before
        // migrate() reads user_version prevents two starters from planning the
        // same ALTER sequence, and makes a failed sequence roll back as a unit.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        schema::migrate(&tx)?;
        tx.commit()?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 仅 db 模块内部使用，用于锁住 Connection。
    pub(in crate::db) fn lock(&self) -> AppResult<MutexGuard<'_, Connection>> {
        locked(&self.conn)
    }

    /// 测试专用：跳过文件系统，直接开一个 in-memory SQLite 并跑完 schema migrate。
    /// 单测里每个 case 都用独立实例，互不污染。pub(crate) 让 crate 内其它模块
    /// （secret / migration 等）的测试能复用，避免每个模块自己 reimplement 一遍。
    #[cfg(test)]
    pub(crate) fn open_in_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        schema::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
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

    /// 跨进程互斥地执行 critical section。SQLite `BEGIN IMMEDIATE` 在文件层级
    /// 取 reserved-lock，其他进程的 IMMEDIATE/EXCLUSIVE 会阻塞到本事务
    /// commit/rollback；同进程其他 connection 也会阻塞。比 `with_transaction`
    /// 的默认 DEFERRED 多了"启动时立刻拿写锁"语义，专给"序列化外部副作用"
    /// 场景用（典型：master key 生成期间不能让别的进程同时跑 get→set）。
    ///
    /// 闭包内**不能再调本 Db 的其他方法**（已持 Mutex 会死锁），可以读写
    /// keychain / 文件等独立子系统。错误自动 rollback（tx drop = ROLLBACK），
    /// 成功才 commit。
    pub(crate) fn with_exclusive_lock<F, T>(&self, f: F) -> AppResult<T>
    where
        F: FnOnce() -> AppResult<T>,
    {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let result = f()?;
        tx.commit()?;
        Ok(result)
    }

    /// Flush and truncate the WAL after sensitive plaintext has been replaced.
    /// SQLite may return a successful row with `busy != 0`; treat that as a
    /// retryable failure instead of pretending the old WAL pages are gone.
    pub(crate) fn checkpoint_truncate(&self) -> AppResult<()> {
        let conn = self.lock()?;
        let (busy, _log, _checkpointed): (i64, i64, i64) =
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        if busy != 0 {
            return Err(AppError::other(
                "db_wal_checkpoint_busy",
                serde_json::json!({}),
            ));
        }
        Ok(())
    }
}

/// Data dir: desktop uses `~/.rssh`, Android uses `app_data_dir` (handled
/// by the caller via `tauri::path::PathResolver`).
///
/// Returns an error (rather than panicking) when `$HOME` is unset — common
/// in `systemd` units without `User=`, Docker `USER nobody`, or `scratch`
/// containers. The old `.expect("...")` aborted the process before the
/// CLI / GUI could surface a useful message.
pub fn data_dir() -> AppResult<PathBuf> {
    let mut p = dirs::home_dir().ok_or_else(|| {
        AppError::config(
            "no_home_dir",
            serde_json::json!({
                "hint": "HOME env var is unset; rssh cannot determine where to place ~/.rssh"
            }),
        )
    })?;
    p.push(".rssh");
    Ok(p)
}

#[cfg(test)]
mod with_exclusive_lock_tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::error::AppError;

    #[test]
    fn open_waits_for_a_concurrent_first_writer_before_enabling_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rssh.db");
        let blocker = Connection::open(path).unwrap();
        blocker
            .execute_batch(
                "CREATE TABLE open_lock (value INTEGER);\n\
                 BEGIN IMMEDIATE;\n\
                 INSERT INTO open_lock VALUES (1);",
            )
            .unwrap();

        let data_dir = dir.path().to_owned();
        let (started_tx, started_rx) = mpsc::channel();
        let opener = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            Db::open(&data_dir)
        });
        started_rx.recv().unwrap();
        // Give Db::open enough time to reach the WAL pragma while the other
        // connection still owns the write reservation.
        std::thread::sleep(Duration::from_millis(100));
        blocker.execute_batch("COMMIT").unwrap();

        opener.join().unwrap().unwrap();
    }

    #[test]
    fn failed_schema_migration_rolls_back_every_schema_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rssh.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE telnet_profiles (id TEXT PRIMARY KEY);\n\
             PRAGMA user_version = 23;",
        )
        .unwrap();
        drop(conn);

        // Migration 24 adds columns before it creates a trigger that requires
        // local_echo. This deliberately malformed pre-v24 table makes the
        // trigger creation fail after those ALTER TABLE statements.
        assert!(Db::open(dir.path()).is_err());

        let conn = Connection::open(path).unwrap();
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('telnet_profiles')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(version, 23);
        assert_eq!(columns, vec!["id"]);
    }

    #[test]
    fn ok_branch_returns_value() {
        let db = Db::open_in_memory().unwrap();
        let r: i32 = db.with_exclusive_lock(|| Ok(42)).unwrap();
        assert_eq!(r, 42);
    }

    #[test]
    fn err_branch_propagates() {
        // 闭包 Err → 整个事务 drop（自动 ROLLBACK），错误透传出去。
        // 注：闭包内不能再调 db.* 方法（会死锁），所以这里只验证 Err 流转，
        // ROLLBACK 由 rusqlite::Transaction 的 Drop 语义保证（无需手动测）。
        let db = Db::open_in_memory().unwrap();
        let r: AppResult<i32> =
            db.with_exclusive_lock(|| Err(AppError::other("intentional", serde_json::json!({}))));
        let err = r.unwrap_err();
        assert_eq!(err.code(), "intentional");
    }

    #[test]
    fn lock_released_after_return() {
        // 第二次 call 必须能拿到锁；如果释放有问题这里会 hang 或失败。
        let db = Db::open_in_memory().unwrap();
        db.with_exclusive_lock(|| Ok(())).unwrap();
        db.with_exclusive_lock(|| Ok(())).unwrap();
    }

    #[test]
    fn lock_released_after_err() {
        // 失败路径也要释放锁；不然第二次 call 会 hang。
        let db = Db::open_in_memory().unwrap();
        let _ = db.with_exclusive_lock(|| -> AppResult<()> {
            Err(AppError::other("first", serde_json::json!({})))
        });
        // 锁已释放：第二次 call 立刻拿得到
        db.with_exclusive_lock(|| Ok(())).unwrap();
    }
}
