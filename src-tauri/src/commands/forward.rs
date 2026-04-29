use tauri::State;

use crate::error::{locked, AppError, AppResult};
use crate::models::{Credential, Forward, ForwardType, Profile};
use crate::ssh::forward as fwd;
use crate::state::AppState;

#[tauri::command]
pub fn list_forwards(state: State<AppState>) -> Result<Vec<Forward>, AppError> {
    crate::db::forward::list(&state.db)
}

#[tauri::command]
pub fn get_forward(state: State<AppState>, id: String) -> Result<Forward, AppError> {
    crate::db::forward::get(&state.db, &id)
}

#[tauri::command]
pub fn create_forward(state: State<AppState>, forward: Forward) -> Result<(), AppError> {
    crate::db::forward::insert(&state.db, &forward)
}

#[tauri::command]
pub fn update_forward(state: State<AppState>, forward: Forward) -> Result<(), AppError> {
    crate::db::forward::update(&state.db, &forward)
}

#[tauri::command]
pub fn delete_forward(state: State<AppState>, id: String) -> Result<(), AppError> {
    crate::db::forward::delete(&state.db, &id)
}

// ---------------------------------------------------------------------------
// 活跃端口转发 — 启动 / 停止
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn forward_start(state: State<'_, AppState>, forward_id: String) -> AppResult<String> {
    let f = crate::db::forward::get(&state.db, &forward_id)?;
    let p = crate::db::profile::get(&state.db, &f.profile_id)
        .map_err(|_| AppError::NotFound("转发关联的 Profile 不存在".into()))?;
    let cred_id = p.credential_id.as_deref().unwrap_or("");
    let mut c = crate::db::credential::get(&state.db, cred_id)
        .map_err(|_| AppError::NotFound("转发关联的凭证不存在".into()))?;
    if !c.id.is_empty() {
        c.secret = state
            .secret_store
            .get(&crate::secret::cred_secret_key(&c.id))?;
    }
    let timeout_secs: u64 = crate::db::settings::get(&state.db, "connect_timeout")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(crate::ssh::client::DEFAULT_CONNECT_TIMEOUT);

    // 解析 forward target profile 的堡垒机链，每一跳加载 secret
    let chain_profiles = crate::ssh::bastion::resolve_chain(&state.db, &p)?;
    let mut chain: Vec<(Profile, Credential)> = Vec::with_capacity(chain_profiles.len());
    for hop in chain_profiles {
        let bcid = hop.credential_id.as_deref().unwrap_or("");
        let mut bc = crate::db::credential::get(&state.db, bcid)
            .map_err(|_| AppError::NotFound(format!("堡垒机 '{}' 凭证不存在", hop.name)))?;
        if !bc.id.is_empty() {
            bc.secret = state
                .secret_store
                .get(&crate::secret::cred_secret_key(&bc.id))?;
        }
        chain.push((hop, bc));
    }
    let known_hosts_path = crate::ssh::known_hosts::path_for(&state.data_dir);
    let host = p.host.clone();
    let port = p.port;
    let kind = f.forward_type;
    let handle = crate::ssh::client::run_blocking_ssh(move || async move {
        match kind {
            ForwardType::Local => {
                fwd::start_local(f, host, port, c, chain, known_hosts_path, timeout_secs).await
            }
            ForwardType::Remote => {
                fwd::start_remote(f, host, port, c, chain, known_hosts_path, timeout_secs).await
            }
            ForwardType::Dynamic => {
                fwd::start_dynamic(f, host, port, c, chain, known_hosts_path, timeout_secs).await
            }
        }
    })
    .await?;
    let active_id = uuid::Uuid::new_v4().to_string();

    locked(&state.active_forwards)?.insert(active_id.clone(), handle);

    Ok(active_id)
}

#[tauri::command]
pub fn forward_stats(
    state: State<'_, AppState>,
    active_id: String,
) -> AppResult<fwd::ForwardStats> {
    let forwards = locked(&state.active_forwards)?;
    let handle = forwards
        .get(&active_id)
        .ok_or(AppError::NotFound("转发不存在".into()))?;
    Ok(handle.stats())
}

#[tauri::command]
pub fn forward_stop(state: State<'_, AppState>, active_id: String) -> AppResult<()> {
    let handle = locked(&state.active_forwards)?
        .remove(&active_id)
        .ok_or(AppError::NotFound("转发不存在".into()))?;
    handle.stop();
    Ok(())
}
