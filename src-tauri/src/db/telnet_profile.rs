use rusqlite::{params, OptionalExtension};

use super::Db;
use crate::error::{AppError, AppResult};
use crate::models::{validate_name, TelnetEchoMode, TelnetProfile};

const COLS: &str =
    "id, name, host, port, input_newline, output_newline, local_echo, echo_mode, backspace, login_script, save_script_to_remote, group_id";

pub(crate) const PURGE_EPOCH_SETTING: &str = "telnet_login_script_purge_epoch";
pub(crate) const PURGED_EPOCH_SETTING: &str = "telnet_login_script_purged_epoch";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoginScriptVersionUpdate {
    Preserve,
    Set(String),
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoginScriptState {
    pub legacy_script: String,
    pub version: Option<String>,
}

/// Metadata and its immutable-secret pointer as observed by one SQLite query.
/// Keeping them together prevents callers from accidentally combining profile
/// generation A with the login-script pointer from generation B.
#[derive(Debug, Clone)]
pub(crate) struct ProfileSnapshot {
    pub metadata: TelnetProfile,
    pub login_script: LoginScriptState,
}

fn invalid_field(field: &'static str) -> AppError {
    AppError::config(
        "telnet_profile_invalid",
        serde_json::json!({ "field": field }),
    )
}

pub(crate) fn validate(t: &TelnetProfile) -> AppResult<()> {
    validate_name(&t.name)?;
    if t.host.is_empty()
        || t.host.chars().any(char::is_whitespace)
        || t.host.chars().any(char::is_control)
    {
        return Err(invalid_field("host"));
    }
    if t.port == 0 {
        return Err(invalid_field("port"));
    }
    if !matches!(t.input_newline.as_str(), "cr" | "lf" | "crlf") {
        return Err(invalid_field("input_newline"));
    }
    if !matches!(t.output_newline.as_str(), "raw" | "cr" | "lf" | "crlf") {
        return Err(invalid_field("output_newline"));
    }
    if !matches!(t.backspace.as_str(), "del" | "bs" | "csi3") {
        return Err(invalid_field("backspace"));
    }
    Ok(())
}

fn parse_echo_mode(raw: &str) -> rusqlite::Result<TelnetEchoMode> {
    match raw {
        "auto" => Ok(TelnetEchoMode::Auto),
        "on" => Ok(TelnetEchoMode::On),
        "off" => Ok(TelnetEchoMode::Off),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            7,
            rusqlite::types::Type::Text,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid telnet echo mode: {raw}"),
            )
            .into(),
        )),
    }
}

fn from_row(row: &rusqlite::Row) -> rusqlite::Result<TelnetProfile> {
    Ok(TelnetProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        // rusqlite's u16 FromSql is range-checked: a corrupted out-of-range
        // INTEGER fails loudly instead of silently truncating.
        port: row.get(3)?,
        input_newline: row.get(4)?,
        output_newline: row.get(5)?,
        local_echo: row.get(6)?,
        echo_mode: row
            .get::<_, Option<String>>(7)?
            .as_deref()
            .map(parse_echo_mode)
            .transpose()?,
        backspace: row.get(8)?,
        // Never expose the legacy plaintext column through metadata reads.
        // The command/sync boundary resolves its versioned SecretStore pointer.
        login_script: String::new(),
        save_script_to_remote: row.get(10)?,
        group_id: row.get(11)?,
    })
}

pub fn get(db: &Db, id: &str) -> AppResult<TelnetProfile> {
    let conn = db.lock()?;
    conn.query_row(
        &format!("SELECT {COLS} FROM telnet_profiles WHERE id = ?1"),
        params![id],
        from_row,
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            crate::error::AppError::not_found("telnet_profile_not_found", serde_json::json!({}))
        }
        other => other.into(),
    })
}

