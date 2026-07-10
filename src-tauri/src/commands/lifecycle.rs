//! Session 生命周期管理：前端 reconcile + 窗口销毁清理。

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use tauri::State;

use crate::error::{locked, AppError, AppResult};
use crate::state::AppState;
use crate::state::{AiSessionRecord, SessionKind, SessionOwner, SessionPhase, SessionRecord};

/// 前端启动 / 重连后调用：把不在 `active_ids` 列表里的所有 session 全部清掉。
///
/// `active_ids` 是前端当前持有的所有 ID（不区分 ssh / sftp / forward —
/// UUID 不会撞）。返回被清理的总数。
#[tauri::command]
pub fn reconcile_sessions(
    window: tauri::Window,
    state: State<'_, AppState>,
    active_ids: Vec<String>,
) -> AppResult<usize> {
    reconcile_sessions_impl(
        &state,
        &SessionOwner::Window(window.label().to_owned()),
        active_ids,
    )
}

/// Resolve a frontend-reserved session identity. Pre-reservation lets the UI
/// subscribe before a fast transport emits its first byte; using that same id
/// everywhere keeps existing consumers (AI, lifecycle, close/write) coherent.
pub fn resolve_session_id(requested: Option<String>) -> AppResult<String> {
    match requested {
        Some(id) => match uuid::Uuid::parse_str(&id) {
            Ok(parsed) if parsed.to_string() == id => Ok(id),
            _ => Err(AppError::config(
                "session_id_invalid",
                serde_json::json!({}),
            )),
        },
        None => Ok(uuid::Uuid::new_v4().to_string()),
    }
}

pub struct ResourceReservation<'a> {
    state: &'a AppState,
    session_id: String,
    nonce: uuid::Uuid,
    kind: SessionKind,
    armed: bool,
}

pub struct AiOwnerReservation<'a> {
    state: &'a AppState,
    tab_id: String,
    owner: SessionOwner,
    nonce: uuid::Uuid,
    armed: bool,
}

impl Drop for AiOwnerReservation<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(mut owners) = self.state.ai_session_owners.lock() {
            if owners.get(&self.tab_id).is_some_and(|record| {
                record.owner == self.owner
                    && record.nonce == self.nonce
                    && record.phase == SessionPhase::Pending
            }) {
                owners.remove(&self.tab_id);
            }
        }
    }
}

impl AiOwnerReservation<'_> {
    pub fn activate(mut self, session: crate::ai::session::DiagnoseSession) -> AppResult<()> {
        let mut owners = locked(&self.state.ai_session_owners)?;
        if !owners.get(&self.tab_id).is_some_and(|record| {
            record.owner == self.owner
                && record.nonce == self.nonce
                && record.phase == SessionPhase::Pending
        }) {
            let _ = session.action_tx.send(crate::ai::session::UserAction::Stop);
            return Err(AppError::not_found(
                "session_reservation_lost",
                serde_json::json!({ "id": self.tab_id }),
            ));
        }
        let mut sessions = locked(&self.state.ai_sessions)?;
        if sessions.contains_key(&self.tab_id) {
            let _ = session.action_tx.send(crate::ai::session::UserAction::Stop);
            return Err(AppError::other(
                "session_already_exists",
                serde_json::json!({ "tab_id": self.tab_id }),
            ));
        }
        if sessions
            .values()
            .any(|existing| existing.conversation_id == session.conversation_id)
        {
            let _ = session.action_tx.send(crate::ai::session::UserAction::Stop);
            return Err(AppError::other(
                "conversation_in_use",
                serde_json::json!({}),
            ));
        }
        sessions.insert(self.tab_id.clone(), session);
        drop(sessions);
        let record = owners
            .get_mut(&self.tab_id)
            .expect("AI owner was validated");
        record.phase = SessionPhase::Ready;
        drop(owners);
        self.armed = false;
        Ok(())
    }
}

pub fn reserve_ai_owner(
    state: &AppState,
    tab_id: String,
    owner: SessionOwner,
) -> AppResult<AiOwnerReservation<'_>> {
    let mut owners = locked(&state.ai_session_owners)?;
    if owners.contains_key(&tab_id) || locked(&state.ai_sessions)?.contains_key(&tab_id) {
        return Err(AppError::other(
            "session_already_exists",
            serde_json::json!({ "tab_id": tab_id }),
        ));
    }
    let nonce = uuid::Uuid::new_v4();
    owners.insert(
        tab_id.clone(),
        AiSessionRecord {
            nonce,
            owner: owner.clone(),
            phase: SessionPhase::Pending,
        },
    );
    drop(owners);
    Ok(AiOwnerReservation {
        state,
        tab_id,
        owner,
        nonce,
        armed: true,
    })
}

