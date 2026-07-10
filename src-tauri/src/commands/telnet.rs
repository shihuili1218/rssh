use tauri::{AppHandle, Emitter, State};

use crate::db::Db;
use crate::error::{locked, AppError, AppResult};
use crate::models::TelnetProfile;
use crate::secret::{telnet_login_script_key, SecretStore};
use crate::state::{AppState, SessionSlot};
use crate::terminal::telnet;

/// Async on purpose: DNS resolution + TCP connect can block for up to 10s per
/// address. A sync command would sit on the main thread and freeze the UI, so
/// the blocking connect runs on a worker via spawn_blocking.
///
/// `cols`/`rows` seed the NAWS activation reply with the real terminal size
/// (same contract as `ssh_connect`).
#[tauri::command]
pub async fn telnet_open(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    host: String,
    port: u16,
    cols: u16,
    rows: u16,
    input_newline: Option<String>,
    session_id: Option<String>,
) -> AppResult<String> {
    // Turn transport-agnostic telnet output into Tauri events. The headless ws
    // server builds a different sink over the same `telnet::open`.
    let session_id = crate::commands::lifecycle::resolve_session_id(session_id)?;
    let input_newline = input_newline.unwrap_or_else(|| "crlf".into());
    let reservation = crate::commands::lifecycle::reserve_window_session(
        &state,
        &state.telnet_sessions,
        window.label(),
        &session_id,
    )?;
    let sink: telnet::TelnetSink =
        std::sync::Arc::new(move |id: &str, out: telnet::TelnetOut| match out {
            telnet::TelnetOut::Data(b) => {
                let _ = app.emit(&format!("telnet:data:{id}"), b);
            }
            telnet::TelnetOut::RemoteEcho(enabled) => {
                let _ = app.emit(&format!("telnet:echo:{id}"), enabled);
            }
            telnet::TelnetOut::Close => {
                let _ = app.emit(&format!("telnet:close:{id}"), ());
            }
        });
    let spawn_session_id = session_id.clone();
    let opened = tauri::async_runtime::spawn_blocking(move || {
        telnet::open(
            spawn_session_id,
            &host,
            port,
            cols,
            rows,
            &input_newline,
            sink,
        )
    })
    .await
    .map_err(|e| {
        AppError::other(
            "task_join_failed",
            serde_json::json!({ "err": e.to_string() }),
        )
    });
    let (id, handle) = opened??;
    reservation.activate(handle)?;
    Ok(id)
}

/// Look up an open telnet session's handle (cloned — `TelnetHandle` is Arc-backed).
fn telnet_handle(state: &State<'_, AppState>, session_id: &str) -> AppResult<telnet::TelnetHandle> {
    locked(&state.telnet_sessions)?
        .get(session_id)
        .and_then(SessionSlot::ready)
        .cloned()
        .ok_or_else(|| AppError::not_found("telnet_not_found", serde_json::json!({})))
}

#[tauri::command]
pub fn telnet_write(
    state: State<'_, AppState>,
    session_id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    telnet_handle(&state, &session_id)?.write(&data)
}

