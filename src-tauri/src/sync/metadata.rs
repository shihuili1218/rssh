use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::secret::SecretStore;

const LOCAL_METADATA_KEY: &str = "sync_local_metadata";
static LOCAL_STATE_GATE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncMetadata {
    pub version: u64,
    pub config_digest: String,
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

impl SyncMetadata {
    fn validate(&self) -> AppResult<()> {
        if self.version == 0 || !valid_sha256(&self.config_digest) {
            return Err(AppError::config(
                "sync_metadata_invalid",
                serde_json::json!({}),
            ));
        }
        Ok(())
    }

    pub fn from_json(raw: &str) -> AppResult<Self> {
        let metadata: Self = serde_json::from_str(raw).map_err(|e| {
            AppError::config(
                "sync_metadata_invalid",
                serde_json::json!({ "err": e.to_string() }),
            )
        })?;
        metadata.validate()?;
        Ok(metadata)
    }

    pub fn to_json(&self) -> AppResult<String> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|e| {
            AppError::other("serde_failed", serde_json::json!({ "err": e.to_string() }))
        })
    }
}

struct CiphertextStore<'a> {
    db: &'a Db,
}

impl SecretStore for CiphertextStore<'_> {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        crate::db::secret::get(self.db, key)
    }

    fn set(&self, _key: &str, _value: &str) -> AppResult<()> {
        Err(AppError::other(
            "sync_fingerprint_secret_store_read_only",
            serde_json::json!({}),
        ))
    }

    fn delete(&self, _key: &str) -> AppResult<()> {
        Err(AppError::other(
            "sync_fingerprint_secret_store_read_only",
            serde_json::json!({}),
        ))
    }

    fn backend_name(&self) -> &'static str {
        "ciphertext"
    }
}

fn sort_unordered_categories(payload: &mut serde_json::Value) {
    const UNORDERED: &[&str] = &[
        "profiles",
        "credentials",
        "forwards",
        "serial_profiles",
        "telnet_profiles",
        "skills",
        "highlights",
        "ai_command_blacklist",
    ];
    for key in UNORDERED {
        let Some(items) = payload
            .get_mut(*key)
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        items.sort_by_cached_key(|item| serde_json::to_string(item).unwrap_or_default());
    }
}