pub fn close_ai_session(
    state: &AppState,
    tab_id: &str,
    expected_owner: &SessionOwner,
) -> AppResult<()> {
    let mut owners = locked(&state.ai_session_owners)?;
    let record = owners
        .get(tab_id)
        .ok_or_else(|| AppError::not_found("ai_session_not_found", serde_json::json!({})))?;
    if &record.owner != expected_owner {
        return Err(AppError::not_found(
            "ai_session_not_found",
            serde_json::json!({}),
        ));
    }
    let session = if record.phase == SessionPhase::Ready {
        Some(locked(&state.ai_sessions)?.remove(tab_id).ok_or_else(|| {
            AppError::other(
                "session_registry_inconsistent",
                serde_json::json!({ "id": tab_id }),
            )
        })?)
    } else {
        None
    };
    owners.remove(tab_id);
    drop(owners);
    if let Some(session) = session {
        let _ = session.action_tx.send(crate::ai::session::UserAction::Stop);
    }
    Ok(())
}

fn close_owned_ai(
    state: &AppState,
    owner: &SessionOwner,
    active: Option<&HashSet<String>>,
) -> AppResult<usize> {
    let mut owners = locked(&state.ai_session_owners)?;
    let ids: Vec<String> = owners
        .iter()
        .filter(|(id, record)| {
            &record.owner == owner && active.is_none_or(|active| !active.contains(*id))
        })
        .map(|(id, _)| id.clone())
        .collect();
    let mut sessions = locked(&state.ai_sessions)?;
    for id in &ids {
        let is_ready = owners
            .get(id)
            .is_some_and(|record| record.phase == SessionPhase::Ready);
        if sessions.contains_key(id) != is_ready {
            return Err(AppError::other(
                "session_registry_inconsistent",
                serde_json::json!({ "id": id }),
            ));
        }
    }
    let mut removed = Vec::new();
    for id in &ids {
        if owners
            .get(id)
            .is_some_and(|record| record.phase == SessionPhase::Ready)
        {
            removed.push(sessions.remove(id).expect("ready AI session was validated"));
        }
        owners.remove(id);
    }
    drop(sessions);
    drop(owners);
    for session in removed {
        let _ = session.action_tx.send(crate::ai::session::UserAction::Stop);
    }
    Ok(ids.len())
}

fn remove_owned_waiters<T>(
    waiters: &Mutex<HashMap<String, crate::state::OwnedWaiter<T>>>,
    owner: &SessionOwner,
    active: Option<&HashSet<String>>,
) -> AppResult<usize> {
    let mut waiters = locked(waiters)?;
    let before = waiters.len();
    waiters.retain(|id, waiter| {
        &waiter.owner != owner || active.is_some_and(|active| active.contains(id))
    });
    Ok(before - waiters.len())
}

fn close_owned_waiters(
    state: &AppState,
    owner: &SessionOwner,
    active: Option<&HashSet<String>>,
) -> AppResult<usize> {
    Ok(remove_owned_waiters(&state.auth_waiters, owner, active)?
        + remove_owned_waiters(&state.passphrase_waiters, owner, active)?
        + remove_owned_waiters(&state.host_key_waiters, owner, active)?)
}

fn remove_owned_waiter_id<T>(
    waiters: &Mutex<HashMap<String, crate::state::OwnedWaiter<T>>>,
    session_id: &str,
    owner: &SessionOwner,
) -> AppResult<()> {
    let mut waiters = locked(waiters)?;
    if waiters
        .get(session_id)
        .is_some_and(|waiter| &waiter.owner == owner)
    {
        waiters.remove(session_id);
    }
    Ok(())
}

fn close_waiters_for_resource(
    state: &AppState,
    session_id: &str,
    owner: &SessionOwner,
) -> AppResult<()> {
    remove_owned_waiter_id(&state.auth_waiters, session_id, owner)?;
    remove_owned_waiter_id(&state.passphrase_waiters, session_id, owner)?;
    remove_owned_waiter_id(&state.host_key_waiters, session_id, owner)
}

/// Atomically verify that an SSH connection attempt is still Pending and
/// publish one of its prompt waiters. The registry lock is deliberately held
/// until after insertion: close paths take the same lock first and then remove
/// waiters, so a prompt can neither appear after cancellation nor be missed by
/// concurrent cleanup.
pub(crate) fn register_prompt_waiter<T>(
    state: &AppState,
    waiters: &Mutex<HashMap<String, crate::state::OwnedWaiter<T>>>,
    resource_id: &str,
    prompt_id: &str,
    owner: &SessionOwner,
    event_prefix: &str,
    nonce: uuid::Uuid,
    sender: tokio::sync::oneshot::Sender<T>,
) -> AppResult<()> {
    let registry = locked(&state.lifecycle_sessions)?;
    let pending = registry.get(resource_id).is_some_and(|record| {
        record.kind == SessionKind::Ssh
            && &record.owner == owner
            && record.phase == SessionPhase::Pending
    });
    if !pending {
        return Err(AppError::not_found(
            "session_reservation_lost",
            serde_json::json!({ "id": resource_id }),
        ));
    }

    let mut waiters = locked(waiters)?;
    if waiters.contains_key(prompt_id) {
        return Err(AppError::other(
            "ssh_prompt_already_pending",
            serde_json::json!({ "prompt_id": prompt_id, "channel": event_prefix }),
        ));
    }
    waiters.insert(
        prompt_id.to_owned(),
        crate::state::OwnedWaiter {
            nonce,
            owner: owner.clone(),
            sender,
        },
    );
    drop(waiters);
    drop(registry);
    Ok(())
}

