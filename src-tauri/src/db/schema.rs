use rusqlite::{params, Connection};

use crate::error::AppResult;

const SCHEMA_VERSION: u32 = 24;

fn column_exists(conn: &Connection, table: &str, col: &str) -> AppResult<bool> {
    let mut stmt = conn.prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2")?;
    Ok(stmt.exists([table, col])?)
}

fn table_exists(conn: &Connection, table: &str) -> AppResult<bool> {
    let mut stmt =
        conn.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")?;
    Ok(stmt.exists([table])?)
}

fn default_algorithms_json() -> AppResult<String> {
    serde_json::to_string(&crate::models::default_ssh_algorithms()).map_err(|e| {
        crate::error::AppError::other("serde_failed", serde_json::json!({ "err": e.to_string() }))
    })
}

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
        //   "ssh:<profile_id>" / "local" / "serial:<port_name>" / "telnet:<host:port>"
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

    if version < 18 {
        // Keyword highlighting now supports regex and case-sensitive toggles.
        // Default 0 preserves existing rule behavior: plain text, case-insensitive.
        if !column_exists(conn, "highlights", "is_regex")? {
            conn.execute_batch(
                "ALTER TABLE highlights ADD COLUMN is_regex INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        if !column_exists(conn, "highlights", "is_case_sensitive")? {
            conn.execute_batch(
                "ALTER TABLE highlights ADD COLUMN is_case_sensitive INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
    }

    if version < 19 {
        // Regex highlight rules support a human-readable name for easier list management.
        if !column_exists(conn, "highlights", "name")? {
            conn.execute_batch("ALTER TABLE highlights ADD COLUMN name TEXT NOT NULL DEFAULT '';")?;
        }
        let ipv4_pattern =
            r"\b(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}\b";
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM highlights WHERE keyword = ?1",
            params![ipv4_pattern],
            |r| r.get(0),
        )?;
        if exists == 0 {
            conn.execute(
                "INSERT INTO highlights (keyword, name, color, enabled, is_regex, is_case_sensitive) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![ipv4_pattern, "IPv4", "#D86BFF", 1, 1, 0],
            )?;
        }
    }

    if version < 20 {
        // group_id on forwards / serial_profiles. They become peers of profiles
        // (v10) under the shared `groups` table. ADD COLUMN DEFAULT NULL → every
        // existing row is "ungrouped"; zero breakage. Mirrors v10's profiles add.
        if !column_exists(conn, "forwards", "group_id")? {
            conn.execute_batch("ALTER TABLE forwards ADD COLUMN group_id TEXT DEFAULT NULL;")?;
        }
        if !column_exists(conn, "serial_profiles", "group_id")? {
            conn.execute_batch(
                "ALTER TABLE serial_profiles ADD COLUMN group_id TEXT DEFAULT NULL;",
            )?;
        }
    }

    if version < 21 {
        // Highlight rules unify on regex: the text/regex split is gone and names
        // are required. Two orthogonal fixups, applied to every row needing either
        // — matching stays byte-for-byte unchanged:
        //   - is_regex=0 (legacy plain text): escape the keyword into the
        //     equivalent regex (C++ -> C\+\+, $HOME -> \$HOME) and set is_regex=1.
        //     An already-regex keyword is NEVER re-escaped.
        //   - name='' (a nameless rule from before names were required — plain OR
        //     regex, the latter created between v18 and this release): seed the
        //     name from the ORIGINAL keyword so it keeps a readable label and
        //     passes the editor's now-required-name check.
        // Idempotent: afterwards every row has is_regex=1 and a non-empty name, so
        // the WHERE matches nothing on a second run.
        let stale: Vec<(i64, String, String, bool)> = {
            let mut stmt = conn.prepare(
                "SELECT id, keyword, name, is_regex FROM highlights \
                 WHERE is_regex = 0 OR trim(name) = ''",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (id, keyword, name, is_regex) in stale {
            let new_keyword = if is_regex {
                keyword.clone()
            } else {
                crate::db::highlight::regex_escape(&keyword)
            };
            let new_name = if name.trim().is_empty() {
                keyword.clone()
            } else {
                name
            };
            conn.execute(
                "UPDATE highlights SET keyword = ?1, name = ?2, is_regex = 1 WHERE id = ?3",
                params![new_keyword, new_name, id],
            )?;
        }
    }

    if version < 22 {
        // Telnet profiles — a peer of serial_profiles: named host:port + the
        // line-discipline knobs that make sense for an NVT. login_script began
        // in this table; migration v2 moves its contents to SecretStore and
        // keeps the column empty for schema compatibility.
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS telnet_profiles (
                id             TEXT PRIMARY KEY,
                name           TEXT NOT NULL UNIQUE COLLATE NOCASE,
                host           TEXT NOT NULL,
                port           INTEGER NOT NULL DEFAULT 23,
                input_newline  TEXT NOT NULL DEFAULT 'crlf',
                output_newline TEXT NOT NULL DEFAULT 'raw',
                local_echo     INTEGER NOT NULL DEFAULT 0,
                echo_mode      TEXT DEFAULT NULL,
                echo_write_version INTEGER NOT NULL DEFAULT 0,
                backspace      TEXT NOT NULL DEFAULT 'del',
                login_script   TEXT NOT NULL DEFAULT '',
                login_script_version TEXT DEFAULT NULL,
                save_script_to_remote INTEGER NOT NULL DEFAULT 0,
                group_id       TEXT DEFAULT NULL
            );
            ",
        )?;
    }

    if version < 23 {
        // Per-profile SSH algorithm preferences. New rows get the current safe
        // default list, and old/partially-migrated rows with NULL/empty payloads
        // are normalized here so sync/export always sees a concrete JSON object.
        if table_exists(conn, "profiles")? {
            let default_json = default_algorithms_json()?;
            if !column_exists(conn, "profiles", "algorithms")? {
                let escaped = default_json.replace('\'', "''");
                conn.execute_batch(&format!(
                    "ALTER TABLE profiles ADD COLUMN algorithms TEXT NOT NULL DEFAULT '{}';",
                    escaped
                ))?;
            }
            conn.execute(
                "UPDATE profiles SET algorithms = ?1 \
                 WHERE algorithms IS NULL OR trim(algorithms) = '' OR trim(algorithms) = 'null'",
                params![default_json],
            )?;
        }
    }

    if version < 24 && table_exists(conn, "telnet_profiles")? {
        // RFC ECHO negotiation needs three states: automatic, forced on and
        // forced off. Preserve the legacy bool exactly for existing profiles.
        if !column_exists(conn, "telnet_profiles", "echo_mode")? {
            conn.execute_batch(
                "ALTER TABLE telnet_profiles ADD COLUMN echo_mode TEXT DEFAULT NULL;",
            )?;
        }
        if !column_exists(conn, "telnet_profiles", "echo_write_version")? {
            conn.execute_batch(
                "ALTER TABLE telnet_profiles ADD COLUMN echo_write_version INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        if !column_exists(conn, "telnet_profiles", "login_script_version")? {
            conn.execute_batch(
                "ALTER TABLE telnet_profiles ADD COLUMN login_script_version TEXT DEFAULT NULL;",
            )?;
        }
        if !column_exists(conn, "telnet_profiles", "save_script_to_remote")? {
            conn.execute_batch(
                "ALTER TABLE telnet_profiles ADD COLUMN save_script_to_remote INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        // Older binaries name local_echo/login_script in every profile write.
        // New writes advance echo_write_version atomically; the echo trigger
        // uses that marker to recognize legacy writes even when a bool is
        // unchanged. Install it before the echo backfill so no legacy write can
        // fall between trigger coverage and the scan.
        //
        // The legacy login_script column is itself the migration inbox:
        // non-empty means Replace, empty means Preserve. The script trigger has
        // one job only: an old metadata save must not erase an already-pending
        // non-empty replacement before reconciliation consumes it.
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS telnet_profiles_legacy_echo_update;
             DROP TRIGGER IF EXISTS telnet_profiles_legacy_script_insert;
             DROP TRIGGER IF EXISTS telnet_profiles_legacy_script_update;

             CREATE TRIGGER telnet_profiles_legacy_echo_update
             AFTER UPDATE OF local_echo ON telnet_profiles
             WHEN NEW.echo_write_version = OLD.echo_write_version
             BEGIN
               UPDATE telnet_profiles
               SET echo_mode = CASE WHEN NEW.local_echo != 0 THEN 'on' ELSE 'off' END
               WHERE id = NEW.id;
             END;

             CREATE TRIGGER telnet_profiles_legacy_script_update
             AFTER UPDATE OF login_script ON telnet_profiles
             WHEN NEW.echo_write_version = OLD.echo_write_version
               AND NEW.login_script = '' AND OLD.login_script != ''
             BEGIN
               UPDATE telnet_profiles
               SET login_script = OLD.login_script,
                   echo_write_version = echo_write_version + 1
               WHERE id = NEW.id;
             END;",
        )?;
        conn.execute(
            "UPDATE telnet_profiles
             SET echo_mode = CASE WHEN local_echo != 0 THEN 'on' ELSE 'off' END
             WHERE echo_mode IS NULL OR echo_write_version = 0",
            [],
        )?;
    }

    if version < SCHEMA_VERSION {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_21_escapes_plain_text_highlights() {
        let conn = Connection::open_in_memory().unwrap();
        // Simulate a pre-v21 DB: highlights as of v19 holding one plain-text rule
        // and one regex rule. Stamp user_version at 20 so migrate() runs only the
        // v21 step (every earlier gate is skipped, so other tables aren't needed).
        conn.execute_batch(
            "CREATE TABLE highlights (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                keyword TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '',
                color TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                is_regex INTEGER NOT NULL DEFAULT 0,
                is_case_sensitive INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO highlights (keyword, color, enabled, is_regex) VALUES ('C++', '#fff', 1, 0);
            INSERT INTO highlights (keyword, name, color, enabled, is_regex) VALUES ('\\d+', 'nums', '#000', 1, 1);",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 20u32).unwrap();

        migrate(&conn).unwrap();

        // Plain-text C++ became an escaped regex; the existing regex is untouched.
        let (kw, name, isr): (String, String, bool) = conn
            .query_row(
                "SELECT keyword, name, is_regex FROM highlights WHERE color = '#fff'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(kw, r"C\+\+");
        assert_eq!(name, "C++", "empty name seeded from the original keyword");
        assert!(isr);

        let kw2: String = conn
            .query_row(
                "SELECT keyword FROM highlights WHERE name = 'nums'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kw2, r"\d+");

        // Idempotent: no is_regex=0 row remains after the migration.
        let zeros: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM highlights WHERE is_regex = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(zeros, 0);
    }

    #[test]
    fn migration_21_seeds_names_for_nameless_regex_highlights() {
        // A regex rule created between v18 (is_regex added) and the required-name
        // UI: is_regex=1 but name='' (v19's default). v21 must seed its name from
        // the keyword too — otherwise the row survives with a blank name and the
        // editor's validateHighlightRule rejects it. The keyword is ALREADY a
        // regex pattern, so it must NOT be re-escaped.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE highlights (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                keyword TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '',
                color TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                is_regex INTEGER NOT NULL DEFAULT 0,
                is_case_sensitive INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO highlights (keyword, name, color, enabled, is_regex) VALUES ('\\d+', '', '#0f0', 1, 1);
            INSERT INTO highlights (keyword, name, color, enabled, is_regex) VALUES ('\\w+', 'words', '#00f', 1, 1);",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 20u32).unwrap();

        migrate(&conn).unwrap();

        // Nameless regex rule: name seeded from keyword, keyword left intact.
        let (kw, name, isr): (String, String, bool) = conn
            .query_row(
                "SELECT keyword, name, is_regex FROM highlights WHERE color = '#0f0'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(kw, r"\d+", "existing regex keyword must not be re-escaped");
        assert_eq!(name, r"\d+", "empty name seeded from the keyword");
        assert!(isr);

        // A regex rule that already had a name is left completely untouched.
        let kw2: String = conn
            .query_row(
                "SELECT keyword FROM highlights WHERE name = 'words'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kw2, r"\w+");
    }

    #[test]
    fn migration_23_adds_default_profile_algorithms() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE profiles (
                id                 TEXT PRIMARY KEY,
                name               TEXT NOT NULL UNIQUE COLLATE NOCASE,
                host               TEXT NOT NULL,
                port               INTEGER NOT NULL DEFAULT 22,
                credential_id      TEXT NOT NULL DEFAULT '',
                bastion_profile_id TEXT,
                init_command       TEXT,
                group_id           TEXT DEFAULT NULL
            );
            INSERT INTO profiles (id, name, host, port, credential_id)
              VALUES ('p1', 'P1', 'h.example', 22, 'c1');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 22u32).unwrap();

        migrate(&conn).unwrap();

        let raw: String = conn
            .query_row("SELECT algorithms FROM profiles WHERE id = 'p1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let algorithms: crate::models::SshAlgorithms = serde_json::from_str(&raw).unwrap();
        assert!(algorithms.kex.contains(&"curve25519-sha256".into()));
        assert!(!algorithms
            .kex
            .contains(&"diffie-hellman-group1-sha1".into()));
    }

    #[test]
    fn migration_23_normalizes_null_profile_algorithms() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE profiles (
                id                 TEXT PRIMARY KEY,
                name               TEXT NOT NULL UNIQUE COLLATE NOCASE,
                host               TEXT NOT NULL,
                port               INTEGER NOT NULL DEFAULT 22,
                credential_id      TEXT NOT NULL DEFAULT '',
                bastion_profile_id TEXT,
                init_command       TEXT,
                group_id           TEXT DEFAULT NULL,
                algorithms         TEXT
            );
            INSERT INTO profiles (id, name, host, port, credential_id, algorithms)
              VALUES ('p1', 'P1', 'h.example', 22, 'c1', NULL),
                     ('p2', 'P2', 'h.example', 22, 'c1', ' null ');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 22u32).unwrap();

        migrate(&conn).unwrap();

        let raw: String = conn
            .query_row("SELECT algorithms FROM profiles WHERE id = 'p1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let algorithms: crate::models::SshAlgorithms = serde_json::from_str(&raw).unwrap();
        assert!(algorithms
            .cipher
            .contains(&"chacha20-poly1305@openssh.com".into()));
        let raw: String = conn
            .query_row("SELECT algorithms FROM profiles WHERE id = 'p2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let algorithms: crate::models::SshAlgorithms = serde_json::from_str(&raw).unwrap();
        assert!(algorithms
            .cipher
            .contains(&"chacha20-poly1305@openssh.com".into()));
    }

    #[test]
    fn migration_24_preserves_legacy_telnet_echo_choice() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE telnet_profiles (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                host TEXT NOT NULL,
                local_echo INTEGER NOT NULL DEFAULT 0,
                login_script TEXT NOT NULL DEFAULT ''
            );
            INSERT INTO telnet_profiles (id, name, host, local_echo)
              VALUES ('auto', 'Auto', 'a', 0), ('on', 'On', 'b', 1);",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 23u32).unwrap();

        migrate(&conn).unwrap();

        let auto: String = conn
            .query_row(
                "SELECT echo_mode FROM telnet_profiles WHERE id = 'auto'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let on: String = conn
            .query_row(
                "SELECT echo_mode FROM telnet_profiles WHERE id = 'on'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(auto, "off");
        assert_eq!(on, "on");

        // A v0.2.11 client only knows local_echo. Its later edit must still be
        // visible to the new enum reader instead of leaving the columns split.
        conn.execute(
            "UPDATE telnet_profiles SET local_echo = 1 WHERE id = 'auto'",
            [],
        )
        .unwrap();
        let updated: String = conn
            .query_row(
                "SELECT echo_mode FROM telnet_profiles WHERE id = 'auto'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(updated, "on");

        conn.execute(
            "INSERT INTO telnet_profiles \
             (id, name, host, local_echo, echo_mode, echo_write_version) \
             VALUES ('new', 'New', 'c', 0, 'auto', 1)",
            [],
        )
        .unwrap();
        // The old writer names local_echo in its UPDATE even when false stays
        // false. Writer version is unchanged, so Auto becomes explicit Off.
        conn.execute(
            "UPDATE telnet_profiles SET local_echo = 0 WHERE id = 'new'",
            [],
        )
        .unwrap();
        let old_writer_mode: String = conn
            .query_row(
                "SELECT echo_mode FROM telnet_profiles WHERE id = 'new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_writer_mode, "off");

        // New writers advance the marker in the same statement, so saving Auto
        // is not mistaken for a legacy false-checkbox write.
        conn.execute(
            "UPDATE telnet_profiles SET local_echo = 0, echo_mode = 'auto', \
             echo_write_version = echo_write_version + 1 WHERE id = 'new'",
            [],
        )
        .unwrap();
        let new_writer_mode: String = conn
            .query_row(
                "SELECT echo_mode FROM telnet_profiles WHERE id = 'new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(new_writer_mode, "auto");

        conn.execute(
            "UPDATE telnet_profiles
             SET login_script_version = 'v1', login_script = ''
             WHERE id = 'new'",
            [],
        )
        .unwrap();
        // Empty is ambiguous after scrubbing, so an old metadata save must not
        // delete the active immutable version.
        conn.execute(
            "UPDATE telnet_profiles SET login_script = '' WHERE id = 'new'",
            [],
        )
        .unwrap();
        let preserved: Option<String> = conn
            .query_row(
                "SELECT login_script_version FROM telnet_profiles WHERE id = 'new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(preserved, Some("v1".into()));

        // Non-empty is the one legacy intent that remains unambiguous.
        conn.execute(
            "UPDATE telnet_profiles SET login_script = 'replacement' WHERE id = 'new'",
            [],
        )
        .unwrap();
        // A stale old client writing empty before reconciliation must preserve
        // that pending replacement as well, not erase it.
        conn.execute(
            "UPDATE telnet_profiles SET login_script = '' WHERE id = 'new'",
            [],
        )
        .unwrap();
        let pending: String = conn
            .query_row(
                "SELECT login_script FROM telnet_profiles WHERE id = 'new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pending, "replacement");
    }

    #[test]
    fn migration_24_backfill_covers_a_write_before_trigger_installation() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE telnet_profiles (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                host TEXT NOT NULL,
                local_echo INTEGER NOT NULL DEFAULT 0,
                echo_mode TEXT DEFAULT NULL,
                echo_write_version INTEGER NOT NULL DEFAULT 0,
                login_script TEXT NOT NULL DEFAULT ''
            );
            INSERT INTO telnet_profiles
              (id, name, host, local_echo, echo_mode, login_script)
              VALUES ('race', 'Race', 'h', 1, 'off', 'late write');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 23u32).unwrap();

        migrate(&conn).unwrap();

        let state: (String, String) = conn
            .query_row(
                "SELECT echo_mode, login_script FROM telnet_profiles WHERE id = 'race'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, ("on".into(), "late write".into()));
    }

    #[test]
    fn migration_24_does_not_add_a_legacy_pending_column() {
        let conn = Connection::open_in_memory().unwrap();

        migrate(&conn).unwrap();

        assert!(!column_exists(&conn, "telnet_profiles", "login_script_legacy_pending").unwrap());
    }
}
