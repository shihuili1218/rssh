use async_trait::async_trait;
use std::path::Path;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::secret::SecretStore;
use crate::sync::metadata::{adopt_remote_version, refresh_local_metadata, SyncMetadata};

#[async_trait]
pub trait RemoteBackup: Send + Sync {
    async fn read_payload(&self) -> AppResult<String>;
    async fn read_metadata(&self) -> AppResult<Option<SyncMetadata>>;
    async fn write_payload(&self, content: &str) -> AppResult<()>;
    async fn write_metadata(&self, metadata: &SyncMetadata) -> AppResult<()>;
}

pub struct PreparedBackup {
    pub json: String,
    pub metadata: SyncMetadata,
}

/// Build the encrypted-backup input and its plaintext metadata through the
/// same path for GUI and CLI. The two remote writes remain deliberately
/// non-transactional: the product explicitly permits concurrent pushes.
pub fn prepare_backup(
    db: &Db,
    secrets: &dyn SecretStore,
    data_dir: &Path,
) -> AppResult<PreparedBackup> {
    let prefs = crate::sync::config::read_sync_prefs(db)?;
    let payload = crate::sync::config::build_payload(
        db,
        secrets,
        data_dir,
        &crate::sync::config::ExportMode::RemotePush(prefs),
    )?;
    let json = serde_json::to_string_pretty(&payload).map_err(|e| {
        AppError::other("serde_failed", serde_json::json!({ "err": e.to_string() }))
    })?;
    let metadata = refresh_local_metadata(db, data_dir)?;
    Ok(PreparedBackup { json, metadata })
}

/// Keep the established wire order: encrypted backup first, plaintext
/// metadata second. A failed second write leaves the first write intact.
pub async fn publish(
    remote: &dyn RemoteBackup,
    encrypted_payload: &str,
    metadata: &SyncMetadata,
) -> AppResult<()> {
    remote.write_payload(encrypted_payload).await?;
    remote.write_metadata(metadata).await
}

pub struct FetchedBackup {
    pub encrypted_payload: String,
    pub metadata: Option<SyncMetadata>,
}

/// The metadata file is optional for compatibility with old backups. If it is
/// present, malformed content is an error instead of being silently ignored.
pub async fn fetch(remote: &dyn RemoteBackup) -> AppResult<FetchedBackup> {
    let metadata = remote.read_metadata().await?;
    let encrypted_payload = remote.read_payload().await?;
    Ok(FetchedBackup {
        encrypted_payload,
        metadata,
    })
}

