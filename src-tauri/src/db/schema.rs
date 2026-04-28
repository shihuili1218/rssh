use rusqlite::Connection;

use crate::error::AppResult;

const SCHEMA_VERSION: u32 = 12;

pub fn migrate(conn: &Connection) -> AppResult<()> {
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0);

    if version < 9 {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS credentials (
                id             TEXT PRIMARY KEY,
                name           TEXT NOT NULL,
                username       TEXT NOT NULL,
                type           TEXT NOT NULL DEFAULT 'none',
                secret         TEXT NOT NULL DEFAULT '',
                save_to_remote INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS profiles (
                id                 TEXT PRIMARY KEY,
                name               TEXT NOT NULL UNIQUE COLLATE NOCASE,
                host               TEXT NOT NULL,
                port               INTEGER NOT NULL DEFAULT 22,
                credential_id      TEXT NOT NULL DEFAULT '',
                bastion_profile_id TEXT,
                init_command       TEXT
            );

            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS forwards (
                id           TEXT PRIMARY KEY,
                name         TEXT NOT NULL UNIQUE COLLATE NOCASE,
                profile_id   TEXT NOT NULL,
                type         TEXT NOT NULL DEFAULT 'local',
                local_port   INTEGER NOT NULL,
                remote_host  TEXT NOT NULL,
                remote_port  INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS highlights (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                keyword TEXT NOT NULL,
                color   TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1
            );
            ",
        )?;

        // Seed default highlights if table is empty
        let count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM highlights", [], |r| r.get(0),
        ).unwrap_or(0);
        if count == 0 {
            conn.execute_batch(
                "
                INSERT INTO highlights (keyword, color, enabled) VALUES ('ERROR', '#FF6B6B', 1);
                INSERT INTO highlights (keyword, color, enabled) VALUES ('WARN', '#FFD060', 1);
                INSERT INTO highlights (keyword, color, enabled) VALUES ('INFO', '#6EDAA0', 1);
                INSERT INTO highlights (keyword, color, enabled) VALUES ('DEBUG', '#40C8E0', 1);
                "
            )?;
        }
    }

    if version < 10 {
        // Passphrase column for credentials
        let _ = conn.execute_batch(
            "ALTER TABLE credentials ADD COLUMN passphrase TEXT NOT NULL DEFAULT '';"
        );
        // Profile groups table
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS groups (
                id         TEXT PRIMARY KEY,
                name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
                color      TEXT NOT NULL DEFAULT '#4A6CF7',
                sort_order INTEGER NOT NULL DEFAULT 0
            );
            "
        )?;
        // group_id column on profiles
        let _ = conn.execute_batch(
            "ALTER TABLE profiles ADD COLUMN group_id TEXT DEFAULT NULL;"
        );
    }

    if version < 11 {
        // 把 secret/passphrase 从 credentials 表移除，统一走 secrets 表（或系统 keychain）
        let _ = conn.execute_batch("ALTER TABLE credentials DROP COLUMN secret;");
        let _ = conn.execute_batch("ALTER TABLE credentials DROP COLUMN passphrase;");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS secrets (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );"
        )?;
    }

    if version < 12 {
        // AI 自定义 skill 表（内置 5 个 skill 不入表，从 prompts.rs 直接读）
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS ai_skills (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                content     TEXT NOT NULL,
                created_at  INTEGER NOT NULL DEFAULT 0,
                updated_at  INTEGER NOT NULL DEFAULT 0
            );
            ",
        )?;
    }

    if version < SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }

    Ok(())
}