#[tauri::command]
pub fn telnet_write_line(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> AppResult<()> {
    telnet_handle(&state, &session_id)?.write_line(&text)
}

/// Report the terminal size to the server (NAWS). Unlike serial, telnet HAS
/// rows/cols; before the server activates NAWS this is a silent no-op.
#[tauri::command]
pub fn telnet_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> AppResult<()> {
    telnet_handle(&state, &session_id)?.resize(cols, rows)
}

#[tauri::command]
pub fn telnet_close(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    crate::commands::lifecycle::unregister_window_session(&state, &session_id);
    locked(&state.telnet_sessions)?.remove(&session_id);
    Ok(())
}

// ── Saved telnet profiles (peer of serial profiles; SQLite-persisted CRUD) ──

#[tauri::command]
pub fn list_telnet_profiles(state: State<'_, AppState>) -> AppResult<Vec<TelnetProfile>> {
    crate::db::telnet_profile::list(&state.db)
}

#[tauri::command]
pub fn get_telnet_profile(state: State<'_, AppState>, id: String) -> AppResult<TelnetProfile> {
    let mut profile = crate::db::telnet_profile::get(&state.db, &id)?;
    hydrate_login_script(&mut profile, &state.db, state.secret_store.as_ref())?;
    Ok(profile)
}

#[tauri::command]
pub fn create_telnet_profile(state: State<'_, AppState>, profile: TelnetProfile) -> AppResult<()> {
    let update = LoginScriptUpdate::from_profile(&profile);
    insert_profile(&state.db, state.secret_store.as_ref(), &profile, update)
}

#[tauri::command]
pub fn update_telnet_profile(state: State<'_, AppState>, profile: TelnetProfile) -> AppResult<()> {
    let update = LoginScriptUpdate::from_profile(&profile);
    update_profile(&state.db, state.secret_store.as_ref(), &profile, update)
}

#[tauri::command]
pub fn delete_telnet_profile(state: State<'_, AppState>, id: String) -> AppResult<()> {
    delete_profile(&state.db, state.secret_store.as_ref(), &id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LoginScriptUpdate {
    /// A scrubbed sync payload: retain this device's existing secret.
    Preserve,
    Set(String),
    Delete,
}

impl LoginScriptUpdate {
    pub(crate) fn from_profile(profile: &TelnetProfile) -> Self {
        if profile.login_script.is_empty() {
            Self::Delete
        } else {
            Self::Set(profile.login_script.clone())
        }
    }
}

pub(crate) fn hydrate_login_script(
    profile: &mut TelnetProfile,
    db: &Db,
    store: &dyn SecretStore,
) -> AppResult<()> {
    crate::migration::v2_telnet_login_script::reconcile_profile(db, store, &profile.id)?;
    crate::migration::v2_telnet_login_script::retry_pending_purge(db);
    let state = crate::db::telnet_profile::login_script_state(db, &profile.id)?;
    profile.login_script = match state.version {
        Some(version) => store
            .get(&telnet_login_script_key(&profile.id, &version))?
            .ok_or_else(|| {
                AppError::other(
                    "telnet_login_script_missing",
                    serde_json::json!({ "id": profile.id, "version": version }),
                )
            })?,
        None => String::new(),
    };
    Ok(())
}

fn optional_profile(db: &Db, id: &str) -> AppResult<Option<TelnetProfile>> {
    match crate::db::telnet_profile::get(db, id) {
        Ok(profile) => Ok(Some(profile)),
        Err(e) if e.code() == "telnet_profile_not_found" => Ok(None),
        Err(e) => Err(e),
    }
}

fn delete_version_best_effort(store: &dyn SecretStore, profile_id: &str, version: Option<&str>) {
    let Some(version) = version else { return };
    if let Err(e) = store.delete(&telnet_login_script_key(profile_id, version)) {
        log::warn!("failed to remove obsolete Telnet login-script version: {e}");
    }
}

fn write_profile(
    db: &Db,
    store: &dyn SecretStore,
    profile: &TelnetProfile,
    update: LoginScriptUpdate,
    insert: bool,
) -> AppResult<()> {
    crate::db::telnet_profile::validate(profile)?;

    if matches!(update, LoginScriptUpdate::Preserve) {
        if optional_profile(db, &profile.id)?.is_some() {
            crate::migration::v2_telnet_login_script::reconcile_profile(db, store, &profile.id)?;
            crate::migration::v2_telnet_login_script::retry_pending_purge(db);
        }
        let script = crate::db::telnet_profile::LoginScriptVersionUpdate::Preserve;
        if insert {
            crate::db::telnet_profile::insert_with_script_version(db, profile, script)?;
        } else {
            crate::db::telnet_profile::update_with_script_version(db, profile, script)?;
        }
        return Ok(());
    }

    let (script, new_version) = match update {
        LoginScriptUpdate::Set(value) => {
            let version = uuid::Uuid::new_v4().to_string();
            store.set(&telnet_login_script_key(&profile.id, &version), &value)?;
            (
                crate::db::telnet_profile::LoginScriptVersionUpdate::Set(version.clone()),
                Some(version),
            )
        }
        LoginScriptUpdate::Delete => (
            crate::db::telnet_profile::LoginScriptVersionUpdate::Delete,
            None,
        ),
        LoginScriptUpdate::Preserve => unreachable!(),
    };

    let result = if insert {
        crate::db::telnet_profile::insert_with_script_version(db, profile, script)
    } else {
        crate::db::telnet_profile::update_with_script_version(db, profile, script)
    };
    let old_version = match result {
        Ok(version) => version,
        Err(e) => {
            delete_version_best_effort(store, &profile.id, new_version.as_deref());
            return Err(e);
        }
    };
    if old_version.as_deref() != new_version.as_deref() {
        delete_version_best_effort(store, &profile.id, old_version.as_deref());
    }
    Ok(())
}

pub(crate) fn insert_profile(
    db: &Db,
    store: &dyn SecretStore,
    profile: &TelnetProfile,
    update: LoginScriptUpdate,
) -> AppResult<()> {
    write_profile(db, store, profile, update, true)
}

pub(crate) fn update_profile(
    db: &Db,
    store: &dyn SecretStore,
    profile: &TelnetProfile,
    update: LoginScriptUpdate,
) -> AppResult<()> {
    write_profile(db, store, profile, update, false)
}

pub(crate) fn delete_profile(db: &Db, store: &dyn SecretStore, id: &str) -> AppResult<()> {
    let old_version = crate::db::telnet_profile::delete_with_script_version(db, id)?;
    delete_version_best_effort(store, id, old_version.as_deref());
    Ok(())
}

#[cfg(test)]
mod profile_secret_tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::models::TelnetEchoMode;

    #[derive(Default)]
    struct TestStore {
        values: Mutex<HashMap<String, String>>,
        fail_set: AtomicBool,
    }

    impl SecretStore for TestStore {
        fn get(&self, key: &str) -> AppResult<Option<String>> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: &str) -> AppResult<()> {
            if self.fail_set.load(Ordering::Relaxed) {
                return Err(AppError::other("secret_set_failed", serde_json::json!({})));
            }
            self.values
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn delete(&self, key: &str) -> AppResult<()> {
            self.values.lock().unwrap().remove(key);
            Ok(())
        }

        fn backend_name(&self) -> &'static str {
            "test"
        }
    }

    fn profile(id: &str, name: &str, script: &str) -> TelnetProfile {
        TelnetProfile {
            id: id.into(),
            name: name.into(),
            host: "192.0.2.1".into(),
            port: 23,
            input_newline: "crlf".into(),
            output_newline: "raw".into(),
            local_echo: false,
            echo_mode: Some(TelnetEchoMode::Auto),
            backspace: "del".into(),
            login_script: script.into(),
            save_script_to_remote: false,
            group_id: None,
        }
    }

    fn stored_script(db: &Db, store: &TestStore, id: &str) -> Option<String> {
        let state = crate::db::telnet_profile::login_script_state(db, id).unwrap();
        state
            .version
            .and_then(|version| store.get(&telnet_login_script_key(id, &version)).unwrap())
    }

    #[test]
    fn secret_failure_leaves_existing_profile_unchanged() {
        let db = Db::open_in_memory().unwrap();
        let store = TestStore::default();
        let original = profile("t1", "Original", "old script");
        insert_profile(
            &db,
            &store,
            &original,
            LoginScriptUpdate::from_profile(&original),
        )
        .unwrap();

        store.fail_set.store(true, Ordering::Relaxed);
        let changed = profile("t1", "Changed", "new script");
        let err = update_profile(
            &db,
            &store,
            &changed,
            LoginScriptUpdate::from_profile(&changed),
        )
        .unwrap_err();

        assert_eq!(err.code(), "secret_set_failed");
        assert_eq!(
            crate::db::telnet_profile::get(&db, "t1").unwrap().name,
            "Original"
        );
        assert_eq!(
            stored_script(&db, &store, "t1").as_deref(),
            Some("old script")
        );
    }

    #[test]
    fn missing_update_does_not_create_orphan_secret() {
        let db = Db::open_in_memory().unwrap();
        let store = TestStore::default();
        let missing = profile("missing", "Missing", "secret");

        let err = update_profile(
            &db,
            &store,
            &missing,
            LoginScriptUpdate::from_profile(&missing),
        )
        .unwrap_err();

        assert_eq!(err.code(), "telnet_profile_not_found");
        assert!(store.values.lock().unwrap().is_empty());
    }

    #[test]
    fn preserve_moves_legacy_plaintext_before_upsert_clears_column() {
        let db = Db::open_in_memory().unwrap();
        let store = TestStore::default();
        db.with_transaction(|tx| {
            tx.execute(
                "INSERT INTO telnet_profiles (id, name, host, login_script) VALUES ('t1', 'Old', 'h', 'legacy secret')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let incoming = profile("t1", "Remote metadata", "");

        insert_profile(&db, &store, &incoming, LoginScriptUpdate::Preserve).unwrap();

        let state = crate::db::telnet_profile::login_script_state(&db, "t1").unwrap();
        assert!(state.legacy_script.is_empty());
        assert!(!state.legacy_pending);
        assert_eq!(
            stored_script(&db, &store, "t1").as_deref(),
            Some("legacy secret")
        );
        assert_eq!(
            crate::db::settings::get(&db, crate::db::telnet_profile::PURGED_EPOCH_SETTING,)
                .unwrap(),
            crate::db::settings::get(&db, crate::db::telnet_profile::PURGE_EPOCH_SETTING,).unwrap(),
            "runtime reconciliation should attempt the WAL purge immediately",
        );
    }
}