/// Apply the existing additive import and then rebase local metadata. A valid
/// remote version is adopted verbatim, including downgrades, as required by
/// the existing push/pull contract.
pub fn apply_fetched_backup(
    db: &Db,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    fetched: FetchedBackup,
    password: &str,
) -> AppResult<SyncMetadata> {
    let json = crate::crypto::decrypt(&fetched.encrypted_payload, password)?;
    let payload: serde_json::Value = serde_json::from_str(&json).map_err(|e| {
        AppError::config(
            "json_parse_failed",
            serde_json::json!({ "err": e.to_string() }),
        )
    })?;

    if let Err(err) = crate::sync::config::merge_import(db, secrets, data_dir, &payload) {
        // merge_import can retain successful rows before returning its
        // aggregate error. Record that actual partial state without granting a
        // failed pull permission to adopt the remote version.
        if let Err(refresh_err) = refresh_local_metadata(db, data_dir) {
            log::warn!("failed to refresh sync metadata after pull error: {refresh_err}");
        }
        return Err(err);
    }

    match fetched.metadata {
        Some(metadata) => adopt_remote_version(db, data_dir, metadata.version),
        None => refresh_local_metadata(db, data_dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppResult;
    use crate::secret::SecretStore;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemSecrets(Mutex<HashMap<String, String>>);

    impl SecretStore for MemSecrets {
        fn get(&self, key: &str) -> AppResult<Option<String>> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: &str) -> AppResult<()> {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }

        fn delete(&self, key: &str) -> AppResult<()> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }

        fn backend_name(&self) -> &'static str {
            "mem"
        }
    }

    #[derive(Default)]
    struct FakeRemote {
        writes: Mutex<Vec<&'static str>>,
        payload: Mutex<Option<String>>,
        metadata: Mutex<Option<SyncMetadata>>,
    }

    #[async_trait::async_trait]
    impl RemoteBackup for FakeRemote {
        async fn read_payload(&self) -> AppResult<String> {
            self.payload
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| AppError::other("test_payload_missing", serde_json::json!({})))
        }

        async fn read_metadata(&self) -> AppResult<Option<SyncMetadata>> {
            Ok(self.metadata.lock().unwrap().clone())
        }

        async fn write_payload(&self, content: &str) -> AppResult<()> {
            self.writes.lock().unwrap().push("payload");
            *self.payload.lock().unwrap() = Some(content.into());
            Ok(())
        }

        async fn write_metadata(&self, metadata: &SyncMetadata) -> AppResult<()> {
            self.writes.lock().unwrap().push("metadata");
            *self.metadata.lock().unwrap() = Some(metadata.clone());
            Ok(())
        }
    }

    fn metadata(version: u64) -> SyncMetadata {
        SyncMetadata {
            version,
            config_digest: format!("sha256:{}", "a".repeat(64)),
        }
    }

    #[tokio::test]
    async fn publish_writes_payload_then_plain_metadata() {
        let remote = FakeRemote::default();
        let metadata = metadata(7);

        publish(&remote, "encrypted", &metadata).await.unwrap();

        assert_eq!(*remote.writes.lock().unwrap(), vec!["payload", "metadata"]);
        assert_eq!(remote.metadata.lock().unwrap().as_ref(), Some(&metadata));
    }

    #[tokio::test]
    async fn fetch_allows_missing_metadata() {
        let remote = FakeRemote::default();
        *remote.payload.lock().unwrap() = Some("legacy-encrypted-payload".into());

        let fetched = fetch(&remote).await.unwrap();

        assert_eq!(fetched.encrypted_payload, "legacy-encrypted-payload");
        assert!(fetched.metadata.is_none());
    }

    #[test]
    fn successful_pull_adopts_lower_remote_version_even_after_additive_merge() {
        let db = Db::open_in_memory().unwrap();
        let secrets = MemSecrets::default();
        let data_dir = tempfile::tempdir().unwrap();
        let payload = crate::sync::config::build_payload(
            &db,
            &secrets,
            data_dir.path(),
            &crate::sync::config::ExportMode::LocalBackup,
        )
        .unwrap();
        let encrypted = crate::crypto::encrypt(&payload.to_string(), "pw").unwrap();
        adopt_remote_version(&db, data_dir.path(), 9).unwrap();

        let local = apply_fetched_backup(
            &db,
            &secrets,
            data_dir.path(),
            FetchedBackup {
                encrypted_payload: encrypted,
                metadata: Some(metadata(3)),
            },
            "pw",
        )
        .unwrap();

        assert_eq!(local.version, 3);
    }

    #[test]
    fn partial_pull_failure_keeps_the_import_error_and_refreshes_local_metadata() {
        let db = Db::open_in_memory().unwrap();
        let secrets = MemSecrets::default();
        let data_dir = tempfile::tempdir().unwrap();
        let before = refresh_local_metadata(&db, data_dir.path()).unwrap();
        let payload = serde_json::json!({
            "version": 1,
            "highlights": [{ "invalid": true }],
            "snippets": [{ "name": "remote", "command": "echo imported" }],
        });
        let encrypted = crate::crypto::encrypt(&payload.to_string(), "pw").unwrap();

        let err = apply_fetched_backup(
            &db,
            &secrets,
            data_dir.path(),
            FetchedBackup {
                encrypted_payload: encrypted,
                metadata: Some(metadata(9)),
            },
            "pw",
        )
        .unwrap_err();

        assert_eq!(err.code(), "import_partial_failed");
        let after = crate::sync::metadata::load_local_metadata(&db)
            .unwrap()
            .unwrap();
        assert_eq!(after.version, before.version + 1);
        assert!(crate::db::snippet::load(data_dir.path())
            .unwrap()
            .iter()
            .any(|snippet| snippet.name == "remote"));
    }
}
