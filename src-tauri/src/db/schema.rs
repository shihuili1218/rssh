use rusqlite::Connection;

use crate::error::AppResult;

const SCHEMA_VERSION: u32 = 13;

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
        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM highlights", [], |r| r.get(0))
            .unwrap_or(0);
        if count == 0 {
            conn.execute_batch(
                "
                INSERT INTO highlights (keyword, color, enabled) VALUES ('ERROR', '#FF6B6B', 1);
                INSERT INTO highlights (keyword, color, enabled) VALUES ('WARN', '#FFD060', 1);
                INSERT INTO highlights (keyword, color, enabled) VALUES ('INFO', '#6EDAA0', 1);
                INSERT INTO highlights (keyword, color, enabled) VALUES ('DEBUG', '#40C8E0', 1);
                ",
            )?;
        }
    }

    if version < 10 {
        // Passphrase column for credentials
        let _ = conn.execute_batch(
            "ALTER TABLE credentials ADD COLUMN passphrase TEXT NOT NULL DEFAULT '';",
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
            ",
        )?;
        // group_id column on profiles
        let _ = conn.execute_batch("ALTER TABLE profiles ADD COLUMN group_id TEXT DEFAULT NULL;");
    }

    if version < 11 {
        // 把 secret/passphrase 从 credentials 表移除，统一走 secrets 表（或系统 keychain）
        let _ = conn.execute_batch("ALTER TABLE credentials DROP COLUMN secret;");
        let _ = conn.execute_batch("ALTER TABLE credentials DROP COLUMN passphrase;");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS secrets (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
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

    if version < 13 {
        // AI 脱敏规则表。设计取舍：默认规则不再写死在代码里"永远生效"，而是首次建表时
        // seed 进表，之后与用户自定义规则一视同仁，统一增删改 —— 消除 builtin 特殊情况。
        //
        // 这个块一辈子只跑一次（user_version 跨过 13 后不再进），所以用户删掉某条默认规则
        // 不会在下次启动时被复活。空表 = 脱敏关闭，是用户的显式选择（等同 danger mode）。
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS ai_redact_rules (
                id          TEXT PRIMARY KEY,
                pattern     TEXT NOT NULL,
                replacement TEXT NOT NULL,
                created_at  INTEGER NOT NULL DEFAULT 0,
                updated_at  INTEGER NOT NULL DEFAULT 0
            );
            ",
        )?;

        // 仅在表为空时 seed（防御性，建表块本就只跑一次）。
        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM ai_redact_rules", [], |r| r.get(0))
            .unwrap_or(0);
        if count == 0 {
            // 8 条默认规则与 ai::sanitize::default_rules() 必须保持同步，由
            // ai::redact_rules 的漂移守卫单测把关（改一处忘改另一处 = 红灯）。
            // created_at = 1..8 保留 default_rules() 的原始应用顺序；用户新规则用
            // 毫秒时间戳（~1.7e12 ms，见 ai_redact_rule::upsert）必然排在默认之后。
            // raw string `r"..."`：pattern 里的正则反斜杠不被 Rust 转义；SQLite
            // 单引号字面量不处理反斜杠，原样入库。
            conn.execute_batch(
                r"
                INSERT INTO ai_redact_rules (id, pattern, replacement, created_at, updated_at) VALUES
                  ('ip-10',   '\b10\.\d{1,3}\.\d{1,3}\.\d{1,3}\b',                                '<REDACTED:ip-10>',   1, 1),
                  ('ip-172',  '\b172\.(1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3}\b',                     '<REDACTED:ip-172>',  2, 2),
                  ('ip-192',  '\b192\.168\.\d{1,3}\.\d{1,3}\b',                                    '<REDACTED:ip-192>',  3, 3),
                  ('bearer',  'Bearer [A-Za-z0-9_\-\.]{20,}',                                      '<REDACTED:bearer>',  4, 4),
                  ('sk-key',  'sk-[A-Za-z0-9_\-]{20,}',                                            '<REDACTED:sk-key>',  5, 5),
                  ('aws-key', 'AKIA[0-9A-Z]{16}',                                                  '<REDACTED:aws-key>', 6, 6),
                  ('jwt',     'eyJ[A-Za-z0-9_\-]{20,}\.[A-Za-z0-9_\-]{20,}\.[A-Za-z0-9_\-]+',      '<REDACTED:jwt>',     7, 7),
                  ('hex',     '\b[0-9a-fA-F]{32,}\b',                                              '<REDACTED:hex>',     8, 8);
                ",
            )?;
        }
    }

    if version < SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }

    Ok(())
}
