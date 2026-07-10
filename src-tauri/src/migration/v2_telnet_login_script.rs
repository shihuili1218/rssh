//! Move Telnet login scripts out of the plaintext profile row.
//!
//! The encrypted secret is immutable and versioned. SQLite stores only the
//! active version pointer, so SecretStore and profile metadata never need a
//! cross-database rollback: write the new encrypted version first, atomically
//! swing the pointer second, then best-effort delete the old version.

use crate::db::{self, telnet_profile, Db};
use crate::error::{AppError, AppResult};
use crate::secret::{telnet_login_script_key, SecretStore};

const MIGRATION_MARKER: &str = "migration_v2_telnet_login_script_secret";

fn epoch(db: &Db, key: &str) -> AppResult<u64> {
    db::settings::get(db, key)?
        .unwrap_or_else(|| "0".into())
        .parse::<u64>()
        .map_err(|e| {
            AppError::other(
                "telnet_login_script_epoch_invalid",
                serde_json::json!({ "key": key, "err": e.to_string() }),
            )
        })
}

fn delete_version_best_effort(store: &dyn SecretStore, profile_id: &str, version: Option<&str>) {
    let Some(version) = version else { return };
    if let Err(e) = store.delete(&telnet_login_script_key(profile_id, version)) {
        // The DB pointer no longer references this immutable version. A failed
        // cleanup leaves encrypted garbage, never a stale script that can run.
        log::warn!("failed to remove obsolete Telnet login-script version: {e}");
    }
}

/// Reconcile an old client's latest non-empty write. Once the plaintext column
/// is scrubbed, an old client sends the same empty string both when it leaves
/// the script untouched and when the user tries to clear it. That intent is
/// unrepresentable, so empty must preserve the active version; only a non-empty
/// value can replace it. Compare-and-swap prevents a scanned value from
/// clearing a newer concurrent write.
pub(crate) fn reconcile_profile(
    db: &Db,
    store: &dyn SecretStore,
    profile_id: &str,
) -> AppResult<()> {
    reconcile_profile_with_hook(db, store, profile_id, |_| Ok(()))
}

fn reconcile_profile_with_hook(
    db: &Db,
    store: &dyn SecretStore,
    profile_id: &str,
    mut before_commit: impl FnMut(&telnet_profile::LoginScriptState) -> AppResult<()>,
) -> AppResult<()> {
    loop {
        let state = telnet_profile::login_script_state(db, profile_id)?;
        if !state.legacy_pending {
            return Ok(());
        }

        let created_version = if state.legacy_script.is_empty() {
            // Defense in depth for rows marked pending by an older trigger:
            // clear the marker while retaining the immutable active pointer.
            None
        } else {
            let version = uuid::Uuid::new_v4().to_string();
            store.set(
                &telnet_login_script_key(profile_id, &version),
                &state.legacy_script,
            )?;
            Some(version)
        };
        let target_version = if state.legacy_script.is_empty() {
            state.version.as_deref()
        } else {
            created_version.as_deref()
        };

        before_commit(&state)?;
        let committed =
            telnet_profile::commit_legacy_login_script(db, profile_id, &state, target_version)?;
        if !committed {
            // Only a UUID allocated and stored by this iteration is garbage on
            // CAS miss. In the empty-Preserve branch target_version is the
            // active pointer; deleting it would destroy the live secret.
            delete_version_best_effort(store, profile_id, created_version.as_deref());
            continue;
        }

        if state.version.as_deref() != target_version {
            delete_version_best_effort(store, profile_id, state.version.as_deref());
        }
        return Ok(());
    }
}

/// Truncate WAL pages after secure-delete rewrites. Monotonic epochs avoid the
/// lost-wakeup race of a boolean pending marker: a clear concurrent with the
/// checkpoint advances `purge_epoch`, so `purged_epoch` remains behind and the
/// next startup retries.
pub(crate) fn finish_pending_purge(db: &Db) -> AppResult<()> {
    let pending = epoch(db, telnet_profile::PURGE_EPOCH_SETTING)?;
    let purged = epoch(db, telnet_profile::PURGED_EPOCH_SETTING)?;
    if pending <= purged {
        return Ok(());
    }
    db.checkpoint_truncate()?;
    db::settings::set(
        db,
        telnet_profile::PURGED_EPOCH_SETTING,
        &pending.to_string(),
    )
}

/// Runtime reconciliation must not leave scrubbed plaintext sitting in WAL
/// until the next process restart. A busy checkpoint is still non-fatal: the
/// monotonic epochs retain the retry for the next profile access/startup.
pub(crate) fn retry_pending_purge(db: &Db) {
    if let Err(e) = finish_pending_purge(db) {
        log::warn!("deferred Telnet login-script WAL cleanup: {e}");
    }
}