pub(crate) fn snapshot(db: &Db, id: &str) -> AppResult<ProfileSnapshot> {
    let conn = db.lock()?;
    conn.query_row(
        &format!("SELECT {COLS}, login_script_version FROM telnet_profiles WHERE id = ?1"),
        params![id],
        |row| {
            Ok(ProfileSnapshot {
                metadata: from_row(row)?,
                login_script: LoginScriptState {
                    legacy_script: row.get(9)?,
                    version: row.get(12)?,
                },
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            AppError::not_found("telnet_profile_not_found", serde_json::json!({}))
        }
        other => other.into(),
    })
}

pub fn list(db: &Db) -> AppResult<Vec<TelnetProfile>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM telnet_profiles ORDER BY name ASC"
    ))?;
    let rows = stmt.query_map([], from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn insert_tx_with_script_version(
    conn: &rusqlite::Connection,
    t: &TelnetProfile,
    script: &LoginScriptVersionUpdate,
) -> AppResult<()> {
    validate(t)?;
    let echo_mode = t.resolved_echo_mode();
    let legacy_local_echo = echo_mode == TelnetEchoMode::On;
    let version = match script {
        LoginScriptVersionUpdate::Set(version) => Some(version.as_str()),
        LoginScriptVersionUpdate::Delete => None,
        LoginScriptVersionUpdate::Preserve => {
            conn.execute(
                "INSERT INTO telnet_profiles \
                 (id, name, host, port, input_newline, output_newline, local_echo, echo_mode, echo_write_version, backspace, login_script, login_script_version, save_script_to_remote, group_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, '', NULL, ?10, ?11) \
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name, host=excluded.host, port=excluded.port, \
                  input_newline=excluded.input_newline, output_newline=excluded.output_newline, \
                  local_echo=excluded.local_echo, echo_mode=excluded.echo_mode, \
                  echo_write_version=telnet_profiles.echo_write_version + 1, \
                  backspace=excluded.backspace, save_script_to_remote=excluded.save_script_to_remote, \
                  group_id=excluded.group_id",
                params![
                    t.id,
                    t.name,
                    t.host,
                    t.port,
                    t.input_newline,
                    t.output_newline,
                    legacy_local_echo,
                    echo_mode.as_str(),
                    t.backspace,
                    t.save_script_to_remote,
                    t.group_id,
                ],
            )?;
            return Ok(());
        }
    };
    conn.execute(
        "INSERT INTO telnet_profiles \
         (id, name, host, port, input_newline, output_newline, local_echo, echo_mode, echo_write_version, backspace, login_script, login_script_version, save_script_to_remote, group_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, '', ?10, ?11, ?12) \
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, host=excluded.host, port=excluded.port, \
          input_newline=excluded.input_newline, output_newline=excluded.output_newline, \
          local_echo=excluded.local_echo, echo_mode=excluded.echo_mode, \
          echo_write_version=telnet_profiles.echo_write_version + 1, \
          backspace=excluded.backspace, login_script='', \
          login_script_version=excluded.login_script_version, \
          save_script_to_remote=excluded.save_script_to_remote, group_id=excluded.group_id",
        params![
            t.id,
            t.name,
            t.host,
            t.port,
            t.input_newline,
            t.output_newline,
            legacy_local_echo,
            echo_mode.as_str(),
            t.backspace,
            version,
            t.save_script_to_remote,
            t.group_id,
        ],
    )?;
    Ok(())
}

pub fn insert_tx(conn: &rusqlite::Connection, t: &TelnetProfile) -> AppResult<()> {
    insert_tx_with_script_version(conn, t, &LoginScriptVersionUpdate::Preserve)
}

pub fn insert(db: &Db, t: &TelnetProfile) -> AppResult<()> {
    let conn = db.lock()?;
    insert_tx(&conn, t)
}

fn update_tx_with_script_version(
    conn: &rusqlite::Connection,
    t: &TelnetProfile,
    script: &LoginScriptVersionUpdate,
) -> AppResult<()> {
    validate(t)?;
    let echo_mode = t.resolved_echo_mode();
    let legacy_local_echo = echo_mode == TelnetEchoMode::On;
    let changed = match script {
        LoginScriptVersionUpdate::Preserve => conn.execute(
            "UPDATE telnet_profiles SET name=?1, host=?2, port=?3, input_newline=?4, output_newline=?5, \
             local_echo=?6, echo_mode=?7, echo_write_version=echo_write_version + 1, \
             backspace=?8, save_script_to_remote=?9, group_id=?10 WHERE id=?11",
            params![
                t.name,
                t.host,
                t.port,
                t.input_newline,
                t.output_newline,
                legacy_local_echo,
                echo_mode.as_str(),
                t.backspace,
                t.save_script_to_remote,
                t.group_id,
                t.id,
            ],
        )?,
        LoginScriptVersionUpdate::Set(version) => conn.execute(
            "UPDATE telnet_profiles SET name=?1, host=?2, port=?3, input_newline=?4, output_newline=?5, \
             local_echo=?6, echo_mode=?7, echo_write_version=echo_write_version + 1, \
             backspace=?8, login_script='', login_script_version=?12, \
             save_script_to_remote=?9, group_id=?10 WHERE id=?11",
            params![
                t.name,
                t.host,
                t.port,
                t.input_newline,
                t.output_newline,
                legacy_local_echo,
                echo_mode.as_str(),
                t.backspace,
                t.save_script_to_remote,
                t.group_id,
                t.id,
                version,
            ],
        )?,
        LoginScriptVersionUpdate::Delete => conn.execute(
            "UPDATE telnet_profiles SET name=?1, host=?2, port=?3, input_newline=?4, output_newline=?5, \
             local_echo=?6, echo_mode=?7, echo_write_version=echo_write_version + 1, \
             backspace=?8, login_script='', login_script_version=NULL, \
             save_script_to_remote=?9, group_id=?10 WHERE id=?11",
            params![
                t.name,
                t.host,
                t.port,
                t.input_newline,
                t.output_newline,
                legacy_local_echo,
                echo_mode.as_str(),
                t.backspace,
                t.save_script_to_remote,
                t.group_id,
                t.id,
            ],
        )?,
    };
    if changed == 0 {
        return Err(AppError::not_found(
            "telnet_profile_not_found",
            serde_json::json!({}),
        ));
    }
    Ok(())
}

pub fn update(db: &Db, t: &TelnetProfile) -> AppResult<()> {
    let conn = db.lock()?;
    update_tx_with_script_version(&conn, t, &LoginScriptVersionUpdate::Preserve)
}

fn current_script_state(
    conn: &rusqlite::Connection,
    id: &str,
) -> AppResult<Option<LoginScriptState>> {
    Ok(conn
        .query_row(
            "SELECT login_script, login_script_version \
             FROM telnet_profiles WHERE id = ?1",
            params![id],
            |row| {
                Ok(LoginScriptState {
                    legacy_script: row.get(0)?,
                    version: row.get(1)?,
                })
            },
        )
        .optional()?)
}

fn bump_purge_epoch(conn: &rusqlite::Connection) -> AppResult<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, '1') \
         ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
        params![PURGE_EPOCH_SETTING],
    )?;
    Ok(())
}

