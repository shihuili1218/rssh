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

/// Whether an update payload carries a complete login-script replacement or
/// only scrubbed profile metadata. Keeping this separate from the script value
/// makes an empty replacement (delete) distinct from an omitted secret
/// (preserve).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginScriptUpdate {
    Preserve,
    Replace,
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

    pub fn from_update_profile(profile: &TelnetProfile, update: Option<LoginScriptUpdate>) -> Self {
        match update {
            Some(LoginScriptUpdate::Preserve) => Self::Preserve,
            Some(LoginScriptUpdate::Replace) => Self::from_profile(profile),
            None if profile.login_script.is_empty() => Self::Preserve,
            None => Self::from_profile(profile),
        }
    }
}

/// List profile metadata without decrypting login scripts.
pub fn list_metadata(db: &Db) -> AppResult<Vec<TelnetProfile>> {
    crate::db::telnet_profile::list(db)
}

/// Load one profile and resolve its active immutable login-script version.
pub fn get_full(db: &Db, store: &dyn SecretStore, id: &str) -> AppResult<TelnetProfile> {
    reconcile_legacy_plaintext(db, store, id)?;
    load_full_snapshot(db, store, id)
}

/// Resolve a metadata-only profile's active login script in place.
pub fn hydrate(db: &Db, store: &dyn SecretStore, profile: &mut TelnetProfile) -> AppResult<()> {
    let id = profile.id.clone();
    reconcile_legacy_plaintext(db, store, &id)?;
    *profile = load_full_snapshot(db, store, &id)?;
    Ok(())
}

/// Consume any old-client plaintext write without decrypting the resulting
/// secret. Remote sync uses this before emitting a scrubbed local-only field.
pub(crate) fn reconcile_legacy_plaintext(
    db: &Db,
    store: &dyn SecretStore,
    id: &str,
) -> AppResult<()> {
    crate::migration::v2_telnet_login_script::reconcile_profile(db, store, id)?;
    crate::migration::v2_telnet_login_script::retry_pending_purge(db);
    Ok(())
}