pub fn run(db: &Db, store: &dyn SecretStore) -> AppResult<()> {
    loop {
        let pending = telnet_profile::list_pending_legacy_login_scripts(db)?;
        if pending.is_empty() {
            break;
        }
        for (id, _) in pending {
            match reconcile_profile(db, store, &id) {
                Ok(()) => {}
                // A concurrent delete already removed the only state to migrate.
                Err(e) if e.code() == "telnet_profile_not_found" => {}
                Err(e) => return Err(e),
            }
        }
    }
    if db::settings::get(db, MIGRATION_MARKER)?.as_deref() != Some("1") {
        db::settings::set(db, MIGRATION_MARKER, "1")?;
    }
    finish_pending_purge(db)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::secret::DbStore;

    fn raw_write(db: &Db, sql: &str) {
        db.with_transaction(|tx| {
            tx.execute_batch(sql)?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn migrates_plaintext_then_clears_profile_column() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        raw_write(
            &db,
            "INSERT INTO telnet_profiles \
             (id, name, host, login_script, echo_write_version) \
             VALUES ('t1', 'switch', '10.0.0.1', 'expect Password:\nsend hunter2', 0);",
        );
        let store = DbStore::new(db.clone());

        run(&db, &store).unwrap();

        let state = telnet_profile::login_script_state(&db, "t1").unwrap();
        assert!(!state.legacy_pending);
        assert!(state.legacy_script.is_empty());
        let version = state.version.unwrap();
        assert_eq!(
            store
                .get(&telnet_login_script_key("t1", &version))
                .unwrap()
                .as_deref(),
            Some("expect Password:\nsend hunter2")
        );
        run(&db, &store).unwrap();
    }

    #[test]
    fn migrates_old_client_writeback_after_marker() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        raw_write(
            &db,
            "INSERT INTO telnet_profiles \
             (id, name, host, login_script, echo_write_version) \
             VALUES ('t1', 'switch', 'h', 'old', 0);",
        );
        let store = DbStore::new(db.clone());
        run(&db, &store).unwrap();

        raw_write(
            &db,
            "UPDATE telnet_profiles SET login_script = 'new' WHERE id = 't1';",
        );
        run(&db, &store).unwrap();

        let state = telnet_profile::login_script_state(&db, "t1").unwrap();
        let version = state.version.unwrap();
        assert_eq!(
            store
                .get(&telnet_login_script_key("t1", &version))
                .unwrap()
                .as_deref(),
            Some("new")
        );
    }

    #[test]
    fn old_client_empty_write_preserves_the_active_script() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        raw_write(
            &db,
            "INSERT INTO telnet_profiles \
             (id, name, host, login_script, echo_write_version) \
             VALUES ('t1', 'switch', 'h', 'old', 0);",
        );
        let store = DbStore::new(db.clone());
        run(&db, &store).unwrap();
        let original_version = telnet_profile::login_script_state(&db, "t1")
            .unwrap()
            .version
            .unwrap();

        // The old writer cannot distinguish an untouched scrubbed field from
        // an explicit clear, so empty is conservatively Preserve.
        raw_write(
            &db,
            "UPDATE telnet_profiles SET login_script = '' WHERE id = 't1';",
        );
        run(&db, &store).unwrap();

        let state = telnet_profile::login_script_state(&db, "t1").unwrap();
        assert!(!state.legacy_pending);
        assert_eq!(state.version.as_deref(), Some(original_version.as_str()));
        assert_eq!(
            store
                .get(&telnet_login_script_key("t1", &original_version))
                .unwrap()
                .as_deref(),
            Some("old")
        );
    }

    #[test]
    fn empty_preserve_cas_miss_never_deletes_the_active_version() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        raw_write(
            &db,
            "INSERT INTO telnet_profiles
             (id, name, host, login_script, login_script_version,
              login_script_legacy_pending, echo_write_version)
             VALUES ('t1', 'switch', 'h', '', 'active-v1', 1, 1);",
        );
        let store = DbStore::new(db.clone());
        store
            .set(&telnet_login_script_key("t1", "active-v1"), "keep me")
            .unwrap();
        let raced = AtomicBool::new(false);

        reconcile_profile_with_hook(&db, &store, "t1", |_| {
            if !raced.swap(true, Ordering::SeqCst) {
                // Simulate a concurrent canonical writer resolving the pending
                // marker before this iteration's compare-and-swap.
                raw_write(
                    &db,
                    "UPDATE telnet_profiles
                     SET login_script_legacy_pending = 0 WHERE id = 't1';",
                );
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(
            store
                .get(&telnet_login_script_key("t1", "active-v1"))
                .unwrap()
                .as_deref(),
            Some("keep me")
        );
        assert_eq!(
            telnet_profile::login_script_state(&db, "t1")
                .unwrap()
                .version
                .as_deref(),
            Some("active-v1")
        );
    }

    #[test]
    fn compare_and_swap_does_not_clear_a_newer_legacy_write() {
        let db = Db::open_in_memory().unwrap();
        raw_write(
            &db,
            "INSERT INTO telnet_profiles \
             (id, name, host, login_script, echo_write_version) \
             VALUES ('t1', 'switch', 'h', 'old', 0);",
        );
        let stale = telnet_profile::login_script_state(&db, "t1").unwrap();
        raw_write(
            &db,
            "UPDATE telnet_profiles SET login_script = 'new' WHERE id = 't1';",
        );

        assert!(!telnet_profile::commit_legacy_login_script(
            &db,
            "t1",
            &stale,
            Some("stale-version"),
        )
        .unwrap());
        let current = telnet_profile::login_script_state(&db, "t1").unwrap();
        assert_eq!(current.legacy_script, "new");
        assert!(current.legacy_pending);
    }
}
