use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;

use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::secret::{setting_key, SecretStore};
use crate::state::AppState;
use crate::sync::config::{build_payload, read_sync_prefs, ExportMode};
use crate::sync::metadata::{
    adopt_remote_version, load_local_metadata, refresh_local_metadata, SyncMetadata,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncProvider {
    Github,
    Webdav,
}

impl SyncProvider {
    fn auto_pull_setting(self) -> &'static str {
        match self {
            Self::Github => "sync_github_auto_pull",
            Self::Webdav => "sync_webdav_auto_pull",
        }
    }

    fn auto_pull_password_setting(self) -> &'static str {
        match self {
            Self::Github => "sync_github_auto_pull_password",
            Self::Webdav => "sync_webdav_auto_pull_password",
        }
    }
}

fn auto_pull_password_key(provider: SyncProvider) -> String {
    setting_key(provider.auto_pull_password_setting())
}

fn auto_pull_enabled(db: &Db, provider: SyncProvider) -> AppResult<bool> {
    Ok(crate::db::settings::get(db, provider.auto_pull_setting())?.as_deref() == Some("1"))
}

fn set_auto_pull(
    db: &Db,
    ss: &dyn SecretStore,
    provider: SyncProvider,
    enabled: bool,
    password: Option<&str>,
) -> AppResult<()> {
    if enabled {
        let password = password
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::config("sync_password_empty", serde_json::json!({})))?;
        ss.set(&auto_pull_password_key(provider), password)?;
        crate::db::settings::set(db, provider.auto_pull_setting(), "1")
    } else {
        // The flag is the commit marker read by both the frontend and probes.
        // Do the fallible secret cleanup first so a failed command cannot leave
        // persisted OFF behind while the frontend correctly retains ON.
        ss.delete(&auto_pull_password_key(provider))?;
        crate::db::settings::set(db, provider.auto_pull_setting(), "0")
    }
}

#[tauri::command]
pub fn set_sync_auto_pull(
    state: State<'_, AppState>,
    provider: SyncProvider,
    enabled: bool,
    password: Option<String>,
) -> AppResult<()> {
    set_sync_auto_pull_impl(&state, provider, enabled, password)
}