pub enum ReadySession {
    Ssh(crate::ssh::client::SessionHandle),
    #[cfg(not(target_os = "android"))]
    Pty(crate::terminal::pty::PtyHandle),
    #[cfg(not(target_os = "android"))]
    Serial(crate::terminal::serial::SerialHandle),
    Telnet(crate::terminal::telnet::TelnetHandle),
    Sftp(std::sync::Arc<crate::ssh::sftp::SftpHandle>),
    Forward(crate::ssh::forward::ForwardHandle),
    #[cfg(test)]
    CleanupProbe {
        kind: SessionKind,
        cleaned: std::sync::Arc<std::sync::atomic::AtomicBool>,
    },
}

impl ReadySession {
    fn kind(&self) -> SessionKind {
        match self {
            Self::Ssh(_) => SessionKind::Ssh,
            #[cfg(not(target_os = "android"))]
            Self::Pty(_) => SessionKind::Pty,
            #[cfg(not(target_os = "android"))]
            Self::Serial(_) => SessionKind::Serial,
            Self::Telnet(_) => SessionKind::Telnet,
            Self::Sftp(_) => SessionKind::Sftp,
            Self::Forward(_) => SessionKind::Forward,
            #[cfg(test)]
            Self::CleanupProbe { kind, .. } => *kind,
        }
    }

    fn close(self) {
        match self {
            Self::Ssh(handle) => handle.force_disconnect(),
            Self::Forward(handle) => handle.stop(),
            #[cfg(test)]
            Self::CleanupProbe { cleaned, .. } => {
                cleaned.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            #[cfg(not(target_os = "android"))]
            Self::Pty(_) | Self::Serial(_) => {}
            Self::Telnet(_) | Self::Sftp(_) => {}
        }
    }
}

impl ResourceReservation<'_> {
    pub fn id(&self) -> &str {
        &self.session_id
    }

    #[cfg(test)]
    pub fn ensure_pending(&self) -> AppResult<()> {
        let registry = locked(&self.state.lifecycle_sessions)?;
        match registry.get(&self.session_id) {
            Some(record)
                if record.nonce == self.nonce
                    && record.kind == self.kind
                    && record.phase == SessionPhase::Pending =>
            {
                Ok(())
            }
            _ => Err(AppError::not_found(
                "session_reservation_lost",
                serde_json::json!({ "id": self.session_id }),
            )),
        }
    }

    pub fn activate(mut self, handle: ReadySession) -> AppResult<()> {
        if handle.kind() != self.kind {
            handle.close();
            return Err(AppError::config(
                "session_kind_mismatch",
                serde_json::json!({ "id": self.session_id }),
            ));
        }
        let mut registry = match locked(&self.state.lifecycle_sessions) {
            Ok(registry) => registry,
            Err(error) => {
                handle.close();
                return Err(error);
            }
        };
        let Some(record) = registry.get_mut(&self.session_id) else {
            drop(registry);
            handle.close();
            return Err(AppError::not_found(
                "session_reservation_lost",
                serde_json::json!({ "id": self.session_id }),
            ));
        };
        if record.nonce != self.nonce
            || record.kind != self.kind
            || record.phase != SessionPhase::Pending
        {
            drop(registry);
            handle.close();
            return Err(AppError::not_found(
                "session_reservation_lost",
                serde_json::json!({ "id": self.session_id }),
            ));
        }
        insert_ready_handle(self.state, &self.session_id, handle)?;
        record.phase = SessionPhase::Ready;
        self.armed = false;
        Ok(())
    }

    pub fn activate_returned(self, returned_id: &str, handle: ReadySession) -> AppResult<()> {
        if returned_id != self.session_id {
            handle.close();
            return Err(AppError::other(
                "session_id_mismatch",
                serde_json::json!({
                    "reserved": &self.session_id,
                    "returned": returned_id,
                }),
            ));
        }
        self.activate(handle)
    }
}

fn insert_unique<T>(
    sessions: &Mutex<HashMap<String, T>>,
    session_id: &str,
    handle: T,
) -> AppResult<()> {
    let mut sessions = locked(sessions)?;
    if sessions.contains_key(session_id) {
        return Err(AppError::config(
            "session_id_conflict",
            serde_json::json!({ "id": session_id }),
        ));
    }
    sessions.insert(session_id.to_owned(), handle);
    Ok(())
}