pub(crate) fn insert_with_script_version(
    db: &Db,
    t: &TelnetProfile,
    script: LoginScriptVersionUpdate,
) -> AppResult<Option<String>> {
    validate(t)?;
    let mut conn = db.lock()?;
    if !matches!(&script, LoginScriptVersionUpdate::Preserve) {
        conn.pragma_update(None, "secure_delete", "ON")?;
    }
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let old_state = current_script_state(&tx, &t.id)?;
    insert_tx_with_script_version(&tx, t, &script)?;
    if !matches!(&script, LoginScriptVersionUpdate::Preserve)
        && old_state
            .as_ref()
            .is_some_and(|state| !state.legacy_script.is_empty())
    {
        bump_purge_epoch(&tx)?;
    }
    tx.commit()?;
    Ok(old_state.and_then(|state| state.version))
}

pub(crate) fn update_with_script_version(
    db: &Db,
    t: &TelnetProfile,
    script: LoginScriptVersionUpdate,
) -> AppResult<Option<String>> {
    validate(t)?;
    let mut conn = db.lock()?;
    if !matches!(&script, LoginScriptVersionUpdate::Preserve) {
        conn.pragma_update(None, "secure_delete", "ON")?;
    }
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let old_state = current_script_state(&tx, &t.id)?
        .ok_or_else(|| AppError::not_found("telnet_profile_not_found", serde_json::json!({})))?;
    update_tx_with_script_version(&tx, t, &script)?;
    if !matches!(&script, LoginScriptVersionUpdate::Preserve) && !old_state.legacy_script.is_empty()
    {
        bump_purge_epoch(&tx)?;
    }
    tx.commit()?;
    Ok(old_state.version)
}