fn load_full_snapshot(db: &Db, store: &dyn SecretStore, id: &str) -> AppResult<TelnetProfile> {
    loop {
        let snapshot = crate::db::telnet_profile::snapshot(db, id)?;
        if !snapshot.login_script.legacy_script.is_empty() {
            reconcile_legacy_plaintext(db, store, id)?;
            continue;
        }
        let Some(version) = snapshot.login_script.version.as_deref() else {
            return Ok(snapshot.metadata);
        };
        let key = telnet_login_script_key(id, version);
        if let Some(script) = store.get(&key)? {
            // The metadata and pointer came from one SQLite snapshot, and the
            // pointed-to immutable value still existed. That complete older
            // generation is a valid linearized read even if a newer pointer
            // was published concurrently; no second snapshot comparison is
            // needed on this successful path.
            let mut profile = snapshot.metadata;
            profile.login_script = script;
            return Ok(profile);
        }

        // A writer publishes the new immutable version before swinging the DB
        // pointer, then may immediately delete the old version. Missing data is
        // therefore retried only when a fresh *complete* snapshot shows that
        // the pointer moved; a stable missing pointer is real corruption.
        let current = crate::db::telnet_profile::snapshot(db, id)?;
        if current.login_script == snapshot.login_script {
            return Err(AppError::other(
                "telnet_login_script_missing",
                serde_json::json!({ "id": id, "version": version }),
            ));
        }
    }
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
    crate::migration::v2_telnet_login_script::retry_pending_purge(db);
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
    crate::migration::v2_telnet_login_script::retry_pending_purge(db);
    delete_version_best_effort(store, id, old_version.as_deref());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

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
    fn scrubbed_update_without_explicit_script_update_preserves_secret() {
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

        let mut scrubbed = list_metadata(&db).unwrap().remove(0);
        scrubbed.name = "Renamed".into();
        let intent = LoginScriptIntent::from_update_profile(&scrubbed, None);
        update(&db, &store, &scrubbed, intent).unwrap();

        assert_eq!(
            stored_script(&db, &store, "t1").as_deref(),
            Some("old script")
        );
        assert_eq!(
            crate::db::telnet_profile::get(&db, "t1").unwrap().name,
            "Renamed"
        );
    }

    #[test]
    fn explicit_replace_can_delete_login_script() {
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

        let cleared = profile("t1", "Original", "");
        let intent =
            LoginScriptIntent::from_update_profile(&cleared, Some(LoginScriptUpdate::Replace));
        update(&db, &store, &cleared, intent).unwrap();

        assert_eq!(stored_script(&db, &store, "t1"), None);
    }

    #[test]
    fn legacy_nonempty_update_still_replaces_login_script() {
        let changed = profile("t1", "Changed", "new script");

        assert_eq!(
            LoginScriptIntent::from_update_profile(&changed, None),
            LoginScriptIntent::Set("new script".into())
        );
    }

    fn assert_plaintext_purge_finished(db: &Db) {
        assert_eq!(
            crate::db::settings::get(db, crate::db::telnet_profile::PURGED_EPOCH_SETTING).unwrap(),
            crate::db::settings::get(db, crate::db::telnet_profile::PURGE_EPOCH_SETTING).unwrap(),
        );
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

    #[test]
    fn set_attempts_plaintext_purge_immediately() {
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
        let changed = profile("t1", "Changed", "new secret");

        upsert(
            &db,
            &store,
            &changed,
            LoginScriptIntent::Set("new secret".into()),
        )
        .unwrap();

        assert_plaintext_purge_finished(&db);
    }

    #[test]
    fn delete_intent_attempts_plaintext_purge_immediately() {
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
        let changed = profile("t1", "Changed", "");

        upsert(&db, &store, &changed, LoginScriptIntent::Delete).unwrap();

        assert_plaintext_purge_finished(&db);
    }

    #[test]
    fn profile_delete_attempts_plaintext_purge_immediately() {
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

        delete(&db, &store, "t1").unwrap();

        assert_plaintext_purge_finished(&db);
    }

    struct SwingOnReadStore {
        db: Arc<Db>,
        values: Mutex<HashMap<String, String>>,
        swung: AtomicBool,
    }

    impl SecretStore for SwingOnReadStore {
        fn get(&self, key: &str) -> AppResult<Option<String>> {
            if key == telnet_login_script_key("t1", "old-version")
                && !self.swung.swap(true, Ordering::SeqCst)
            {
                let new_key = telnet_login_script_key("t1", "new-version");
                self.values
                    .lock()
                    .unwrap()
                    .insert(new_key, "new script".into());
                let changed = profile("t1", "New metadata", "");
                crate::db::telnet_profile::update_with_script_version(
                    &self.db,
                    &changed,
                    crate::db::telnet_profile::LoginScriptVersionUpdate::Set("new-version".into()),
                )?;
                self.values.lock().unwrap().remove(key);
            }
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: &str) -> AppResult<()> {
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
            "swing-on-read"
        }
    }

    #[test]
    fn full_read_retries_after_pointer_swing_deletes_old_secret() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        crate::db::telnet_profile::insert_with_script_version(
            &db,
            &profile("t1", "Old metadata", ""),
            crate::db::telnet_profile::LoginScriptVersionUpdate::Set("old-version".into()),
        )
        .unwrap();
        let store = SwingOnReadStore {
            db: db.clone(),
            values: Mutex::new(HashMap::from([(
                telnet_login_script_key("t1", "old-version"),
                "old script".into(),
            )])),
            swung: AtomicBool::new(false),
        };

        let loaded = get_full(&db, &store, "t1").unwrap();

        assert_eq!(loaded.name, "New metadata");
        assert_eq!(loaded.login_script, "new script");
    }
}