fn insert_ready_handle(state: &AppState, session_id: &str, handle: ReadySession) -> AppResult<()> {
    match handle {
        ReadySession::Ssh(handle) => {
            let mut sessions = match locked(&state.sessions) {
                Ok(sessions) => sessions,
                Err(error) => {
                    handle.force_disconnect();
                    return Err(error);
                }
            };
            if sessions.contains_key(session_id) {
                handle.force_disconnect();
                return Err(AppError::config(
                    "session_id_conflict",
                    serde_json::json!({ "id": session_id }),
                ));
            }
            sessions.insert(session_id.to_owned(), handle);
            Ok(())
        }
        #[cfg(not(target_os = "android"))]
        ReadySession::Pty(handle) => insert_unique(&state.pty_sessions, session_id, handle),
        #[cfg(not(target_os = "android"))]
        ReadySession::Serial(handle) => insert_unique(&state.serial_sessions, session_id, handle),
        ReadySession::Telnet(handle) => insert_unique(&state.telnet_sessions, session_id, handle),
        ReadySession::Sftp(handle) => insert_unique(&state.sftp_sessions, session_id, handle),
        ReadySession::Forward(handle) => insert_unique(&state.active_forwards, session_id, handle),
        #[cfg(test)]
        ReadySession::CleanupProbe { cleaned, .. } => {
            cleaned.store(true, std::sync::atomic::Ordering::SeqCst);
            Err(AppError::other(
                "test_cleanup_probe_cannot_activate",
                serde_json::json!({}),
            ))
        }
    }
}

impl Drop for ResourceReservation<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(mut registry) = self.state.lifecycle_sessions.lock() else {
            return;
        };
        if let Some(record) = registry.get_mut(&self.session_id) {
            if record.nonce == self.nonce && record.phase == SessionPhase::Pending {
                record.phase = SessionPhase::Closed;
            }
        }
    }
}

pub fn reserve_resource<'a>(
    state: &'a AppState,
    session_id: &str,
    kind: SessionKind,
    owner: SessionOwner,
) -> AppResult<ResourceReservation<'a>> {
    let nonce = uuid::Uuid::new_v4();
    let mut registry = locked(&state.lifecycle_sessions)?;
    if registry.contains_key(session_id) {
        return Err(AppError::config(
            "session_id_conflict",
            serde_json::json!({ "id": session_id }),
        ));
    }
    registry.insert(
        session_id.to_owned(),
        SessionRecord {
            nonce,
            kind,
            owner,
            phase: SessionPhase::Pending,
            parent: None,
        },
    );
    drop(registry);
    Ok(ResourceReservation {
        state,
        session_id: session_id.to_owned(),
        nonce,
        kind,
        armed: true,
    })
}

pub fn reserve_generated_resource(
    state: &AppState,
    kind: SessionKind,
    owner: SessionOwner,
) -> AppResult<ResourceReservation<'_>> {
    loop {
        let candidate = uuid::Uuid::new_v4().to_string();
        match reserve_resource(state, &candidate, kind, owner.clone()) {
            Ok(reservation) => return Ok(reservation),
            Err(error) if error.code() == "session_id_conflict" => continue,
            Err(error) => return Err(error),
        }
    }
}

pub fn reserve_sftp_child<'a>(
    state: &'a AppState,
    parent_id: &str,
    requester: &SessionOwner,
) -> AppResult<(ResourceReservation<'a>, crate::ssh::client::SshHandle)> {
    let mut registry = locked(&state.lifecycle_sessions)?;
    let parent = registry
        .get(parent_id)
        .ok_or_else(|| AppError::not_found("ssh_session_not_found_msg", serde_json::json!({})))?;
    if parent.kind != SessionKind::Ssh || parent.phase != SessionPhase::Ready {
        return Err(AppError::not_found(
            "ssh_session_not_found_msg",
            serde_json::json!({}),
        ));
    }
    if &parent.owner != requester {
        return Err(AppError::config(
            "session_owner_mismatch",
            serde_json::json!({ "id": parent_id }),
        ));
    }
    let owner = parent.owner.clone();
    let ssh_handle = locked(&state.sessions)?
        .get(parent_id)
        .ok_or_else(|| {
            AppError::other(
                "session_registry_inconsistent",
                serde_json::json!({ "id": parent_id }),
            )
        })?
        .ssh_handle()
        .clone();
    let session_id = loop {
        let candidate = uuid::Uuid::new_v4().to_string();
        if !registry.contains_key(&candidate) {
            break candidate;
        }
    };
    let nonce = uuid::Uuid::new_v4();
    registry.insert(
        session_id.clone(),
        SessionRecord {
            nonce,
            kind: SessionKind::Sftp,
            owner,
            phase: SessionPhase::Pending,
            parent: Some(parent_id.to_owned()),
        },
    );
    drop(registry);
    Ok((
        ResourceReservation {
            state,
            session_id,
            nonce,
            kind: SessionKind::Sftp,
            armed: true,
        },
        ssh_handle,
    ))
}