pub fn delete(db: &Db, id: &str) -> AppResult<()> {
    let conn = db.lock()?;
    let changed = conn.execute("DELETE FROM telnet_profiles WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::not_found(
            "telnet_profile_not_found",
            serde_json::json!({}),
        ));
    }
    Ok(())
}

pub(crate) fn delete_with_script_version(db: &Db, id: &str) -> AppResult<Option<String>> {
    let mut conn = db.lock()?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let old_state = current_script_state(&tx, id)?
        .ok_or_else(|| AppError::not_found("telnet_profile_not_found", serde_json::json!({})))?;
    let changed = tx.execute("DELETE FROM telnet_profiles WHERE id = ?1", params![id])?;
    debug_assert_eq!(changed, 1);
    if !old_state.legacy_script.is_empty() {
        bump_purge_epoch(&tx)?;
    }
    tx.commit()?;
    Ok(old_state.version)
}

pub fn clear_all_tx(conn: &rusqlite::Connection) -> AppResult<()> {
    conn.execute("DELETE FROM telnet_profiles", [])?;
    Ok(())
}

pub(crate) fn login_script_state(db: &Db, id: &str) -> AppResult<LoginScriptState> {
    let conn = db.lock()?;
    conn.query_row(
        "SELECT login_script, login_script_version \
         FROM telnet_profiles WHERE id = ?1",
        params![id],
        |row| {
            Ok(LoginScriptState {
                legacy_script: row.get(0)?,
                version: row.get(1)?,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            AppError::not_found("telnet_profile_not_found", serde_json::json!({}))
        }
        other => other.into(),
    })
}

pub(crate) fn list_pending_legacy_login_scripts(
    db: &Db,
) -> AppResult<Vec<(String, LoginScriptState)>> {
    let conn = db.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, login_script, login_script_version \
         FROM telnet_profiles WHERE login_script != '' ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get(0)?,
            LoginScriptState {
                legacy_script: row.get(1)?,
                version: row.get(2)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn commit_legacy_login_script(
    db: &Db,
    id: &str,
    expected: &LoginScriptState,
    new_version: &str,
) -> AppResult<bool> {
    let mut conn = db.lock()?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let changed = tx.execute(
        "UPDATE telnet_profiles SET login_script = '', login_script_version = ?1, \
         echo_write_version = echo_write_version + 1 \
         WHERE id = ?2 AND login_script != '' AND login_script = ?3 \
         AND login_script_version IS ?4",
        params![
            new_version,
            id,
            expected.legacy_script,
            expected.version.as_deref()
        ],
    )?;
    if changed == 1 {
        bump_purge_epoch(&tx)?;
    }
    tx.commit()?;
    Ok(changed == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, name: &str) -> TelnetProfile {
        TelnetProfile {
            id: id.into(),
            name: name.into(),
            host: "192.168.1.1".into(),
            port: 23,
            input_newline: "crlf".into(),
            output_newline: "raw".into(),
            local_echo: false,
            echo_mode: Some(TelnetEchoMode::Auto),
            backspace: "del".into(),
            login_script: String::new(),
            save_script_to_remote: false,
            group_id: None,
        }
    }

    #[test]
    fn insert_then_get_roundtrips_all_fields() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("t1", "core-switch")).unwrap();
        let got = get(&db, "t1").unwrap();
        assert_eq!(got.name, "core-switch");
        assert_eq!(got.host, "192.168.1.1");
        assert_eq!(got.port, 23);
        assert_eq!(got.input_newline, "crlf");
        assert_eq!(got.output_newline, "raw");
        assert_eq!(got.backspace, "del");
        assert!(!got.local_echo);
        assert_eq!(got.echo_mode, Some(TelnetEchoMode::Auto));
    }

    #[test]
    fn upsert_overwrites_line_discipline() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("t1", "switch")).unwrap();
        let mut u = mk("t1", "switch");
        u.port = 2323;
        u.local_echo = true;
        u.echo_mode = Some(TelnetEchoMode::On);
        u.input_newline = "cr".into();
        u.backspace = "bs".into();
        u.login_script = "expect login:\nsend admin".into();
        insert(&db, &u).unwrap();
        let got = get(&db, "t1").unwrap();
        assert_eq!(got.port, 2323);
        assert!(got.local_echo);
        assert_eq!(got.input_newline, "cr");
        assert_eq!(got.backspace, "bs");
        assert!(
            got.login_script.is_empty(),
            "secret must not be stored in the profile row"
        );
    }

    #[test]
    fn update_changes_group_id() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("t1", "switch")).unwrap();
        let mut t = mk("t1", "switch");
        t.group_id = Some("g9".into());
        update(&db, &t).unwrap();
        assert_eq!(get(&db, "t1").unwrap().group_id.as_deref(), Some("g9"));
    }

    #[test]
    fn list_sorted_by_name() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("t1", "zebra")).unwrap();
        insert(&db, &mk("t2", "apple")).unwrap();
        let names: Vec<String> = list(&db).unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["apple", "zebra"]);
    }

    #[test]
    fn delete_removes_row() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, &mk("t1", "switch")).unwrap();
        delete(&db, "t1").unwrap();
        assert_eq!(
            get(&db, "t1").unwrap_err().code(),
            "telnet_profile_not_found"
        );
    }

    #[test]
    fn insert_rejects_name_with_control_char() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(
            insert(&db, &mk("t1", "bad\nname")).unwrap_err().code(),
            "name_has_control_char"
        );
    }

    #[test]
    fn high_port_roundtrips() {
        // u16 boundary through the INTEGER column.
        let db = Db::open_in_memory().unwrap();
        let mut t = mk("t1", "highport");
        t.port = 65535;
        insert(&db, &t).unwrap();
        assert_eq!(get(&db, "t1").unwrap().port, 65535);
    }

    #[test]
    fn out_of_range_port_in_db_fails_loudly() {
        // The typed API can't store >65535; plant it with raw SQL the way a
        // corrupted row would arrive. Reading back must error, not truncate.
        let db = Db::open_in_memory().unwrap();
        db.lock()
            .unwrap()
            .execute(
                "INSERT INTO telnet_profiles (id, name, host, port) VALUES ('x', 'x', 'h', 70000)",
                [],
            )
            .unwrap();
        assert!(get(&db, "x").is_err());
    }

    #[test]
    fn older_payload_without_new_keys_deserializes_with_defaults() {
        // Import path: a minimal JSON (id/name/host only) must parse — serde
        // defaults fill the rest. Guards the sync/export compatibility promise.
        let t: TelnetProfile =
            serde_json::from_str(r#"{"id":"t1","name":"sw","host":"10.0.0.1"}"#).unwrap();
        assert_eq!(t.port, 23);
        assert_eq!(t.input_newline, "crlf");
        assert_eq!(t.output_newline, "raw");
        assert_eq!(t.backspace, "del");
        assert!(!t.local_echo);
        assert_eq!(t.echo_mode, None);
        assert!(!t.save_script_to_remote);
        assert_eq!(t.group_id, None);
    }

    #[test]
    fn insert_rejects_invalid_line_discipline() {
        let db = Db::open_in_memory().unwrap();
        for (field, value) in [
            ("input_newline", "wat"),
            ("output_newline", "wat"),
            ("backspace", "wat"),
        ] {
            let mut t = mk(field, field);
            match field {
                "input_newline" => t.input_newline = value.into(),
                "output_newline" => t.output_newline = value.into(),
                "backspace" => t.backspace = value.into(),
                _ => unreachable!(),
            }
            assert_eq!(
                insert(&db, &t).unwrap_err().code(),
                "telnet_profile_invalid"
            );
        }
    }

    #[test]
    fn insert_rejects_zero_port() {
        let db = Db::open_in_memory().unwrap();
        let mut t = mk("t1", "switch");
        t.port = 0;
        assert_eq!(
            insert(&db, &t).unwrap_err().code(),
            "telnet_profile_invalid"
        );
    }

    #[test]
    fn legacy_local_echo_true_is_canonicalized_to_on() {
        let db = Db::open_in_memory().unwrap();
        let mut t = mk("t1", "switch");
        t.echo_mode = None;
        t.local_echo = true;
        insert(&db, &t).unwrap();
        let got = get(&db, "t1").unwrap();
        assert!(got.local_echo);
        assert_eq!(got.echo_mode, Some(TelnetEchoMode::On));
    }
}
