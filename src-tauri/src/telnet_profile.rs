//! Transport-neutral Telnet profile persistence.
//!
//! Profile metadata lives in SQLite while login scripts live in `SecretStore`
//! as immutable versions. This module owns the ordering between those stores:
//! write the new secret, atomically swing SQLite's version pointer, then
//! best-effort delete the obsolete version.

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::models::TelnetProfile;
use crate::secret::{telnet_login_script_key, SecretStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginScriptIntent {
    /// A scrubbed sync payload: retain this device's existing secret.
    Preserve,
    Set(String),
    Delete,
}

impl LoginScriptIntent {
    /// A direct profile edit replaces the complete script value; unlike a
    /// scrubbed sync payload, an empty value is an intentional deletion.
    pub fn from_profile(profile: &TelnetProfile) -> Self {
        if profile.login_script.is_empty() {
            Self::Delete
        } else {
            Self::Set(profile.login_script.clone())
        }
    }
}

/// List profile metadata without decrypting login scripts.
pub fn list_metadata(db: &Db) -> AppResult<Vec<TelnetProfile>> {
    crate::db::telnet_profile::list(db)
}

/// Load one profile and resolve its active immutable login-script version.
pub fn get_full(db: &Db, store: &dyn SecretStore, id: &str) -> AppResult<TelnetProfile> {
    let mut profile = crate::db::telnet_profile::get(db, id)?;
    hydrate(db, store, &mut profile)?;
    Ok(profile)
}

/// Resolve a metadata-only profile's active login script in place.
pub fn hydrate(db: &Db, store: &dyn SecretStore, profile: &mut TelnetProfile) -> AppResult<()> {
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

#[derive(Clone, Copy)]
enum WriteMode {
    Upsert,
    Update,
}

fn persist_profile(
    db: &Db,
    profile: &TelnetProfile,
    script: crate::db::telnet_profile::LoginScriptVersionUpdate,
    mode: WriteMode,
) -> AppResult<Option<String>> {
    match mode {
        WriteMode::Upsert => {
            crate::db::telnet_profile::insert_with_script_version(db, profile, script)
        }
        WriteMode::Update => {
            crate::db::telnet_profile::update_with_script_version(db, profile, script)
        }
    }
}

fn replace_script(
    db: &Db,
    store: &dyn SecretStore,
    profile: &TelnetProfile,
    script: crate::db::telnet_profile::LoginScriptVersionUpdate,
    new_version: Option<String>,
    mode: WriteMode,
) -> AppResult<()> {
    let old_version = match persist_profile(db, profile, script, mode) {
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

fn write_profile(
    db: &Db,
    store: &dyn SecretStore,
    profile: &TelnetProfile,
    intent: LoginScriptIntent,
    mode: WriteMode,
) -> AppResult<()> {
    crate::db::telnet_profile::validate(profile)?;

    match intent {
        LoginScriptIntent::Preserve => {
            if optional_profile(db, &profile.id)?.is_some() {
                crate::migration::v2_telnet_login_script::reconcile_profile(
                    db,
                    store,
                    &profile.id,
                )?;
                crate::migration::v2_telnet_login_script::retry_pending_purge(db);
            }
            persist_profile(
                db,
                profile,
                crate::db::telnet_profile::LoginScriptVersionUpdate::Preserve,
                mode,
            )?;
            Ok(())
        }
        LoginScriptIntent::Set(value) => {
            let version = uuid::Uuid::new_v4().to_string();
            store.set(&telnet_login_script_key(&profile.id, &version), &value)?;
            replace_script(
                db,
                store,
                profile,
                crate::db::telnet_profile::LoginScriptVersionUpdate::Set(version.clone()),
                Some(version),
                mode,
            )
        }
        LoginScriptIntent::Delete => replace_script(
            db,
            store,
            profile,
            crate::db::telnet_profile::LoginScriptVersionUpdate::Delete,
            None,
            mode,
        ),
    }
}

pub fn upsert(
    db: &Db,
    store: &dyn SecretStore,
    profile: &TelnetProfile,
    intent: LoginScriptIntent,
) -> AppResult<()> {
    write_profile(db, store, profile, intent, WriteMode::Upsert)
}

pub fn update(
    db: &Db,
    store: &dyn SecretStore,
    profile: &TelnetProfile,
    intent: LoginScriptIntent,
) -> AppResult<()> {
    write_profile(db, store, profile, intent, WriteMode::Update)
}

pub fn delete(db: &Db, store: &dyn SecretStore, id: &str) -> AppResult<()> {
    let old_version = crate::db::telnet_profile::delete_with_script_version(db, id)?;
    delete_version_best_effort(store, id, old_version.as_deref());
    Ok(())
}

#[cfg(test)]
mod tests {
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
        upsert(
            &db,
            &store,
            &original,
            LoginScriptIntent::from_profile(&original),
        )
        .unwrap();

        store.fail_set.store(true, Ordering::Relaxed);
        let changed = profile("t1", "Changed", "new script");
        let err = update(
            &db,
            &store,
            &changed,
            LoginScriptIntent::from_profile(&changed),
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

        let err = update(
            &db,
            &store,
            &missing,
            LoginScriptIntent::from_profile(&missing),
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

        upsert(&db, &store, &incoming, LoginScriptIntent::Preserve).unwrap();

        let state = crate::db::telnet_profile::login_script_state(&db, "t1").unwrap();
        assert!(state.legacy_script.is_empty());
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