fn take_ready_handle(
    state: &AppState,
    session_id: &str,
    kind: SessionKind,
) -> AppResult<Option<ReadySession>> {
    Ok(match kind {
        SessionKind::Ssh => locked(&state.sessions)?
            .remove(session_id)
            .map(ReadySession::Ssh),
        #[cfg(not(target_os = "android"))]
        SessionKind::Pty => locked(&state.pty_sessions)?
            .remove(session_id)
            .map(ReadySession::Pty),
        #[cfg(not(target_os = "android"))]
        SessionKind::Serial => locked(&state.serial_sessions)?
            .remove(session_id)
            .map(ReadySession::Serial),
        SessionKind::Telnet => locked(&state.telnet_sessions)?
            .remove(session_id)
            .map(ReadySession::Telnet),
        SessionKind::Sftp => locked(&state.sftp_sessions)?
            .remove(session_id)
            .map(ReadySession::Sftp),
        SessionKind::Forward => locked(&state.active_forwards)?
            .remove(session_id)
            .map(ReadySession::Forward),
    })
}

fn close_removed(mut removed: Vec<ReadySession>) {
    removed.sort_by_key(|handle| match handle.kind() {
        SessionKind::Sftp => 0,
        SessionKind::Ssh => 2,
        _ => 1,
    });
    for handle in removed {
        handle.close();
    }
}

pub fn close_resource(
    state: &AppState,
    session_id: &str,
    expected_kind: SessionKind,
    expected_owner: &SessionOwner,
) -> AppResult<()> {
    if expected_kind == SessionKind::Ssh {
        return close_ssh_tree(state, session_id, expected_owner);
    }
    let mut registry = locked(&state.lifecycle_sessions)?;
    let record = registry.get_mut(session_id).ok_or_else(|| {
        AppError::not_found("session_not_found", serde_json::json!({ "id": session_id }))
    })?;
    if record.kind != expected_kind {
        return Err(AppError::config(
            "session_kind_mismatch",
            serde_json::json!({ "id": session_id }),
        ));
    }
    if &record.owner != expected_owner {
        return Err(AppError::config(
            "session_owner_mismatch",
            serde_json::json!({ "id": session_id }),
        ));
    }
    if record.phase == SessionPhase::Closed {
        return Err(AppError::not_found(
            "session_not_found",
            serde_json::json!({ "id": session_id }),
        ));
    }
    let removed = if record.phase == SessionPhase::Ready {
        Some(
            take_ready_handle(state, session_id, record.kind)?.ok_or_else(|| {
                AppError::other(
                    "session_registry_inconsistent",
                    serde_json::json!({ "id": session_id }),
                )
            })?,
        )
    } else {
        None
    };
    record.phase = SessionPhase::Closed;
    drop(registry);
    if let Some(handle) = removed {
        handle.close();
    }
    close_waiters_for_resource(state, session_id, expected_owner)?;
    Ok(())
}

pub fn close_ssh_tree(
    state: &AppState,
    session_id: &str,
    expected_owner: &SessionOwner,
) -> AppResult<()> {
    let mut registry = locked(&state.lifecycle_sessions)?;
    let parent = registry.get(session_id).ok_or_else(|| {
        AppError::not_found("session_not_found", serde_json::json!({ "id": session_id }))
    })?;
    if parent.kind != SessionKind::Ssh {
        return Err(AppError::config(
            "session_kind_mismatch",
            serde_json::json!({ "id": session_id }),
        ));
    }
    if &parent.owner != expected_owner {
        return Err(AppError::config(
            "session_owner_mismatch",
            serde_json::json!({ "id": session_id }),
        ));
    }
    if parent.phase == SessionPhase::Closed {
        return Err(AppError::not_found(
            "session_not_found",
            serde_json::json!({ "id": session_id }),
        ));
    }

    let child_ids: Vec<String> = registry
        .iter()
        .filter(|(_, record)| {
            record.parent.as_deref() == Some(session_id) && record.phase != SessionPhase::Closed
        })
        .map(|(id, _)| id.clone())
        .collect();
    let mut removed = Vec::new();
    for child_id in child_ids {
        let child = registry
            .get_mut(&child_id)
            .expect("child came from registry");
        if child.kind != SessionKind::Sftp {
            return Err(AppError::other(
                "session_registry_inconsistent",
                serde_json::json!({ "id": child_id }),
            ));
        }
        if child.phase == SessionPhase::Ready {
            removed.push(
                take_ready_handle(state, &child_id, SessionKind::Sftp)?.ok_or_else(|| {
                    AppError::other(
                        "session_registry_inconsistent",
                        serde_json::json!({ "id": child_id }),
                    )
                })?,
            );
        }
        child.phase = SessionPhase::Closed;
    }

    let parent = registry.get_mut(session_id).expect("parent was validated");
    if parent.phase == SessionPhase::Ready {
        removed.push(
            take_ready_handle(state, session_id, SessionKind::Ssh)?.ok_or_else(|| {
                AppError::other(
                    "session_registry_inconsistent",
                    serde_json::json!({ "id": session_id }),
                )
            })?,
        );
    }
    parent.phase = SessionPhase::Closed;
    drop(registry);
    close_removed(removed);
    close_waiters_for_resource(state, session_id, expected_owner)?;
    Ok(())
}

