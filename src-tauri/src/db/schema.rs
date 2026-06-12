use rusqlite::Connection;

use crate::error::AppResult;

const SCHEMA_VERSION: u32 = 17;

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

    if version < 14 {
        // AI 命令黑名单表。模型同 ai_redact_rules（v13）：出厂默认首次建表 seed 进表，
        // 之后无 builtin 概念，统一 CRUD。某类无行 = 放行该类，整表皆空 = 全放行（皆用户显式删除）。
        //
        // 这个块一辈子只跑一次（user_version 跨过 14 后不再进），删掉的命令不会复活。
        // name 是 PRIMARY KEY —— 一个命令只属一类，DB 层钉死。
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS ai_command_blacklist (
                name     TEXT PRIMARY KEY,
                category TEXT NOT NULL
            );
            ",
        )?;

        // 仅在表为空时 seed（防御性，建表块本就只跑一次）。
        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM ai_command_blacklist", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        if count == 0 {
            // 这 5 类 49 条与 ai::sanitize 的 5 张 const 表必须一致，由
            // ai::command_blacklist 的漂移守卫单测 `seed_matches_builtin` 把关
            // （改一处忘改另一处 = 红灯）。category 串见 BlCategory::as_str。
            conn.execute_batch(
                "
                INSERT INTO ai_command_blacklist (name, category) VALUES
                  ('rm','destructive'),('unlink','destructive'),('dd','destructive'),('mkfs','destructive'),
                  ('iptables','destructive'),('ip6tables','destructive'),('shutdown','destructive'),
                  ('reboot','destructive'),('halt','destructive'),('poweroff','destructive'),
                  ('kill','destructive'),('pkill','destructive'),('killall','destructive'),
                  ('mount','destructive'),('umount','destructive'),('exec','destructive'),
                  ('tee','write_verb'),('cp','write_verb'),('mv','write_verb'),('ln','write_verb'),
                  ('install','write_verb'),('truncate','write_verb'),('ed','write_verb'),
                  ('tar','write_verb'),('unzip','write_verb'),('cpio','write_verb'),
                  ('python','interpreter'),('python3','interpreter'),('python2','interpreter'),
                  ('perl','interpreter'),('ruby','interpreter'),('node','interpreter'),
                  ('nodejs','interpreter'),('lua','interpreter'),('luajit','interpreter'),
                  ('php','interpreter'),
                  ('eval','deferred_exec'),('source','deferred_exec'),('.','deferred_exec'),
                  ('xargs','forwarder'),('nice','forwarder'),('time','forwarder'),
                  ('timeout','forwarder'),('nohup','forwarder'),('stdbuf','forwarder'),
                  ('setsid','forwarder'),('ionice','forwarder'),('flock','forwarder'),
                  ('taskset','forwarder'),('chrt','forwarder');
                ",
            )?;
        }
    }

    if version < 15 {
        // Serial console profiles — a peer of `profiles`/`forwards`. No secret,
        // no FK: a saved port + line framing. UNIQUE name like the others.
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS serial_profiles (
                id           TEXT PRIMARY KEY,
                name         TEXT NOT NULL UNIQUE COLLATE NOCASE,
                port         TEXT NOT NULL,
                baud_rate    INTEGER NOT NULL DEFAULT 115200,
                data_bits    INTEGER NOT NULL DEFAULT 8,
                parity       TEXT NOT NULL DEFAULT 'none',
                stop_bits    INTEGER NOT NULL DEFAULT 1,
                flow_control TEXT NOT NULL DEFAULT 'none'
            );
            ",
        )?;
    }

    if version < 16 {
        // Tabby-style serial extras. ADD COLUMN (not a fresh CREATE) so a DB that
        // already ran v15's 8-column serial_profiles gets the new columns too.
        for stmt in [
            "ALTER TABLE serial_profiles ADD COLUMN xany INTEGER NOT NULL DEFAULT 0;",
            "ALTER TABLE serial_profiles ADD COLUMN input_newline TEXT NOT NULL DEFAULT 'cr';",
            "ALTER TABLE serial_profiles ADD COLUMN output_newline TEXT NOT NULL DEFAULT 'raw';",
            "ALTER TABLE serial_profiles ADD COLUMN local_echo INTEGER NOT NULL DEFAULT 0;",
            "ALTER TABLE serial_profiles ADD COLUMN backspace TEXT NOT NULL DEFAULT 'del';",
            "ALTER TABLE serial_profiles ADD COLUMN slow_send INTEGER NOT NULL DEFAULT 0;",
            "ALTER TABLE serial_profiles ADD COLUMN input_mode TEXT NOT NULL DEFAULT 'normal';",
            "ALTER TABLE serial_profiles ADD COLUMN output_mode TEXT NOT NULL DEFAULT 'text';",
            "ALTER TABLE serial_profiles ADD COLUMN login_script TEXT NOT NULL DEFAULT '';",
        ] {
            // Ignore "duplicate column" if a partial run already added some.
            let _ = conn.execute_batch(stmt);
        }
    }

    if version < 17 {
        // AI conversation persistence — two blobs per conversation, mirroring the
        // deliberate data fork in ai::session: history_json is the LLM's truth
        // (Vec<ChatMessage>, resume + continue), timeline_json is the UI's truth
        // (ChatItem[], re-render bubbles/cards verbatim). Reconstructing one from
        // the other is lossy reverse-engineering; storing both is dumb and clear.
        //
        // target_key groups conversations per terminal identity:
        //   "ssh:<profile_id>" / "local" / "serial:<port_name>"
        // One string, no kind column, no special cases.
        //
        // history_json holds UNREDACTED terminal output — same trust domain as
        // the credentials/secrets tables in this same local DB; redaction stays
        // at the LLM boundary (see docs/ai-diagnose-design.md).
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS ai_conversations (
                id            TEXT PRIMARY KEY,
                target_key    TEXT NOT NULL,
                title         TEXT NOT NULL DEFAULT '',
                history_json  TEXT NOT NULL DEFAULT '[]',
                timeline_json TEXT NOT NULL DEFAULT '[]',
                created_at    INTEGER NOT NULL DEFAULT 0,
                updated_at    INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_ai_conversations_target
                ON ai_conversations(target_key, updated_at);
            ",
        )?;
    }

    if version < SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }

    Ok(())
}