pub fn set_sync_auto_pull_impl(
    state: &AppState,
    provider: SyncProvider,
    enabled: bool,
    password: Option<String>,
) -> AppResult<()> {
    set_auto_pull(
        &state.db,
        state.secret_store.as_ref(),
        provider,
        enabled,
        password.as_deref(),
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderSyncStatus {
    pub enabled: bool,
    pub auto_pull: bool,
    pub remote: Option<SyncMetadata>,
    pub error: Option<String>,
    pub pulled: bool,
}

impl ProviderSyncStatus {
    fn new(enabled: bool, auto_pull: bool) -> Self {
        Self {
            enabled,
            auto_pull,
            remote: None,
            error: None,
            pulled: false,
        }
    }
}

struct ProviderProbe {
    status: ProviderSyncStatus,
    password: Option<String>,
}

impl ProviderProbe {
    fn remote_is_newer_than(&self, local: &SyncMetadata) -> bool {
        self.status.auto_pull
            && self
                .status
                .remote
                .as_ref()
                .is_some_and(|remote| remote.version > local.version)
    }
}

async fn join_provider_probes<G, W>(github: G, webdav: W) -> (G::Output, W::Output)
where
    G: std::future::Future,
    W: std::future::Future,
{
    tokio::join!(github, webdav)
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncCheckResult {
    pub local: SyncMetadata,
    pub github: ProviderSyncStatus,
    pub webdav: ProviderSyncStatus,
}

#[tauri::command]
pub async fn sync_check(state: State<'_, AppState>) -> AppResult<SyncCheckResult> {
    sync_check_impl(&state).await
}

pub async fn sync_check_impl(state: &AppState) -> AppResult<SyncCheckResult> {
    sync_refresh_local_metadata_impl(state).await?;
    sync_check_remotes_impl(state).await
}

#[tauri::command]
pub async fn sync_refresh_local_metadata(state: State<'_, AppState>) -> AppResult<SyncMetadata> {
    sync_refresh_local_metadata_impl(&state).await
}

pub async fn sync_refresh_local_metadata_impl(state: &AppState) -> AppResult<SyncMetadata> {
    refresh_local_metadata_blocking(state).await
}

#[tauri::command]
pub async fn sync_check_remotes(state: State<'_, AppState>) -> AppResult<SyncCheckResult> {
    sync_check_remotes_impl(&state).await
}

pub async fn sync_check_remotes_impl(state: &AppState) -> AppResult<SyncCheckResult> {
    let local = load_or_refresh_local_metadata_blocking(state).await?;
    // The probes only read settings/secrets and fetch the small metadata file,
    // so they are safe to overlap. Automatic pulls remain ordered below: both
    // import configuration and rewrite local metadata.
    let (github_probe, webdav_probe) =
        join_provider_probes(probe_github(state), probe_webdav(state)).await;
    let (github, local) = maybe_auto_pull_github(state, github_probe, local).await;
    let (webdav, local) = maybe_auto_pull_webdav(state, webdav_probe, local).await;
    Ok(SyncCheckResult {
        local,
        github,
        webdav,
    })
}

async fn probe_github(state: &AppState) -> ProviderProbe {
    use crate::sync::github::GitHubSync;

    let runtime = run_db_blocking(state, |db, ss| {
        let enabled = crate::db::settings::get(&db, "sync_github_enabled")?.as_deref() != Some("0");
        let auto_pull = auto_pull_enabled(&db, SyncProvider::Github)?;
        let password = if auto_pull {
            ss.get(&auto_pull_password_key(SyncProvider::Github))?
        } else {
            None
        };
        let sync = if enabled {
            match (
                ss.get(&setting_key("github_token"))?,
                crate::db::settings::get(&db, "github_repo")?,
            ) {
                (Some(token), Some(repo)) if !token.is_empty() && !repo.is_empty() => {
                    let branch = crate::db::settings::get(&db, "github_branch")?
                        .unwrap_or_else(|| "main".into());
                    Some(GitHubSync::from_settings(&token, &repo, &branch)?)
                }
                _ => None,
            }
        } else {
            None
        };
        Ok((enabled, auto_pull, password, sync))
    })
    .await;

    let (enabled, auto_pull, password, sync) = match runtime {
        Ok(runtime) => runtime,
        Err(err) => {
            let mut status = ProviderSyncStatus::new(true, false);
            status.error = Some(err.to_string());
            return ProviderProbe {
                status,
                password: None,
            };
        }
    };
    let Some(sync) = sync else {
        return ProviderProbe {
            status: ProviderSyncStatus::new(enabled, auto_pull),
            password,
        };
    };

    let mut status = ProviderSyncStatus::new(enabled, auto_pull);
    let remote = match sync.pull_metadata().await {
        Ok(remote) => remote,
        Err(err) => {
            status.error = Some(err.to_string());
            return ProviderProbe { status, password };
        }
    };
    status.remote = remote;
    ProviderProbe { status, password }
}

async fn maybe_auto_pull_github(
    state: &AppState,
    mut probe: ProviderProbe,
    local: SyncMetadata,
) -> (ProviderSyncStatus, SyncMetadata) {
    if !probe.remote_is_newer_than(&local) {
        return (probe.status, local);
    }
    let Some(password) = probe.password else {
        probe.status.error =
            Some(AppError::config("sync_auto_pull_password_missing", json!({})).to_string());
        return (probe.status, local);
    };
    match github_pull_impl(state, password).await {
        Ok(()) => match refresh_local_metadata_blocking(state).await {
            Ok(local) => {
                probe.status.pulled = true;
                (probe.status, local)
            }
            Err(err) => {
                probe.status.error = Some(err.to_string());
                (probe.status, local)
            }
        },
        Err(err) => {
            probe.status.error = Some(err.to_string());
            (probe.status, local)
        }
    }
}

async fn probe_webdav(state: &AppState) -> ProviderProbe {
    use crate::sync::webdav::WebDavSync;

    let runtime = run_db_blocking(state, |db, ss| {
        let enabled = crate::db::settings::get(&db, "sync_webdav_enabled")?.as_deref() == Some("1");
        let auto_pull = auto_pull_enabled(&db, SyncProvider::Webdav)?;
        let password = if auto_pull {
            ss.get(&auto_pull_password_key(SyncProvider::Webdav))?
        } else {
            None
        };
        let sync = if enabled {
            match (
                crate::db::settings::get(&db, "webdav_url")?,
                ss.get(&setting_key("webdav_password"))?,
            ) {
                (Some(url), Some(webdav_password))
                    if !url.is_empty() && !webdav_password.is_empty() =>
                {
                    let username =
                        crate::db::settings::get(&db, "webdav_username")?.unwrap_or_default();
                    Some(WebDavSync::from_settings(
                        &url,
                        &username,
                        &webdav_password,
                    )?)
                }
                _ => None,
            }
        } else {
            None
        };
        Ok((enabled, auto_pull, password, sync))
    })
    .await;

    let (enabled, auto_pull, password, sync) = match runtime {
        Ok(runtime) => runtime,
        Err(err) => {
            let mut status = ProviderSyncStatus::new(false, false);
            status.error = Some(err.to_string());
            return ProviderProbe {
                status,
                password: None,
            };
        }
    };
    let Some(sync) = sync else {
        return ProviderProbe {
            status: ProviderSyncStatus::new(enabled, auto_pull),
            password,
        };
    };

    let mut status = ProviderSyncStatus::new(enabled, auto_pull);
    let remote = match sync.pull_metadata().await {
        Ok(remote) => remote,
        Err(err) => {
            status.error = Some(err.to_string());
            return ProviderProbe { status, password };
        }
    };
    status.remote = remote;
    ProviderProbe { status, password }
}

async fn maybe_auto_pull_webdav(
    state: &AppState,
    mut probe: ProviderProbe,
    local: SyncMetadata,
) -> (ProviderSyncStatus, SyncMetadata) {
    if !probe.remote_is_newer_than(&local) {
        return (probe.status, local);
    }
    let Some(password) = probe.password else {
        probe.status.error =
            Some(AppError::config("sync_auto_pull_password_missing", json!({})).to_string());
        return (probe.status, local);
    };
    match webdav_pull_impl(state, password).await {
        Ok(()) => match refresh_local_metadata_blocking(state).await {
            Ok(local) => {
                probe.status.pulled = true;
                (probe.status, local)
            }
            Err(err) => {
                probe.status.error = Some(err.to_string());
                (probe.status, local)
            }
        },
        Err(err) => {
            probe.status.error = Some(err.to_string());
            (probe.status, local)
        }
    }
}

/// Run a blocking DB closure off the tokio async runtime. The async GitHub
/// sync commands all wrap a multi-statement DB step (the worst is
/// `merge_import`, which upserts every config category and can hold the SQLite
/// writer for tens to hundreds of ms). Without spawn_blocking, the async
/// runtime worker that handled `github_push` / `github_pull` stalls for that
/// whole window and any other tab's async command on the same worker stalls
/// with it.
///
/// `ssh_connect` and `forward_start` also touch the DB inside an `async fn`,
/// but each call is a fast SELECT (<1 ms) and the total per-connect run is
/// ~10 ms — well below the threshold where wrapping pays for the closure
/// boilerplate. Leave them alone; this helper is targeted at sync's heavy paths.
async fn run_db_blocking<F, T>(state: &AppState, f: F) -> AppResult<T>
where
    F: FnOnce(Arc<Db>, Arc<dyn SecretStore>) -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    let db = state.db.clone();
    let ss = state.secret_store.clone();
    tauri::async_runtime::spawn_blocking(move || f(db, ss))
        .await
        .map_err(|e| AppError::other("blocking_join_failed", json!({ "err": e.to_string() })))?
}

async fn refresh_local_metadata_blocking(state: &AppState) -> AppResult<SyncMetadata> {
    let data_dir = state.data_dir.clone();
    run_db_blocking(state, move |db, _| refresh_local_metadata(&db, &data_dir)).await
}

async fn load_or_refresh_local_metadata_blocking(state: &AppState) -> AppResult<SyncMetadata> {
    let data_dir = state.data_dir.clone();
    run_db_blocking(state, move |db, _| {
        load_local_metadata(&db)?.map_or_else(|| refresh_local_metadata(&db, &data_dir), Ok)
    })
    .await
}

/// Pull may partially apply rows before `merge_import` reports its aggregate
/// error. Rebase that partial state on the old version so a higher remote stays
/// higher and the next scheduled pass retries it.
async fn finish_pull(
    state: &AppState,
    previous_version: u64,
    remote: Option<SyncMetadata>,
    result: AppResult<()>,
) -> AppResult<()> {
    let data_dir = state.data_dir.clone();
    match result {
        Ok(()) => {
            run_db_blocking(state, move |db, _| {
                if let Some(remote) = remote {
                    adopt_remote_version(&db, &data_dir, remote.version)?;
                } else {
                    refresh_local_metadata(&db, &data_dir)?;
                }
                Ok(())
            })
            .await
        }
        Err(err) => {
            if let Err(rebase_err) = run_db_blocking(state, move |db, _| {
                adopt_remote_version(&db, &data_dir, previous_version).map(|_| ())
            })
            .await
            {
                log::warn!("failed to rebase sync metadata after pull error: {rebase_err}");
            }
            Err(err)
        }
    }
}

// Local import/export (cross-platform). The payload builder lives in
// `crate::sync::config` (next to merge_import) so GUI + CLI share one shape.

/// Pretty-printed full local backup (every category + secret, no toggles).
/// Used by `export_config` and `export_config_to_file`.
fn build_export_json_blocking(
    db: &Db,
    ss: &dyn SecretStore,
    data_dir: &std::path::Path,
) -> AppResult<String> {
    let payload = build_payload(db, ss, data_dir, &ExportMode::LocalBackup)?;
    serde_json::to_string_pretty(&payload)
        .map_err(|e| AppError::other("serde_failed", json!({ "err": e.to_string() })))
}

#[tauri::command]
pub fn export_config(state: State<'_, AppState>) -> AppResult<String> {
    export_config_impl(&state)
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
/// Sync — the caller runs it on a blocking-safe context.
pub fn export_config_impl(state: &AppState) -> AppResult<String> {
    build_export_json_blocking(&state.db, state.secret_store.as_ref(), &state.data_dir)
}

/// File import: incremental merge. Local rows survive; same-id rows are
/// overwritten; parse/write failures are collected per row, not aborting
/// the whole import.
///
/// Sync command — Tauri runs this on the blocking pool, so the multi-table
/// transaction inside `merge_import` doesn't stall the async runtime.
#[tauri::command]
pub fn import_config(state: State<'_, AppState>, json: String) -> AppResult<()> {
    import_config_impl(&state, json)
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub fn import_config_impl(state: &AppState, json: String) -> AppResult<()> {
    let data: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
    crate::sync::config::merge_import(
        &state.db,
        state.secret_store.as_ref(),
        &state.data_dir,
        &data,
    )
}

// ---------------------------------------------------------------------------
// GitHub sync
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn github_push(state: State<'_, AppState>, password: String) -> AppResult<()> {
    github_push_impl(&state, password).await
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub async fn github_push_impl(state: &AppState, password: String) -> AppResult<()> {
    use crate::sync::github::GitHubSync;

    // Build the full JSON payload off the async runtime — list_*, secret-store
    // lookups, and serde all run in the blocking pool. See `run_db_blocking`
    // doc for why this matters for sync but not for ssh_connect.
    let data_dir = state.data_dir.clone();
    let (token, repo, branch, json, metadata) = run_db_blocking(state, move |db, ss| {
        let token = ss
            .get(&setting_key("github_token"))?
            .ok_or_else(|| AppError::config("github_token_missing", json!({})))?;
        let repo = crate::db::settings::get(&db, "github_repo")?
            .ok_or_else(|| AppError::config("github_repo_missing", json!({})))?;
        let branch = crate::db::settings::get(&db, "github_branch")?.unwrap_or("main".into());

        // Push path: apply per-category toggles + group filter, scrub
        // local-only secrets. Same builder as local export so the shape can't
        // drift; a disabled category is just absent from the JSON.
        let prefs = read_sync_prefs(&db)?;
        let payload = build_payload(&db, ss.as_ref(), &data_dir, &ExportMode::RemotePush(prefs))?;
        let payload = serde_json::to_string_pretty(&payload)
            .map_err(|e| AppError::other("serde_failed", json!({ "err": e.to_string() })))?;
        let metadata = refresh_local_metadata(&db, &data_dir)?;

        Ok((token, repo, branch, payload, metadata))
    })
    .await?;

    let encrypted = crate::crypto::encrypt(&json, &password)?;
    let sync = GitHubSync::from_settings(&token, &repo, &branch)?;
    sync.push(&encrypted).await?;
    sync.push_metadata(&metadata).await
}

#[tauri::command]
pub async fn github_pull(state: State<'_, AppState>, password: String) -> AppResult<()> {
    github_pull_impl(&state, password).await
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub async fn github_pull_impl(state: &AppState, password: String) -> AppResult<()> {
    use crate::sync::github::GitHubSync;

    let previous = refresh_local_metadata_blocking(state).await?;

    // Settings reads are cheap; group them with the network-prep step.
    // The decrypt + merge_import block — `merge_import` upserts every config
    // category — is the expensive part and goes to spawn_blocking below.
    let (token, repo, branch) = run_db_blocking(state, |db, ss| {
        let token = ss
            .get(&setting_key("github_token"))?
            .ok_or_else(|| AppError::config("github_token_missing", json!({})))?;
        let repo = crate::db::settings::get(&db, "github_repo")?
            .ok_or_else(|| AppError::config("github_repo_missing", json!({})))?;
        let branch = crate::db::settings::get(&db, "github_branch")?.unwrap_or("main".into());
        Ok((token, repo, branch))
    })
    .await?;

    let sync = GitHubSync::from_settings(&token, &repo, &branch)?;
    let remote = sync.pull_metadata().await?;
    let encrypted = sync.pull().await?;

    // decrypt + JSON parse + merge upsert: all blocking work.
    let data_dir = state.data_dir.clone();
    let result = run_db_blocking(state, move |db, ss| {
        let json = crate::crypto::decrypt(&encrypted, &password)?;
        let data: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
        crate::sync::config::merge_import(&db, ss.as_ref(), &data_dir, &data)
    })
    .await;
    finish_pull(state, previous.version, remote, result).await
}

// ---------------------------------------------------------------------------
// WebDAV sync
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn webdav_push(state: State<'_, AppState>, password: String) -> AppResult<()> {
    webdav_push_impl(&state, password).await
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub async fn webdav_push_impl(state: &AppState, password: String) -> AppResult<()> {
    use crate::sync::webdav::WebDavSync;

    let data_dir = state.data_dir.clone();
    let (url, username, wd_password, json, metadata) = run_db_blocking(state, move |db, ss| {
        let url = crate::db::settings::get(&db, "webdav_url")?
            .ok_or_else(|| AppError::config("webdav_url_missing", json!({})))?;
        let username = crate::db::settings::get(&db, "webdav_username")?.unwrap_or_default();
        let wd_password = ss
            .get(&setting_key("webdav_password"))?
            .ok_or_else(|| AppError::config("webdav_password_missing", json!({})))?;

        // Same filter + scrub rules as GitHub push; the only transport-specific
        // part is where the encrypted blob lands.
        let prefs = read_sync_prefs(&db)?;
        let payload = build_payload(&db, ss.as_ref(), &data_dir, &ExportMode::RemotePush(prefs))?;
        let payload = serde_json::to_string_pretty(&payload)
            .map_err(|e| AppError::other("serde_failed", json!({ "err": e.to_string() })))?;
        let metadata = refresh_local_metadata(&db, &data_dir)?;

        Ok((url, username, wd_password, payload, metadata))
    })
    .await?;

    let encrypted = crate::crypto::encrypt(&json, &password)?;
    let sync = WebDavSync::from_settings(&url, &username, &wd_password)?;
    sync.push(&encrypted).await?;
    sync.push_metadata(&metadata).await
}

#[tauri::command]
pub async fn webdav_pull(state: State<'_, AppState>, password: String) -> AppResult<()> {
    webdav_pull_impl(&state, password).await
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
pub async fn webdav_pull_impl(state: &AppState, password: String) -> AppResult<()> {
    use crate::sync::webdav::WebDavSync;

    let previous = refresh_local_metadata_blocking(state).await?;

    let (url, username, wd_password) = run_db_blocking(state, |db, ss| {
        let url = crate::db::settings::get(&db, "webdav_url")?
            .ok_or_else(|| AppError::config("webdav_url_missing", json!({})))?;
        let username = crate::db::settings::get(&db, "webdav_username")?.unwrap_or_default();
        let wd_password = ss
            .get(&setting_key("webdav_password"))?
            .ok_or_else(|| AppError::config("webdav_password_missing", json!({})))?;
        Ok((url, username, wd_password))
    })
    .await?;

    let sync = WebDavSync::from_settings(&url, &username, &wd_password)?;
    let remote = sync.pull_metadata().await?;
    let encrypted = sync.pull().await?;

    let data_dir = state.data_dir.clone();
    let result = run_db_blocking(state, move |db, ss| {
        let json = crate::crypto::decrypt(&encrypted, &password)?;
        let data: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| AppError::config("json_parse_failed", json!({ "err": e.to_string() })))?;
        crate::sync::config::merge_import(&db, ss.as_ref(), &data_dir, &data)
    })
    .await;
    finish_pull(state, previous.version, remote, result).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::profile;
    use crate::models::{Forward, ForwardType, Profile, SerialProfile};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-process SecretStore for tests.
    #[derive(Default)]
    struct MemStore {
        inner: Mutex<HashMap<String, String>>,
    }
    impl SecretStore for MemStore {
        fn get(&self, key: &str) -> AppResult<Option<String>> {
            Ok(self.inner.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> AppResult<()> {
            self.inner
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, key: &str) -> AppResult<()> {
            self.inner.lock().unwrap().remove(key);
            Ok(())
        }
        fn backend_name(&self) -> &'static str {
            "mem"
        }
    }

    #[derive(Default)]
    struct DeleteFailStore {
        inner: MemStore,
    }

    impl SecretStore for DeleteFailStore {
        fn get(&self, key: &str) -> AppResult<Option<String>> {
            self.inner.get(key)
        }

        fn set(&self, key: &str, value: &str) -> AppResult<()> {
            self.inner.set(key, value)
        }

        fn delete(&self, _key: &str) -> AppResult<()> {
            Err(AppError::other(
                "test_secret_delete_failed",
                serde_json::json!({}),
            ))
        }

        fn backend_name(&self) -> &'static str {
            "delete-fail"
        }
    }

    fn fixture() -> (Db, MemStore, tempfile::TempDir) {
        (
            Db::open_in_memory().unwrap(),
            MemStore::default(),
            tempfile::tempdir().unwrap(),
        )
    }

    fn prof(id: &str, group: Option<&str>) -> Profile {
        Profile {
            id: id.into(),
            name: format!("name-{id}"),
            host: "h".into(),
            port: 22,
            credential_id: "c".into(),
            bastion_profile_id: None,
            init_command: None,
            group_id: group.map(String::from),
            algorithms: Default::default(),
        }
    }

    fn fwd(id: &str, group: Option<&str>) -> Forward {
        Forward {
            id: id.into(),
            name: format!("fwd-{id}"),
            forward_type: ForwardType::Local,
            local_port: 8080,
            remote_host: "127.0.0.1".into(),
            remote_port: 80,
            profile_id: "p".into(),
            group_id: group.map(String::from),
        }
    }

    fn ser(id: &str, group: Option<&str>) -> SerialProfile {
        SerialProfile {
            id: id.into(),
            name: format!("ser-{id}"),
            port: "/dev/ttyUSB0".into(),
            baud_rate: 115200,
            data_bits: 8,
            parity: "none".into(),
            stop_bits: 1,
            flow_control: "none".into(),
            xany: false,
            input_newline: "cr".into(),
            output_newline: "raw".into(),
            local_echo: false,
            backspace: "del".into(),
            slow_send: false,
            input_mode: "normal".into(),
            output_mode: "text".into(),
            login_script: String::new(),
            group_id: group.map(String::from),
        }
    }

    #[test]
    fn local_backup_includes_all_categories() {
        let (db, ss, dir) = fixture();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::LocalBackup).unwrap();
        let obj = v.as_object().unwrap();
        for k in [
            "profiles",
            "credentials",
            "forwards",
            "groups",
            "serial_profiles",
            "telnet_profiles",
            "skills",
            "highlights",
            "snippets",
            "ai_redact_rules",
            "ai_command_blacklist",
            "ai",
        ] {
            assert!(obj.contains_key(k), "local backup missing '{k}'");
        }
    }

    #[test]
    fn push_excludes_disabled_category() {
        let (db, ss, dir) = fixture();
        crate::db::settings::set(&db, "sync_include_highlights", "0").unwrap();
        crate::db::settings::set(&db, "sync_include_snippets", "0").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("highlights"), "disabled key omitted");
        assert!(!obj.contains_key("snippets"), "disabled key omitted");
        assert!(obj.contains_key("credentials"), "enabled key present");
        assert!(obj.contains_key("profiles"));
    }

    #[test]
    fn push_always_includes_credentials_and_groups_secret_still_gated() {
        use crate::db::credential;
        use crate::models::{Credential, CredentialType};
        use crate::secret::cred_secret_key;
        let (db, ss, dir) = fixture();
        // credentials + groups are referential deps of the always-exported
        // profiles/forwards/serial; their category toggles were removed, so a
        // stale "0" must NOT drop them. Secret upload stays gated per-credential
        // by save_to_remote — independent of (and unaffected by) that removal.
        crate::db::settings::set(&db, "sync_include_credentials", "0").unwrap();
        crate::db::settings::set(&db, "sync_include_groups", "0").unwrap();
        let meta = |id: &str, remote: bool| Credential {
            id: id.into(),
            name: id.into(),
            username: "u".into(),
            credential_type: CredentialType::Password,
            secret: None,
            save_to_remote: remote,
        };
        credential::insert(&db, &meta("r", true)).unwrap();
        credential::insert(&db, &meta("l", false)).unwrap();
        ss.set(&cred_secret_key("r"), "s-r").unwrap();
        ss.set(&cred_secret_key("l"), "s-l").unwrap();

        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        let obj = v.as_object().unwrap();
        assert!(obj.contains_key("groups"), "groups always pushed");
        let creds = obj["credentials"].as_array().unwrap();
        assert_eq!(creds.len(), 2, "all credential metadata always pushed");
        let secret = |id: &str| creds.iter().find(|c| c["id"] == id).unwrap()["secret"].clone();
        assert_eq!(
            secret("r"),
            json!("s-r"),
            "save_to_remote=true keeps secret"
        );
        assert_eq!(
            secret("l"),
            json!(null),
            "save_to_remote=false scrubs secret"
        );
    }

    #[test]
    fn push_filters_profiles_by_group() {
        let (db, ss, dir) = fixture();
        profile::insert(&db, &prof("p1", Some("g1"))).unwrap();
        profile::insert(&db, &prof("p2", Some("g2"))).unwrap();
        profile::insert(&db, &prof("p3", None)).unwrap();
        crate::db::settings::set(&db, "sync_profile_group_ids", "[\"g1\"]").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        let ids: Vec<&str> = v["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["p1"], "only the selected group is exported");
    }

    #[test]
    fn push_filters_forwards_and_serial_by_group() {
        // Forwards and serial profiles now share the profile group filter: pick a
        // group, only that group's rows of every kind go out. Ungrouped drop out.
        let (db, ss, dir) = fixture();
        crate::db::forward::insert(&db, &fwd("f1", Some("g1"))).unwrap();
        crate::db::forward::insert(&db, &fwd("f2", Some("g2"))).unwrap();
        crate::db::forward::insert(&db, &fwd("f3", None)).unwrap();
        crate::db::serial_profile::insert(&db, &ser("s1", Some("g1"))).unwrap();
        crate::db::serial_profile::insert(&db, &ser("s2", Some("g2"))).unwrap();
        crate::db::settings::set(&db, "sync_profile_group_ids", "[\"g1\"]").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();

        let fids: Vec<&str> = v["forwards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["id"].as_str().unwrap())
            .collect();
        assert_eq!(fids, vec!["f1"], "only g1 forwards exported");
        let sids: Vec<&str> = v["serial_profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert_eq!(sids, vec!["s1"], "only g1 serial profiles exported");
    }

    #[test]
    fn push_all_groups_includes_ungrouped_forwards_serial() {
        // Empty-string sentinel = all groups = everything, including ungrouped
        // forwards/serial (the "select all groups to sync everything" contract).
        let (db, ss, dir) = fixture();
        crate::db::forward::insert(&db, &fwd("f1", Some("g1"))).unwrap();
        crate::db::forward::insert(&db, &fwd("f2", None)).unwrap();
        crate::db::serial_profile::insert(&db, &ser("s1", None)).unwrap();
        crate::db::settings::set(&db, "sync_profile_group_ids", "").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        assert_eq!(
            v["forwards"].as_array().unwrap().len(),
            2,
            "all forwards incl ungrouped"
        );
        assert_eq!(
            v["serial_profiles"].as_array().unwrap().len(),
            1,
            "ungrouped serial included"
        );
    }

    #[test]
    fn push_ungrouped_sentinel_selects_ungrouped_rows() {
        // "" = the "Ungrouped" chip: selecting only it syncs only the rows that
        // have no group, across all three kinds.
        let (db, ss, dir) = fixture();
        profile::insert(&db, &prof("p1", Some("g1"))).unwrap();
        profile::insert(&db, &prof("p2", None)).unwrap();
        crate::db::forward::insert(&db, &fwd("f1", None)).unwrap();
        crate::db::forward::insert(&db, &fwd("f2", Some("g1"))).unwrap();
        crate::db::serial_profile::insert(&db, &ser("s1", None)).unwrap();
        crate::db::settings::set(&db, "sync_profile_group_ids", "[\"\"]").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        let pid: Vec<&str> = v["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert_eq!(pid, vec!["p2"], "only ungrouped profile");
        let fid: Vec<&str> = v["forwards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["id"].as_str().unwrap())
            .collect();
        assert_eq!(fid, vec!["f1"], "only ungrouped forward");
        assert_eq!(
            v["serial_profiles"].as_array().unwrap().len(),
            1,
            "ungrouped serial"
        );
    }

    #[test]
    fn push_group_plus_ungrouped_sentinel() {
        // A real group id + "" syncs that group AND the ungrouped rows.
        let (db, ss, dir) = fixture();
        profile::insert(&db, &prof("p1", Some("g1"))).unwrap();
        profile::insert(&db, &prof("p2", Some("g2"))).unwrap();
        profile::insert(&db, &prof("p3", None)).unwrap();
        crate::db::settings::set(&db, "sync_profile_group_ids", "[\"g1\",\"\"]").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        let pid: Vec<&str> = v["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert_eq!(pid, vec!["p1", "p3"], "g1 + ungrouped");
    }

    #[test]
    fn push_empty_group_list_syncs_no_profiles() {
        // Explicit empty array = user deselected every group → sync nothing.
        let (db, ss, dir) = fixture();
        profile::insert(&db, &prof("p1", Some("g1"))).unwrap();
        crate::db::settings::set(&db, "sync_profile_group_ids", "[]").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        assert_eq!(v["profiles"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn malformed_group_filter_errors_not_fail_open() {
        // A corrupted setting must error, never silently fall back to None
        // (= "sync all"), which would widen a narrowed export to every profile.
        let (db, _ss, _dir) = fixture();
        crate::db::settings::set(&db, "sync_profile_group_ids", "{not json").unwrap();
        let err = read_sync_prefs(&db).unwrap_err();
        assert_eq!(err.code(), "sync_profile_group_ids_invalid");
    }

    #[test]
    fn push_empty_string_group_filter_syncs_all_profiles() {
        // Empty string = "all groups selected" sentinel → no filter (incl. ungrouped).
        let (db, ss, dir) = fixture();
        profile::insert(&db, &prof("p1", Some("g1"))).unwrap();
        profile::insert(&db, &prof("p2", None)).unwrap();
        crate::db::settings::set(&db, "sync_profile_group_ids", "").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        assert_eq!(
            v["profiles"].as_array().unwrap().len(),
            2,
            "empty string = sync all, including ungrouped"
        );
    }

    #[test]
    fn push_ai_toggle_gates_whole_block_including_key() {
        // "AI 配置" is one switch (merged from the old sync_include_ai +
        // sync_include_ai_key pair): on → provider config AND key are exported;
        // off → the whole "ai" section is omitted.
        let (db, ss, dir) = fixture();
        crate::db::settings::set(&db, "ai_anthropic_model", "claude-x").unwrap();
        ss.set(&setting_key("ai_anthropic_key"), "sk-secret")
            .unwrap();

        // On (default): the key rides alongside model/endpoint.
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        let anth = v["ai"]["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["provider"] == "anthropic")
            .expect("anthropic present (model configured)");
        assert_eq!(anth["api_key"], "sk-secret", "key rides with the AI block");
        assert_eq!(anth["model"], "claude-x");

        // Off: the entire ai section is gone.
        crate::db::settings::set(&db, "sync_include_ai", "0").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        assert!(v.get("ai").is_none(), "ai section omitted when toggle off");
    }

    #[test]
    fn payload_never_contains_sync_toggles() {
        let (db, ss, dir) = fixture();
        crate::db::settings::set(&db, "sync_include_highlights", "0").unwrap();
        let prefs = read_sync_prefs(&db).unwrap();
        let v = build_payload(&db, &ss, dir.path(), &ExportMode::RemotePush(prefs)).unwrap();
        let s = serde_json::to_string(&v).unwrap();
        assert!(
            !s.contains("sync_include"),
            "toggle keys never leave the device"
        );
        assert!(!s.contains("sync_profile_group_ids"));
    }

    #[test]
    fn auto_pull_defaults_off_and_keeps_passwords_per_provider() {
        let (db, ss, _dir) = fixture();
        assert!(!auto_pull_enabled(&db, SyncProvider::Github).unwrap());
        assert!(!auto_pull_enabled(&db, SyncProvider::Webdav).unwrap());

        set_auto_pull(&db, &ss, SyncProvider::Github, true, Some("github-secret")).unwrap();
        assert!(auto_pull_enabled(&db, SyncProvider::Github).unwrap());
        assert!(!auto_pull_enabled(&db, SyncProvider::Webdav).unwrap());
        assert_eq!(
            ss.get(&auto_pull_password_key(SyncProvider::Github))
                .unwrap()
                .as_deref(),
            Some("github-secret")
        );

        set_auto_pull(&db, &ss, SyncProvider::Github, false, None).unwrap();
        assert!(!auto_pull_enabled(&db, SyncProvider::Github).unwrap());
        assert!(ss
            .get(&auto_pull_password_key(SyncProvider::Github))
            .unwrap()
            .is_none());
    }

    #[test]
    fn auto_pull_disable_failure_keeps_enabled_commit_marker() {
        let db = Db::open_in_memory().unwrap();
        let ss = DeleteFailStore::default();
        let key = auto_pull_password_key(SyncProvider::Github);
        set_auto_pull(&db, &ss, SyncProvider::Github, true, Some("secret")).unwrap();

        let err = set_auto_pull(&db, &ss, SyncProvider::Github, false, None).unwrap_err();

        assert_eq!(err.code(), "test_secret_delete_failed");
        assert!(
            auto_pull_enabled(&db, SyncProvider::Github).unwrap(),
            "a failed disable must not persist OFF while the frontend keeps showing ON"
        );
        assert_eq!(ss.get(&key).unwrap().as_deref(), Some("secret"));
    }

    #[test]
    fn auto_pull_probe_rechecks_against_the_latest_local_version() {
        let probe = ProviderProbe {
            status: ProviderSyncStatus {
                enabled: true,
                auto_pull: true,
                remote: Some(SyncMetadata {
                    version: 5,
                    config_digest: format!("sha256:{}", "5".repeat(64)),
                }),
                error: None,
                pulled: false,
            },
            password: Some("secret".into()),
        };
        let local = |version| SyncMetadata {
            version,
            config_digest: format!("sha256:{}", "1".repeat(64)),
        };

        assert!(probe.remote_is_newer_than(&local(1)));
        assert!(!probe.remote_is_newer_than(&local(6)));
    }

    #[tokio::test]
    async fn provider_metadata_probes_start_together() {
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::sync::Barrier;

        let barrier = Arc::new(Barrier::new(3));
        let github_barrier = barrier.clone();
        let webdav_barrier = barrier.clone();
        let github = async move {
            github_barrier.wait().await;
            "github"
        };
        let webdav = async move {
            webdav_barrier.wait().await;
            "webdav"
        };

        let joined = join_provider_probes(github, webdav);
        let all_started = barrier.wait();
        let (joined, started) = tokio::join!(
            tokio::time::timeout(Duration::from_millis(250), joined),
            tokio::time::timeout(Duration::from_millis(250), all_started),
        );

        started.expect("both provider probes must start before either completes");
        assert_eq!(joined.unwrap(), ("github", "webdav"));
    }
}