fn remove_owned_resources(
    state: &AppState,
    owner: &SessionOwner,
    active_ids: Option<&HashSet<String>>,
) -> AppResult<(usize, Vec<ReadySession>)> {
    let mut registry = locked(&state.lifecycle_sessions)?;
    let mut ids: Vec<String> = registry
        .iter()
        .filter(|(id, record)| {
            record.owner == *owner
                && record.phase != SessionPhase::Closed
                && active_ids.is_none_or(|active| !active.contains(*id))
        })
        .map(|(id, _)| id.clone())
        .collect();
    let closing_parents: HashSet<String> = ids
        .iter()
        .filter(|id| {
            registry
                .get(*id)
                .is_some_and(|record| record.kind == SessionKind::Ssh)
        })
        .cloned()
        .collect();
    let child_ids: Vec<String> = registry
        .iter()
        .filter(|(id, record)| {
            !ids.contains(id)
                && record.phase != SessionPhase::Closed
                && record
                    .parent
                    .as_ref()
                    .is_some_and(|parent| closing_parents.contains(parent))
        })
        .map(|(id, _)| id.clone())
        .collect();
    ids.extend(child_ids);
    let mut removed = Vec::new();
    for id in &ids {
        let record = registry.get_mut(id).expect("id came from registry");
        if record.phase == SessionPhase::Ready {
            removed.push(take_ready_handle(state, id, record.kind)?.ok_or_else(|| {
                AppError::other(
                    "session_registry_inconsistent",
                    serde_json::json!({ "id": id }),
                )
            })?);
        }
        record.phase = SessionPhase::Closed;
    }
    Ok((ids.len(), removed))
}

pub fn reconcile_owner(
    state: &AppState,
    owner: &SessionOwner,
    active_ids: Vec<String>,
) -> AppResult<usize> {
    let active: HashSet<String> = active_ids.into_iter().collect();
    let (closed, removed) = remove_owned_resources(state, owner, Some(&active))?;
    close_removed(removed);
    Ok(closed
        + close_owned_ai(state, owner, Some(&active))?
        + close_owned_waiters(state, owner, Some(&active))?)
}

pub fn close_owner(state: &AppState, owner: &SessionOwner) {
    match remove_owned_resources(state, owner, None) {
        Ok((_, removed)) => close_removed(removed),
        Err(error) => log::warn!("close owner sessions failed: {error}"),
    }
    if let Err(error) = close_owned_ai(state, owner, None) {
        log::warn!("close owner AI sessions failed: {error}");
    }
    if let Err(error) = close_owned_waiters(state, owner, None) {
        log::warn!("close owner prompt waiters failed: {error}");
    }
}

pub fn reconcile_sessions_impl(
    state: &AppState,
    owner: &SessionOwner,
    active_ids: Vec<String>,
) -> AppResult<usize> {
    reconcile_owner(state, owner, active_ids)
}