fn current_digest(db: &Db, data_dir: &Path) -> AppResult<String> {
    let prefs = crate::sync::config::read_sync_prefs(db)?;
    let ciphertexts = CiphertextStore { db };
    let mut payload =
        crate::sync::config::build_fingerprint_payload(db, &ciphertexts, data_dir, prefs)?;
    sort_unordered_categories(&mut payload);
    let bytes = serde_json::to_vec(&payload).map_err(|e| {
        AppError::other("serde_failed", serde_json::json!({ "err": e.to_string() }))
    })?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

fn stored_metadata(db: &Db) -> AppResult<Option<SyncMetadata>> {
    let Some(raw) = crate::db::settings::get(db, LOCAL_METADATA_KEY)? else {
        return Ok(None);
    };
    SyncMetadata::from_json(&raw).map(Some).map_err(|e| {
        AppError::config(
            "sync_local_metadata_invalid",
            serde_json::json!({ "err": e.to_string() }),
        )
    })
}

/// Read the last metadata snapshot without rebuilding the configuration
/// fingerprint. The staged remote check uses exactly the snapshot published by
/// the preceding local refresh, so network work cannot delay that local result.
pub(crate) fn load_local_metadata(db: &Db) -> AppResult<Option<SyncMetadata>> {
    stored_metadata(db)
}

fn persist(db: &Db, metadata: &SyncMetadata) -> AppResult<()> {
    let raw = metadata.to_json()?;
    crate::db::settings::set(db, LOCAL_METADATA_KEY, &raw)
}

fn refresh_local_metadata_unlocked(db: &Db, data_dir: &Path) -> AppResult<SyncMetadata> {
    let config_digest = current_digest(db, data_dir)?;
    let previous = stored_metadata(db)?;
    let version = match previous {
        None => 1,
        Some(previous) if previous.config_digest == config_digest => previous.version.max(1),
        Some(previous) => previous.version.max(1).saturating_add(1),
    };
    let metadata = SyncMetadata {
        version,
        config_digest,
    };
    persist(db, &metadata)?;
    Ok(metadata)
}

pub fn refresh_local_metadata(db: &Db, data_dir: &Path) -> AppResult<SyncMetadata> {
    let _guard = crate::error::locked(&LOCAL_STATE_GATE)?;
    refresh_local_metadata_unlocked(db, data_dir)
}

/// Rebase the local digest on the configuration that exists *after* a pull,
/// while taking the caller-provided version verbatim. Manual pull deliberately
/// permits both upgrades and downgrades.
pub fn adopt_remote_version(db: &Db, data_dir: &Path, version: u64) -> AppResult<SyncMetadata> {
    let _guard = crate::error::locked(&LOCAL_STATE_GATE)?;
    let metadata = SyncMetadata {
        version,
        config_digest: current_digest(db, data_dir)?,
    };
    persist(db, &metadata)?;
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::models::{Credential, CredentialType};
    use crate::secret::cred_secret_key;

    #[test]
    fn first_refresh_starts_at_v1_and_stays_stable() {
        let db = Db::open_in_memory().unwrap();
        let data_dir = tempfile::tempdir().unwrap();

        let first = refresh_local_metadata(&db, data_dir.path()).unwrap();
        let second = refresh_local_metadata(&db, data_dir.path()).unwrap();

        assert_eq!(first.version, 1);
        assert_eq!(second, first);
        assert!(first.config_digest.starts_with("sha256:"));
    }

    #[test]
    fn config_change_increments_once() {
        let db = Db::open_in_memory().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        assert_eq!(
            refresh_local_metadata(&db, data_dir.path())
                .unwrap()
                .version,
            1
        );

        crate::db::settings::set(&db, "sync_include_snippets", "0").unwrap();
        let changed = refresh_local_metadata(&db, data_dir.path()).unwrap();
        let stable = refresh_local_metadata(&db, data_dir.path()).unwrap();

        assert_eq!(changed.version, 2);
        assert_eq!(stable, changed);
    }

    #[test]
    fn loading_persisted_metadata_does_not_recompute_the_digest() {
        let db = Db::open_in_memory().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let first = refresh_local_metadata(&db, data_dir.path()).unwrap();

        crate::db::settings::set(&db, "sync_include_highlights", "0").unwrap();

        assert_eq!(load_local_metadata(&db).unwrap(), Some(first));
    }

    #[test]
    fn adopting_remote_version_allows_upgrade_and_downgrade() {
        let db = Db::open_in_memory().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        refresh_local_metadata(&db, data_dir.path()).unwrap();

        crate::db::settings::set(&db, "sync_include_snippets", "0").unwrap();
        let upgraded = adopt_remote_version(&db, data_dir.path(), 9).unwrap();
        assert_eq!(upgraded.version, 9);
        assert_eq!(
            refresh_local_metadata(&db, data_dir.path()).unwrap(),
            upgraded
        );

        crate::db::settings::set(&db, "sync_include_highlights", "0").unwrap();
        let downgraded = adopt_remote_version(&db, data_dir.path(), 3).unwrap();
        assert_eq!(downgraded.version, 3);
        assert_eq!(
            refresh_local_metadata(&db, data_dir.path()).unwrap(),
            downgraded
        );
    }

    #[test]
    fn remote_secret_ciphertext_changes_digest_but_local_only_secret_does_not() {
        let db = Db::open_in_memory().unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let credential = |id: &str, save_to_remote: bool| Credential {
            id: id.into(),
            name: id.into(),
            username: "user".into(),
            credential_type: CredentialType::Password,
            secret: None,
            save_to_remote,
        };
        crate::db::credential::insert(&db, &credential("remote", true)).unwrap();
        crate::db::credential::insert(&db, &credential("local", false)).unwrap();
        crate::db::secret::set(&db, &cred_secret_key("remote"), "enc:v1:first").unwrap();
        crate::db::secret::set(&db, &cred_secret_key("local"), "enc:v1:first").unwrap();
        let initial = refresh_local_metadata(&db, data_dir.path()).unwrap();

        crate::db::secret::set(&db, &cred_secret_key("local"), "enc:v1:second").unwrap();
        assert_eq!(
            refresh_local_metadata(&db, data_dir.path()).unwrap(),
            initial,
            "a secret excluded from remote sync is not part of its digest"
        );

        crate::db::secret::set(&db, &cred_secret_key("remote"), "enc:v1:second").unwrap();
        let changed = refresh_local_metadata(&db, data_dir.path()).unwrap();
        assert_eq!(changed.version, initial.version + 1);
        assert_ne!(changed.config_digest, initial.config_digest);
    }

    #[test]
    fn metadata_json_round_trips_and_rejects_invalid_digest() {
        let metadata = SyncMetadata {
            version: 6,
            config_digest:
                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
        };
        assert_eq!(
            SyncMetadata::from_json(&metadata.to_json().unwrap()).unwrap(),
            metadata
        );

        let err =
            SyncMetadata::from_json(r#"{"version":6,"config_digest":"plaintext"}"#).unwrap_err();
        assert_eq!(err.code(), "sync_metadata_invalid");
    }
}