pub fn close_window_sessions(state: &AppState, window_label: &str) {
    close_owner(state, &SessionOwner::Window(window_label.to_owned()));
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;

    fn empty_state() -> AppState {
        let db = Arc::new(crate::db::Db::open_in_memory().unwrap());
        let secret_store: Arc<dyn crate::secret::SecretStore> =
            Arc::new(crate::secret::DbStore::new(db.clone()));
        AppState {
            db,
            secret_store,
            lifecycle_sessions: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            #[cfg(not(target_os = "android"))]
            pty_sessions: Mutex::new(HashMap::new()),
            #[cfg(not(target_os = "android"))]
            serial_sessions: Mutex::new(HashMap::new()),
            telnet_sessions: Mutex::new(HashMap::new()),
            sftp_sessions: Mutex::new(HashMap::new()),
            transfer_cancels: Mutex::new(HashMap::new()),
            active_forwards: Mutex::new(HashMap::new()),
            auth_waiters: Mutex::new(HashMap::new()),
            passphrase_waiters: Mutex::new(HashMap::new()),
            host_key_waiters: Mutex::new(HashMap::new()),
            passphrase_cache: Mutex::new(HashMap::new()),
            #[cfg(desktop)]
            window_groups: Mutex::new(crate::commands::window::WindowGroups::default()),
            ai_sessions: Mutex::new(HashMap::new()),
            ai_session_owners: Mutex::new(HashMap::new()),
            ai_remote_shell_cache: Mutex::new(HashMap::new()),
            data_dir: PathBuf::new(),
        }
    }

    #[test]
    fn reserved_session_id_is_canonical() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(resolve_session_id(Some(id.into())).unwrap(), id);
        assert_eq!(
            resolve_session_id(Some("550E8400E29B41D4A716446655440000".into()))
                .unwrap_err()
                .code(),
            "session_id_invalid"
        );
    }

    #[test]
    fn missing_session_id_is_generated_and_invalid_id_is_rejected() {
        assert!(uuid::Uuid::parse_str(&resolve_session_id(None).unwrap()).is_ok());
        assert_eq!(
            resolve_session_id(Some("not-a-uuid".into()))
                .unwrap_err()
                .code(),
            "session_id_invalid"
        );
    }

    #[test]
    fn cancelled_session_id_cannot_be_reused() {
        let state = empty_state();
        let owner = crate::state::SessionOwner::Window("main".into());
        let reservation = reserve_resource(
            &state,
            "550e8400-e29b-41d4-a716-446655440000",
            crate::state::SessionKind::Pty,
            owner.clone(),
        )
        .unwrap();

        drop(reservation);

        let error = match reserve_resource(
            &state,
            "550e8400-e29b-41d4-a716-446655440000",
            crate::state::SessionKind::Pty,
            owner,
        ) {
            Ok(_) => panic!("cancelled id was reused"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "session_id_conflict");
    }

    #[test]
    fn wrong_owner_or_kind_cannot_cancel_pending_session() {
        let state = empty_state();
        let owner = crate::state::SessionOwner::Window("main".into());
        let other = crate::state::SessionOwner::Window("other".into());
        let reservation = reserve_resource(
            &state,
            "550e8400-e29b-41d4-a716-446655440001",
            crate::state::SessionKind::Pty,
            owner.clone(),
        )
        .unwrap();

        assert!(close_resource(
            &state,
            reservation.id(),
            crate::state::SessionKind::Pty,
            &other,
        )
        .is_err());
        reservation.ensure_pending().unwrap();

        assert!(close_resource(
            &state,
            reservation.id(),
            crate::state::SessionKind::Serial,
            &owner,
        )
        .is_err());
        reservation.ensure_pending().unwrap();

        close_resource(
            &state,
            reservation.id(),
            crate::state::SessionKind::Pty,
            &owner,
        )
        .unwrap();
        assert_eq!(
            reservation.ensure_pending().unwrap_err().code(),
            "session_reservation_lost"
        );
    }

    #[test]
    fn reconcile_is_scoped_to_one_owner() {
        let state = empty_state();
        let owner_a = SessionOwner::Window("a".into());
        let owner_b = SessionOwner::Window("b".into());
        let a = reserve_resource(
            &state,
            "550e8400-e29b-41d4-a716-446655440010",
            SessionKind::Pty,
            owner_a.clone(),
        )
        .unwrap();
        let b = reserve_resource(
            &state,
            "550e8400-e29b-41d4-a716-446655440011",
            SessionKind::Pty,
            owner_b,
        )
        .unwrap();

        assert_eq!(reconcile_owner(&state, &owner_a, Vec::new()).unwrap(), 1);

        assert_eq!(
            a.ensure_pending().unwrap_err().code(),
            "session_reservation_lost"
        );
        b.ensure_pending().unwrap();
    }

    #[test]
    fn late_activation_cleans_handle_after_pending_close() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let state = empty_state();
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_resource(
            &state,
            "550e8400-e29b-41d4-a716-446655440020",
            SessionKind::Pty,
            owner.clone(),
        )
        .unwrap();
        close_resource(&state, reservation.id(), SessionKind::Pty, &owner).unwrap();
        let cleaned = Arc::new(AtomicBool::new(false));

        let error = reservation
            .activate(ReadySession::CleanupProbe {
                kind: SessionKind::Pty,
                cleaned: cleaned.clone(),
            })
            .unwrap_err();

        assert_eq!(error.code(), "session_reservation_lost");
        assert!(cleaned.load(Ordering::SeqCst));
        assert!(state.pty_sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn prompt_cannot_register_after_pending_connection_is_closed() {
        let state = empty_state();
        let owner = SessionOwner::Window("main".into());
        let session_id = "550e8400-e29b-41d4-a716-446655440023";
        let reservation =
            reserve_resource(&state, session_id, SessionKind::Ssh, owner.clone()).unwrap();
        close_resource(&state, session_id, SessionKind::Ssh, &owner).unwrap();
        let (sender, _receiver) = tokio::sync::oneshot::channel::<String>();

        let error = register_prompt_waiter(
            &state,
            &state.passphrase_waiters,
            session_id,
            session_id,
            &owner,
            "ssh:passphrase_prompt",
            uuid::Uuid::new_v4(),
            sender,
        )
        .unwrap_err();

        assert_eq!(error.code(), "session_reservation_lost");
        assert!(state.passphrase_waiters.lock().unwrap().is_empty());
        drop(reservation);
    }

    #[test]
    fn backend_cannot_replace_reserved_session_id() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let state = empty_state();
        let reservation = reserve_resource(
            &state,
            "550e8400-e29b-41d4-a716-446655440021",
            SessionKind::Pty,
            SessionOwner::Window("main".into()),
        )
        .unwrap();
        let cleaned = Arc::new(AtomicBool::new(false));

        let error = reservation
            .activate_returned(
                "550e8400-e29b-41d4-a716-446655440022",
                ReadySession::CleanupProbe {
                    kind: SessionKind::Pty,
                    cleaned: cleaned.clone(),
                },
            )
            .unwrap_err();

        assert_eq!(error.code(), "session_id_mismatch");
        assert!(cleaned.load(Ordering::SeqCst));
        assert_eq!(
            state
                .lifecycle_sessions
                .lock()
                .unwrap()
                .get("550e8400-e29b-41d4-a716-446655440021")
                .unwrap()
                .phase,
            SessionPhase::Closed
        );
    }

    fn fake_ai_session(
        tab_id: &str,
        conversation_id: &str,
    ) -> (
        crate::ai::session::DiagnoseSession,
        tokio::sync::mpsc::UnboundedReceiver<crate::ai::session::UserAction>,
    ) {
        let (action_tx, action_rx) = tokio::sync::mpsc::unbounded_channel();
        (
            crate::ai::session::DiagnoseSession {
                tab_id: tab_id.to_owned(),
                target_id: "target".to_owned(),
                skill: "general".to_owned(),
                model: "model".to_owned(),
                provider: "provider".to_owned(),
                action_tx,
                audit: Arc::new(Mutex::new(crate::ai::audit::AuditLog::default())),
                cancel_slot: Arc::new(Mutex::new(None)),
                conversation_id: conversation_id.to_owned(),
                target_key: "local".to_owned(),
            },
            action_rx,
        )
    }

    #[test]
    fn late_ai_activation_after_close_stops_actor_and_does_not_publish_session() {
        let state = empty_state();
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();

        close_ai_session(&state, "tab", &owner).unwrap();
        let replacement = reserve_ai_owner(&state, "tab".into(), owner).unwrap();
        let (session, mut actions) = fake_ai_session("tab", "conversation");
        let error = reservation.activate(session).unwrap_err();

        assert_eq!(error.code(), "session_reservation_lost");
        assert!(matches!(
            actions.try_recv(),
            Ok(crate::ai::session::UserAction::Stop)
        ));
        assert!(state.ai_sessions.lock().unwrap().is_empty());

        let (replacement_session, _replacement_actions) =
            fake_ai_session("tab", "replacement-conversation");
        replacement.activate(replacement_session).unwrap();
        assert!(state.ai_sessions.lock().unwrap().contains_key("tab"));
        assert_eq!(
            state
                .ai_session_owners
                .lock()
                .unwrap()
                .get("tab")
                .unwrap()
                .phase,
            SessionPhase::Ready
        );
    }

    #[test]
    fn closed_ai_tab_id_can_be_reserved_again() {
        let state = empty_state();
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (session, mut actions) = fake_ai_session("tab", "conversation");
        reservation.activate(session).unwrap();

        close_ai_session(&state, "tab", &owner).unwrap();
        assert!(matches!(
            actions.try_recv(),
            Ok(crate::ai::session::UserAction::Stop)
        ));

        let replacement = reserve_ai_owner(&state, "tab".into(), owner).unwrap();
        drop(replacement);
        assert!(state.ai_session_owners.lock().unwrap().is_empty());
    }

    #[test]
    fn closing_owner_does_not_cancel_another_owners_prompt() {
        let state = empty_state();
        let owner_a = SessionOwner::Headless(uuid::Uuid::new_v4());
        let owner_b = SessionOwner::Headless(uuid::Uuid::new_v4());
        let (sender_a, mut receiver_a) = tokio::sync::oneshot::channel();
        let (sender_b, mut receiver_b) = tokio::sync::oneshot::channel();
        {
            let mut waiters = state.passphrase_waiters.lock().unwrap();
            waiters.insert(
                "a".into(),
                crate::state::OwnedWaiter {
                    nonce: uuid::Uuid::new_v4(),
                    owner: owner_a.clone(),
                    sender: sender_a,
                },
            );
            waiters.insert(
                "b".into(),
                crate::state::OwnedWaiter {
                    nonce: uuid::Uuid::new_v4(),
                    owner: owner_b,
                    sender: sender_b,
                },
            );
        }

        close_owner(&state, &owner_a);

        assert!(matches!(
            receiver_a.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
        assert!(state.passphrase_waiters.lock().unwrap().contains_key("b"));
        assert!(matches!(
            receiver_b.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
    }
}
