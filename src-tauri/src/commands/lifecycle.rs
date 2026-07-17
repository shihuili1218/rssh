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
    pub fn instance_id(&self) -> uuid::Uuid {
        self.nonce
    }

    /// Atomically lease a conversation before loading or creating its row.
    /// The lease lives in the owner record, so pending startup, a running actor,
    /// and stopping cleanup all share one source of truth.
    pub fn claim_conversation(&mut self, conversation_id: &str) -> AppResult<()> {
        let mut owners = locked(&self.state.ai_session_owners)?;
        let Some(record) = owners.get(&self.tab_id) else {
            return Err(AppError::not_found(
                "session_reservation_lost",
                serde_json::json!({ "id": self.tab_id }),
            ));
        };
        if record.owner != self.owner
            || record.nonce != self.nonce
            || record.phase != SessionPhase::Pending
        {
            return Err(AppError::not_found(
                "session_reservation_lost",
                serde_json::json!({ "id": self.tab_id }),
            ));
        }
        if record.conversation_id.as_deref() == Some(conversation_id) {
            return Ok(());
        }
        if record.conversation_id.is_some()
            || owners.iter().any(|(tab_id, existing)| {
                tab_id != &self.tab_id
                    && existing.conversation_id.as_deref() == Some(conversation_id)
            })
        {
            return Err(AppError::other(
                "conversation_in_use",
                serde_json::json!({}),
            ));
        }
        owners
            .get_mut(&self.tab_id)
            .expect("AI owner was validated")
            .conversation_id = Some(conversation_id.to_owned());
        Ok(())
    }

    pub fn activate(self, pending: crate::ai::session::PendingSession) -> AppResult<()> {
        let conversation_id = pending.info().conversation_id.clone();
        self.activate_after_validation(&conversation_id, |instance_id| pending.launch(instance_id))
    }

    fn activate_after_validation(
        mut self,
        conversation_id: &str,
        launch: impl FnOnce(uuid::Uuid) -> crate::ai::session::DiagnoseSession,
    ) -> AppResult<()> {
        let mut owners = locked(&self.state.ai_session_owners)?;
        if !owners.get(&self.tab_id).is_some_and(|record| {
            record.owner == self.owner
                && record.nonce == self.nonce
                && record.phase == SessionPhase::Pending
                && record.conversation_id.as_deref() == Some(conversation_id)
        }) {
            return Err(AppError::not_found(
                "session_reservation_lost",
                serde_json::json!({ "id": self.tab_id }),
            ));
        }
        if owners.iter().any(|(tab_id, existing)| {
            tab_id != &self.tab_id && existing.conversation_id.as_deref() == Some(conversation_id)
        }) {
            return Err(AppError::other(
                "conversation_in_use",
                serde_json::json!({}),
            ));
        }
        let mut sessions = locked(&self.state.ai_sessions)?;
        if sessions.contains_key(&self.tab_id) {
            return Err(AppError::other(
                "session_already_exists",
                serde_json::json!({ "tab_id": self.tab_id }),
            ));
        }
        // Launch is the first actor-side effect. All reservation and registry
        // checks must succeed before it can run; a loser is dropped unlaunched.
        let session = launch(self.nonce);
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
            conversation_id: None,
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

fn spawn_ai_stop(
    owners: std::sync::Arc<Mutex<HashMap<String, AiSessionRecord>>>,
    tab_id: String,
    owner: SessionOwner,
    nonce: uuid::Uuid,
    session: crate::ai::session::DiagnoseSession,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        session.stop().await;
        let Ok(mut owners) = owners.lock() else {
            log::warn!("AI owner cleanup lock poisoned for tab {tab_id}");
            return;
        };
        if owners.get(&tab_id).is_some_and(|record| {
            record.owner == owner && record.nonce == nonce && record.phase == SessionPhase::Closed
        }) {
            owners.remove(&tab_id);
        }
    })
}

pub async fn prepare_ai_session_stop(
    state: &AppState,
    tab_id: &str,
    expected_owner: &SessionOwner,
    expected_instance_id: Option<&str>,
) -> AppResult<Vec<crate::ai::session::AiTerminalMutation>> {
    let (mut actor_done, terminal_mutations) = {
        let owners = locked(&state.ai_session_owners)?;
        let record = owners
            .get(tab_id)
            .ok_or_else(|| AppError::not_found("ai_session_not_found", serde_json::json!({})))?;
        if &record.owner != expected_owner
            || record.phase != SessionPhase::Ready
            || expected_instance_id.is_some_and(|expected| expected != record.nonce.to_string())
        {
            return Err(AppError::not_found(
                "ai_session_not_found",
                serde_json::json!({}),
            ));
        }
        let sessions = locked(&state.ai_sessions)?;
        let session = sessions.get(tab_id).ok_or_else(|| {
            AppError::other(
                "session_registry_inconsistent",
                serde_json::json!({ "id": tab_id }),
            )
        })?;
        session.request_stop();
        (
            session.actor_done_rx.clone(),
            session.terminal_mutations.clone(),
        )
    };

    // This is the explicit actor/event drain barrier. Never hold either
    // registry mutex while waiting: full stop and owner cleanup need them.
    loop {
        if *actor_done.borrow_and_update() {
            break;
        }
        if actor_done.changed().await.is_err() {
            // Sender drop means the actor task terminated abnormally. It still
            // cannot emit another mutation, so the snapshot is final.
            break;
        }
    }

    let snapshot = locked(&terminal_mutations)?.clone();
    Ok(snapshot)
}

pub async fn close_ai_session(
    state: &AppState,
    tab_id: &str,
    expected_owner: &SessionOwner,
    expected_instance_id: Option<&str>,
) -> AppResult<()> {
    let (nonce, session) = {
        let mut owners = locked(&state.ai_session_owners)?;
        let record = owners
            .get(tab_id)
            .ok_or_else(|| AppError::not_found("ai_session_not_found", serde_json::json!({})))?;
        if &record.owner != expected_owner
            || record.phase == SessionPhase::Closed
            || expected_instance_id.is_some_and(|expected| expected != record.nonce.to_string())
        {
            return Err(AppError::not_found(
                "ai_session_not_found",
                serde_json::json!({}),
            ));
        }
        let nonce = record.nonce;
        let phase = record.phase;

        if phase == SessionPhase::Pending {
            owners.remove(tab_id);
            return Ok(());
        }

        let session = locked(&state.ai_sessions)?.remove(tab_id).ok_or_else(|| {
            AppError::other(
                "session_registry_inconsistent",
                serde_json::json!({ "id": tab_id }),
            )
        })?;
        owners
            .get_mut(tab_id)
            .expect("AI owner was validated")
            .phase = SessionPhase::Closed;
        (nonce, session)
    };

    // Cleanup owns the actor and tombstone in a detached task. Awaiting its
    // JoinHandle gives explicit close a barrier; if the caller disappears, the
    // task still finishes and releases the lease instead of stranding Closed.
    let cleanup = spawn_ai_stop(
        state.ai_session_owners.clone(),
        tab_id.to_owned(),
        expected_owner.clone(),
        nonce,
        session,
    );
    let _ = cleanup.await;
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
    let mut closed_count = 0;
    for id in &ids {
        let phase = owners
            .get(id)
            .expect("id came from AI owner registry")
            .phase;
        match phase {
            SessionPhase::Pending => {
                owners.remove(id);
                closed_count += 1;
            }
            SessionPhase::Ready => {
                let session = sessions.remove(id).expect("ready AI session was validated");
                let record = owners.get_mut(id).expect("AI owner was validated");
                record.phase = SessionPhase::Closed;
                removed.push((id.clone(), record.owner.clone(), record.nonce, session));
                closed_count += 1;
            }
            // An explicit close already owns the cleanup task. Leave its
            // tombstone alone; owner shutdown must not reopen the ABA window.
            SessionPhase::Closed => {}
        }
    }
    drop(sessions);
    drop(owners);
    for (id, session_owner, nonce, session) in removed {
        drop(spawn_ai_stop(
            state.ai_session_owners.clone(),
            id,
            session_owner,
            nonce,
            session,
        ));
    }
    Ok(closed_count)
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

/// A live terminal target after its lifecycle owner has been verified. AI
/// start/rebind needs a small, typed subset of the transport handles, so keep
/// that lookup here with the lifecycle lock held before the concrete map.
pub enum OwnedAiTarget {
    Ssh {
        handle: crate::ssh::client::SshHandle,
        profile_id: String,
    },
    #[cfg(not(target_os = "android"))]
    PtyShellPath(String),
    #[cfg(not(target_os = "android"))]
    Serial,
    Telnet,
}

pub fn owned_ready_ai_target(
    state: &AppState,
    id: &str,
    kind: SessionKind,
    expected_owner: &SessionOwner,
) -> AppResult<OwnedAiTarget> {
    let registry = locked(&state.lifecycle_sessions)?;
    let record = registry
        .get(id)
        .ok_or_else(|| AppError::not_found("session_not_found", serde_json::json!({ "id": id })))?;
    if record.kind != kind || record.phase != SessionPhase::Ready {
        return Err(AppError::not_found(
            "session_not_found",
            serde_json::json!({ "id": id }),
        ));
    }
    if &record.owner != expected_owner {
        return Err(AppError::config(
            "session_owner_mismatch",
            serde_json::json!({ "id": id }),
        ));
    }

    // Keep `lifecycle_sessions -> concrete handle` ordering. Resource close
    // uses the same ordering, so an owned lookup cannot race a close into a
    // borrowed handle from a different lifecycle epoch.
    match kind {
        SessionKind::Ssh => locked(&state.sessions)?
            .get(id)
            .map(|session| OwnedAiTarget::Ssh {
                handle: session.ssh_handle().clone(),
                profile_id: session.profile_id().to_owned(),
            })
            .ok_or_else(|| {
                AppError::other(
                    "session_registry_inconsistent",
                    serde_json::json!({ "id": id }),
                )
            }),
        #[cfg(not(target_os = "android"))]
        SessionKind::Pty => locked(&state.pty_sessions)?
            .get(id)
            .map(|session| OwnedAiTarget::PtyShellPath(session.shell_path().to_owned()))
            .ok_or_else(|| {
                AppError::other(
                    "session_registry_inconsistent",
                    serde_json::json!({ "id": id }),
                )
            }),
        #[cfg(not(target_os = "android"))]
        SessionKind::Serial => locked(&state.serial_sessions)?
            .contains_key(id)
            .then_some(OwnedAiTarget::Serial)
            .ok_or_else(|| {
                AppError::other(
                    "session_registry_inconsistent",
                    serde_json::json!({ "id": id }),
                )
            }),
        SessionKind::Telnet => locked(&state.telnet_sessions)?
            .contains_key(id)
            .then_some(OwnedAiTarget::Telnet)
            .ok_or_else(|| {
                AppError::other(
                    "session_registry_inconsistent",
                    serde_json::json!({ "id": id }),
                )
            }),
        SessionKind::Sftp | SessionKind::Forward => Err(AppError::config(
            "session_kind_mismatch",
            serde_json::json!({ "id": id }),
        )),
    }
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

    use async_trait::async_trait;

    use super::*;

    struct CommandProposalClient {
        calls: std::sync::atomic::AtomicUsize,
    }

    struct StreamingClient;

    struct FileOpClient {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl crate::ai::llm::LlmClient for CommandProposalClient {
        async fn chat(
            &self,
            _req: crate::ai::llm::ChatRequest,
            _sink: crate::ai::llm::DeltaSink,
        ) -> AppResult<crate::ai::llm::ChatResponse> {
            use std::sync::atomic::Ordering;

            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Ok(crate::ai::llm::ChatResponse {
                    text: String::new(),
                    tool_calls: vec![crate::ai::llm::ToolCall {
                        id: "tool-call".into(),
                        name: crate::ai::tools::TOOL_RUN_COMMAND.into(),
                        input: serde_json::json!({
                            "cmd": "echo ready",
                            "explain": "wait for approval",
                            "side_effect": "none",
                            "timeout_s": 60,
                        }),
                    }],
                    stop_reason: "tool_use".into(),
                    tokens_in: None,
                    tokens_out: None,
                    reasoning_content: None,
                });
            }

            std::future::pending().await
        }

        async fn list_models(&self) -> AppResult<Vec<crate::ai::llm::ModelInfo>> {
            Ok(Vec::new())
        }

        fn provider(&self) -> &'static str {
            "test"
        }
    }

    #[async_trait]
    impl crate::ai::llm::LlmClient for StreamingClient {
        async fn chat(
            &self,
            _req: crate::ai::llm::ChatRequest,
            sink: crate::ai::llm::DeltaSink,
        ) -> AppResult<crate::ai::llm::ChatResponse> {
            sink(crate::ai::llm::ChatDelta::Text("partial response".into()));
            std::future::pending().await
        }

        async fn list_models(&self) -> AppResult<Vec<crate::ai::llm::ModelInfo>> {
            Ok(Vec::new())
        }

        fn provider(&self) -> &'static str {
            "test"
        }
    }

    #[async_trait]
    impl crate::ai::llm::LlmClient for FileOpClient {
        async fn chat(
            &self,
            _req: crate::ai::llm::ChatRequest,
            _sink: crate::ai::llm::DeltaSink,
        ) -> AppResult<crate::ai::llm::ChatResponse> {
            use std::sync::atomic::Ordering;

            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Ok(crate::ai::llm::ChatResponse {
                    text: String::new(),
                    tool_calls: vec![crate::ai::llm::ToolCall {
                        id: "file-tool".into(),
                        name: crate::ai::tools::TOOL_MATCH_FILE.into(),
                        input: serde_json::json!({
                            "path": "/tmp/test",
                            "find": "needle",
                            "before": 0,
                            "after": 0,
                        }),
                    }],
                    stop_reason: "tool_use".into(),
                    tokens_in: None,
                    tokens_out: None,
                    reasoning_content: None,
                });
            }

            std::future::pending().await
        }

        async fn list_models(&self) -> AppResult<Vec<crate::ai::llm::ModelInfo>> {
            Ok(Vec::new())
        }

        fn provider(&self) -> &'static str {
            "test"
        }
    }

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
            ai_session_owners: Arc::new(Mutex::new(HashMap::new())),
            ai_remote_shell_cache: Mutex::new(HashMap::new()),
            data_dir: PathBuf::new(),
        }
    }

    fn command_proposal_session(
        state: &Arc<AppState>,
        tab_id: &str,
    ) -> (
        crate::ai::session::PendingSession,
        tokio::sync::mpsc::UnboundedReceiver<(String, serde_json::Value)>,
    ) {
        let conversation_id = uuid::Uuid::new_v4().to_string();
        crate::db::ai_conversation::create(&state.db, &conversation_id, "local").unwrap();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let host = crate::emitter::Host::Headless {
            sink: Arc::new(move |event: &str, payload: serde_json::Value| {
                event_tx.send((event.to_owned(), payload)).is_ok()
            }),
            state: state.clone(),
        };
        let pending = crate::ai::session::start(
            crate::ai::session::SessionConfig {
                tab_id: tab_id.to_owned(),
                target_id: "target".into(),
                skill: "general".into(),
                system_prompt: "test".into(),
                user_skills_cache: Vec::new(),
                model: "test".into(),
                client: Box::new(CommandProposalClient {
                    calls: std::sync::atomic::AtomicUsize::new(0),
                }),
                redact_rules: Vec::new(),
                blacklist: crate::ai::sanitize::Blacklist::default(),
                max_output_bytes: crate::ai::sanitize::DEFAULT_MAX_OUTPUT_BYTES,
                ssh_handle: None,
                data_dir: PathBuf::new(),
                shell_kind: crate::ai::shell::ShellKind::default(),
                db: state.db.clone(),
                conversation_id,
                target_key: "local".into(),
                initial_history: Vec::new(),
            },
            host,
        )
        .unwrap();
        (pending, event_rx)
    }

    fn streaming_session(
        state: &Arc<AppState>,
        tab_id: &str,
    ) -> (
        crate::ai::session::PendingSession,
        tokio::sync::mpsc::UnboundedReceiver<(String, serde_json::Value)>,
    ) {
        let conversation_id = uuid::Uuid::new_v4().to_string();
        crate::db::ai_conversation::create(&state.db, &conversation_id, "local").unwrap();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let host = crate::emitter::Host::Headless {
            sink: Arc::new(move |event: &str, payload: serde_json::Value| {
                event_tx.send((event.to_owned(), payload)).is_ok()
            }),
            state: state.clone(),
        };
        let pending = crate::ai::session::start(
            crate::ai::session::SessionConfig {
                tab_id: tab_id.to_owned(),
                target_id: "target".into(),
                skill: "general".into(),
                system_prompt: "test".into(),
                user_skills_cache: Vec::new(),
                model: "test".into(),
                client: Box::new(StreamingClient),
                redact_rules: Vec::new(),
                blacklist: crate::ai::sanitize::Blacklist::default(),
                max_output_bytes: crate::ai::sanitize::DEFAULT_MAX_OUTPUT_BYTES,
                ssh_handle: None,
                data_dir: PathBuf::new(),
                shell_kind: crate::ai::shell::ShellKind::default(),
                db: state.db.clone(),
                conversation_id,
                target_key: "local".into(),
                initial_history: Vec::new(),
            },
            host,
        )
        .unwrap();
        (pending, event_rx)
    }

    fn file_op_session(
        state: &Arc<AppState>,
        tab_id: &str,
    ) -> (
        crate::ai::session::PendingSession,
        tokio::sync::mpsc::UnboundedReceiver<(String, serde_json::Value)>,
    ) {
        let conversation_id = uuid::Uuid::new_v4().to_string();
        crate::db::ai_conversation::create(&state.db, &conversation_id, "local").unwrap();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let host = crate::emitter::Host::Headless {
            sink: Arc::new(move |event: &str, payload: serde_json::Value| {
                event_tx.send((event.to_owned(), payload)).is_ok()
            }),
            state: state.clone(),
        };
        let pending = crate::ai::session::start(
            crate::ai::session::SessionConfig {
                tab_id: tab_id.to_owned(),
                target_id: "target".into(),
                skill: "general".into(),
                system_prompt: "test".into(),
                user_skills_cache: Vec::new(),
                model: "test".into(),
                client: Box::new(FileOpClient {
                    calls: std::sync::atomic::AtomicUsize::new(0),
                }),
                redact_rules: Vec::new(),
                blacklist: crate::ai::sanitize::Blacklist::default(),
                max_output_bytes: crate::ai::sanitize::DEFAULT_MAX_OUTPUT_BYTES,
                ssh_handle: None,
                data_dir: PathBuf::new(),
                shell_kind: crate::ai::shell::ShellKind::default(),
                db: state.db.clone(),
                conversation_id,
                target_key: "local".into(),
                initial_history: Vec::new(),
            },
            host,
        )
        .unwrap();
        (pending, event_rx)
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
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
        let (_actor_done_tx, actor_done_rx) = tokio::sync::watch::channel(true);
        (
            crate::ai::session::DiagnoseSession {
                tab_id: tab_id.to_owned(),
                instance_id: uuid::Uuid::nil(),
                target_id: "target".to_owned(),
                skill: "general".to_owned(),
                model: "model".to_owned(),
                provider: "provider".to_owned(),
                action_tx,
                audit: Arc::new(Mutex::new(crate::ai::audit::AuditLog::default())),
                cancel_slot: Arc::new(Mutex::new(None)),
                shutdown_tx,
                actor_task: None,
                actor_done_rx,
                terminal_mutations: Arc::new(Mutex::new(Vec::new())),
                conversation_id: conversation_id.to_owned(),
                target_key: "local".to_owned(),
            },
            action_rx,
        )
    }

    fn activate_fake_ai_session(
        mut reservation: AiOwnerReservation<'_>,
        session: crate::ai::session::DiagnoseSession,
    ) -> AppResult<()> {
        let conversation_id = session.conversation_id.clone();
        reservation.claim_conversation(&conversation_id)?;
        reservation.activate_after_validation(&conversation_id, |instance_id| {
            let mut session = session;
            session.instance_id = instance_id;
            session
        })
    }

    fn fake_running_ai_session(
        tab_id: &str,
        conversation_id: &str,
    ) -> (
        crate::ai::session::DiagnoseSession,
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        use std::sync::atomic::{AtomicBool, Ordering};

        let (action_tx, _action_rx) = tokio::sync::mpsc::unbounded_channel();
        let stream_cancel = Arc::new(tokio::sync::Notify::new());
        let stream_cancelled = Arc::new(AtomicBool::new(false));
        let actor_exited = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        let (actor_done_tx, actor_done_rx) = tokio::sync::watch::channel(false);
        let stream_cancel_for_actor = stream_cancel.clone();
        let stream_cancelled_for_actor = stream_cancelled.clone();
        let actor_exited_for_task = actor_exited.clone();
        let actor_task = tauri::async_runtime::spawn(async move {
            stream_cancel_for_actor.notified().await;
            stream_cancelled_for_actor.store(true, Ordering::SeqCst);
            if !*shutdown_rx.borrow_and_update() {
                shutdown_rx.changed().await.unwrap();
            }
            assert!(*shutdown_rx.borrow());
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            actor_exited_for_task.store(true, Ordering::SeqCst);
            actor_done_tx.send_replace(true);
        });

        (
            crate::ai::session::DiagnoseSession {
                tab_id: tab_id.to_owned(),
                instance_id: uuid::Uuid::nil(),
                target_id: "target".to_owned(),
                skill: "general".to_owned(),
                model: "model".to_owned(),
                provider: "provider".to_owned(),
                action_tx,
                audit: Arc::new(Mutex::new(crate::ai::audit::AuditLog::default())),
                cancel_slot: Arc::new(Mutex::new(Some(stream_cancel))),
                shutdown_tx,
                actor_task: Some(actor_task),
                actor_done_rx,
                terminal_mutations: Arc::new(Mutex::new(Vec::new())),
                conversation_id: conversation_id.to_owned(),
                target_key: "local".to_owned(),
            },
            stream_cancelled,
            actor_exited,
        )
    }

    #[test]
    fn pending_conversation_claim_blocks_delete_and_competing_start() {
        let state = empty_state();
        crate::db::ai_conversation::create(&state.db, "conversation", "local").unwrap();
        let owner = SessionOwner::Window("main".into());
        let mut first = reserve_ai_owner(&state, "tab-a".into(), owner.clone()).unwrap();
        first.claim_conversation("conversation").unwrap();

        let error =
            crate::ai::commands::ai_conversation_delete_impl(&state, "conversation").unwrap_err();
        assert_eq!(error.code(), "conversation_in_use");

        let mut competing = reserve_ai_owner(&state, "tab-b".into(), owner).unwrap();
        let error = competing.claim_conversation("conversation").unwrap_err();
        assert_eq!(error.code(), "conversation_in_use");

        drop(first);
        competing.claim_conversation("conversation").unwrap();
        drop(competing);
        crate::ai::commands::ai_conversation_delete_impl(&state, "conversation").unwrap();
    }

    #[test]
    fn legacy_timeline_fetch_without_target_remains_supported() {
        let state = empty_state();
        crate::db::ai_conversation::create(&state.db, "conversation", "ssh:profile").unwrap();
        crate::db::ai_conversation::set_timeline(&state.db, "conversation", r#"[{"role":"user"}]"#)
            .unwrap();

        let timeline =
            crate::ai::commands::ai_conversation_timeline_impl(&state, "conversation", None)
                .unwrap();

        assert_eq!(timeline, r#"[{"role":"user"}]"#);
    }

    #[test]
    fn scoped_timeline_fetch_rejects_target_mismatch() {
        let state = empty_state();
        crate::db::ai_conversation::create(&state.db, "conversation", "ssh:profile").unwrap();
        let target = crate::ai::commands::AiTarget::Local("pty".into());

        let error = crate::ai::commands::ai_conversation_timeline_impl(
            &state,
            "conversation",
            Some(&target),
        )
        .unwrap_err();

        assert_eq!(error.code(), "conversation_target_mismatch");
    }

    #[tokio::test]
    async fn failed_new_conversation_activation_removes_the_precreated_row() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, mut events) = command_proposal_session(&state, "tab");
        let conversation_id = pending.info().conversation_id.clone();
        crate::db::ai_conversation::delete(&state.db, &conversation_id).unwrap();
        reservation.claim_conversation(&conversation_id).unwrap();

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
        let error = crate::ai::commands::activate_pending_ai_session(
            &state,
            reservation,
            pending,
            true,
            "local",
        )
        .unwrap_err();

        assert_eq!(error.code(), "session_reservation_lost");
        assert!(crate::db::ai_conversation::get(&state.db, &conversation_id)
            .unwrap()
            .is_none());
        assert!(state.ai_sessions.lock().unwrap().is_empty());
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn late_ai_activation_after_close_is_rejected_without_publishing_session() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
        let replacement = reserve_ai_owner(&state, "tab".into(), owner).unwrap();
        let (session, _actions) = fake_ai_session("tab", "conversation");
        let error = activate_fake_ai_session(reservation, session).unwrap_err();

        assert_eq!(error.code(), "session_reservation_lost");
        assert!(state.ai_sessions.lock().unwrap().is_empty());

        let (replacement_session, _replacement_actions) =
            fake_ai_session("tab", "replacement-conversation");
        activate_fake_ai_session(replacement, replacement_session).unwrap();
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

    #[tokio::test]
    async fn late_ai_activation_after_close_does_not_launch_actor() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let state = empty_state();
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        close_ai_session(&state, "tab", &owner, None).await.unwrap();
        let _replacement = reserve_ai_owner(&state, "tab".into(), owner).unwrap();
        let launched = Arc::new(AtomicBool::new(false));
        let (session, _actions) = fake_ai_session("tab", "conversation");
        let conversation_id = session.conversation_id.clone();

        let launched_in_argument = launched.clone();
        let error = reservation
            .activate_after_validation(&conversation_id, |instance_id| {
                launched_in_argument.store(true, Ordering::SeqCst);
                let mut session = session;
                session.instance_id = instance_id;
                session
            })
            .unwrap_err();

        assert_eq!(error.code(), "session_reservation_lost");
        assert!(!launched.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn closed_ai_tab_id_can_be_reserved_again() {
        let state = empty_state();
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (session, mut actions) = fake_ai_session("tab", "conversation");
        activate_fake_ai_session(reservation, session).unwrap();

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
        assert!(matches!(
            actions.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
        ));

        let replacement = reserve_ai_owner(&state, "tab".into(), owner).unwrap();
        drop(replacement);
        assert!(state.ai_session_owners.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn close_ai_session_cancels_stream_and_waits_for_actor_exit() {
        use std::sync::atomic::Ordering;

        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (session, stream_cancelled, actor_exited) =
            fake_running_ai_session("tab", "conversation");
        activate_fake_ai_session(reservation, session).unwrap();

        let close_state = state.clone();
        let close_owner = owner.clone();
        let close_task =
            tokio::spawn(
                async move { close_ai_session(&close_state, "tab", &close_owner, None).await },
            );

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !stream_cancelled.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown never cancelled the active stream");

        let reserve_while_stopping = reserve_ai_owner(&state, "tab".into(), owner.clone());
        assert!(matches!(
            reserve_while_stopping,
            Err(ref error) if error.code() == "session_already_exists"
        ));

        close_task.await.unwrap().unwrap();

        assert!(stream_cancelled.load(Ordering::SeqCst));
        assert!(actor_exited.load(Ordering::SeqCst));
        assert!(state.ai_sessions.lock().unwrap().is_empty());
        assert!(state.ai_session_owners.lock().unwrap().is_empty());
        let replacement = reserve_ai_owner(&state, "tab".into(), owner).unwrap();
        drop(replacement);
    }

    #[tokio::test]
    async fn stopping_session_keeps_its_conversation_lease_until_actor_exit() {
        use std::sync::atomic::Ordering;

        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab-a".into(), owner.clone()).unwrap();
        let (session, stream_cancelled, _actor_exited) =
            fake_running_ai_session("tab-a", "conversation");
        activate_fake_ai_session(reservation, session).unwrap();

        let close_state = state.clone();
        let close_owner = owner.clone();
        let close_task = tokio::spawn(async move {
            close_ai_session(&close_state, "tab-a", &close_owner, None).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !stream_cancelled.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown never reached the stopping actor");

        let competing = reserve_ai_owner(&state, "tab-b".into(), owner.clone()).unwrap();
        let (competing_session, _actions) = fake_ai_session("tab-b", "conversation");
        let error = activate_fake_ai_session(competing, competing_session).unwrap_err();
        assert_eq!(error.code(), "conversation_in_use");

        close_task.await.unwrap().unwrap();
        let replacement = reserve_ai_owner(&state, "tab-b".into(), owner).unwrap();
        let (replacement_session, _actions) = fake_ai_session("tab-b", "conversation");
        activate_fake_ai_session(replacement, replacement_session).unwrap();
    }

    #[tokio::test]
    async fn stopping_session_prevents_deleting_its_conversation() {
        use std::sync::atomic::Ordering;

        let state = Arc::new(empty_state());
        crate::db::ai_conversation::create(&state.db, "conversation", "local").unwrap();
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (session, stream_cancelled, _actor_exited) =
            fake_running_ai_session("tab", "conversation");
        activate_fake_ai_session(reservation, session).unwrap();

        let close_state = state.clone();
        let close_owner = owner.clone();
        let close_task =
            tokio::spawn(
                async move { close_ai_session(&close_state, "tab", &close_owner, None).await },
            );
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !stream_cancelled.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("shutdown never reached the stopping actor");

        let error =
            crate::ai::commands::ai_conversation_delete_impl(&state, "conversation").unwrap_err();
        assert_eq!(error.code(), "conversation_in_use");

        close_task.await.unwrap().unwrap();
        crate::ai::commands::ai_conversation_delete_impl(&state, "conversation").unwrap();
        assert!(crate::db::ai_conversation::get(&state.db, "conversation")
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn close_owner_keeps_tombstone_until_background_actor_exit() {
        use std::sync::atomic::Ordering;

        let state = Arc::new(empty_state());
        let owner = SessionOwner::Headless(uuid::Uuid::new_v4());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (session, _stream_cancelled, actor_exited) =
            fake_running_ai_session("tab", "conversation");
        activate_fake_ai_session(reservation, session).unwrap();

        close_owner(&state, &owner);

        assert!(state.ai_sessions.lock().unwrap().is_empty());
        let record = state
            .ai_session_owners
            .lock()
            .unwrap()
            .get("tab")
            .cloned()
            .expect("stopping tombstone was released before actor exit");
        assert_eq!(record.phase, SessionPhase::Closed);
        assert_eq!(record.conversation_id.as_deref(), Some("conversation"));
        assert!(matches!(
            reserve_ai_owner(&state, "tab".into(), owner.clone()),
            Err(ref error) if error.code() == "session_already_exists"
        ));

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !actor_exited.load(Ordering::SeqCst)
                || state.ai_session_owners.lock().unwrap().contains_key("tab")
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("background actor cleanup never released the tombstone");

        let replacement = reserve_ai_owner(&state, "tab".into(), owner).unwrap();
        drop(replacement);
    }

    #[tokio::test]
    async fn stale_instance_id_cannot_close_current_session() {
        let state = empty_state();
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let instance_id = reservation.instance_id().to_string();
        let (session, _actions) = fake_ai_session("tab", "conversation");
        activate_fake_ai_session(reservation, session).unwrap();

        let error = close_ai_session(&state, "tab", &owner, Some("stale-instance"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ai_session_not_found");
        assert!(state.ai_sessions.lock().unwrap().contains_key("tab"));

        close_ai_session(&state, "tab", &owner, Some(&instance_id))
            .await
            .unwrap();
        assert!(state.ai_sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn stale_instance_id_cannot_target_current_actor() {
        let state = empty_state();
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let instance_id = reservation.instance_id().to_string();
        let (session, mut actions) = fake_ai_session("tab", "conversation");
        activate_fake_ai_session(reservation, session).unwrap();

        let error =
            crate::ai::commands::ai_action_sender(&state, "tab", &owner, Some("stale-instance"))
                .unwrap_err();
        assert_eq!(error.code(), "ai_session_not_found");
        assert!(matches!(
            actions.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        crate::ai::commands::ai_send_action(
            &state,
            "tab",
            &owner,
            Some(&instance_id),
            crate::ai::session::UserAction::ClearContext { ack: None },
        )
        .unwrap();
        assert!(matches!(
            actions.try_recv(),
            Ok(crate::ai::session::UserAction::ClearContext { ack: None })
        ));
    }

    #[test]
    fn stale_instance_id_cannot_read_current_audit() {
        let state = empty_state();
        let owner = SessionOwner::Window("main".into());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let instance_id = reservation.instance_id().to_string();
        let (session, _actions) = fake_ai_session("tab", "conversation");
        activate_fake_ai_session(reservation, session).unwrap();

        let error =
            crate::ai::commands::ai_audit_handle(&state, "tab", &owner, Some("stale-instance"))
                .unwrap_err();
        assert_eq!(error.code(), "ai_session_not_found");
        crate::ai::commands::ai_audit_handle(&state, "tab", &owner, Some(&instance_id)).unwrap();
    }

    #[test]
    fn ai_runtime_owner_fence_isolates_actions_reads_lists_and_rebinds() {
        let state = empty_state();
        let owner = SessionOwner::Window("main".into());
        let other = SessionOwner::Headless(uuid::Uuid::new_v4());
        let reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (session, mut actions) = fake_ai_session("tab", "conversation");
        activate_fake_ai_session(reservation, session).unwrap();

        let action_error =
            crate::ai::commands::ai_action_sender(&state, "tab", &other, None).unwrap_err();
        assert_eq!(action_error.code(), "ai_session_not_found");
        assert!(matches!(
            actions.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        let audit_error =
            crate::ai::commands::ai_audit_handle(&state, "tab", &other, None).unwrap_err();
        assert_eq!(audit_error.code(), "ai_session_not_found");
        assert!(
            crate::ai::commands::ai_list_sessions_for_owner(&state, &other)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            crate::ai::commands::ai_list_sessions_for_owner(&state, &owner)
                .unwrap()
                .len(),
            1
        );

        // The owner fence runs before target lookup, so another owner cannot
        // rebind the actor even when it invents a target id.
        let rebind_error = crate::ai::commands::ai_session_rebind_target_impl(
            &state,
            &other,
            "tab".into(),
            crate::ai::commands::AiTarget::Telnet("invented-target".into()),
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(rebind_error.code(), "ai_session_not_found");

        state.lifecycle_sessions.lock().unwrap().insert(
            "foreign-target".into(),
            SessionRecord {
                nonce: uuid::Uuid::new_v4(),
                kind: SessionKind::Telnet,
                owner: other.clone(),
                phase: SessionPhase::Ready,
                parent: None,
            },
        );
        let target_error = crate::ai::commands::ai_session_rebind_target_impl(
            &state,
            &owner,
            "tab".into(),
            crate::ai::commands::AiTarget::Telnet("foreign-target".into()),
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(target_error.code(), "session_owner_mismatch");

        // instanceId remains optional for old clients, but only for the owner.
        crate::ai::commands::ai_send_action(
            &state,
            "tab",
            &owner,
            None,
            crate::ai::session::UserAction::ClearContext { ack: None },
        )
        .unwrap();
        assert!(matches!(
            actions.try_recv(),
            Ok(crate::ai::session::UserAction::ClearContext { ack: None })
        ));
        crate::ai::commands::ai_audit_handle(&state, "tab", &owner, None).unwrap();
    }

    #[tokio::test]
    async fn stop_persists_a_message_accepted_before_the_actor_runs() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, _events) = command_proposal_session(&state, "tab");
        let conversation_id = pending.info().conversation_id.clone();
        reservation.claim_conversation(&conversation_id).unwrap();

        pending
            .info()
            .action_tx
            .send(crate::ai::session::UserAction::Message {
                text: "first".into(),
                ack: None,
            })
            .expect("the command accepted the message");
        pending.info().shutdown_tx.send_replace(true);
        reservation.activate(pending).unwrap();

        close_ai_session(&state, "tab", &owner, None).await.unwrap();

        let row = crate::db::ai_conversation::get(&state.db, &conversation_id)
            .unwrap()
            .unwrap();
        let history: Vec<crate::ai::llm::ChatMessage> =
            serde_json::from_str(&row.history_json).unwrap();
        assert!(matches!(
            history.as_slice(),
            [
                crate::ai::llm::ChatMessage::User { content },
                crate::ai::llm::ChatMessage::Assistant { content: marker, .. },
            ] if content == "first" && marker == "[session closed by user]"
        ));
    }

    #[tokio::test]
    async fn stop_drains_multiple_accepted_messages_in_protocol_order() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, _events) = command_proposal_session(&state, "tab");
        let conversation_id = pending.info().conversation_id.clone();
        reservation.claim_conversation(&conversation_id).unwrap();

        for text in ["first", "second"] {
            pending
                .info()
                .action_tx
                .send(crate::ai::session::UserAction::Message {
                    text: text.into(),
                    ack: None,
                })
                .expect("the command accepted the message");
        }
        pending.info().shutdown_tx.send_replace(true);
        reservation.activate(pending).unwrap();

        close_ai_session(&state, "tab", &owner, None).await.unwrap();

        let row = crate::db::ai_conversation::get(&state.db, &conversation_id)
            .unwrap()
            .unwrap();
        let history: Vec<crate::ai::llm::ChatMessage> =
            serde_json::from_str(&row.history_json).unwrap();
        assert!(matches!(
            history.as_slice(),
            [
                crate::ai::llm::ChatMessage::User { content: first },
                crate::ai::llm::ChatMessage::Assistant { content: marker_a, .. },
                crate::ai::llm::ChatMessage::User { content: second },
                crate::ai::llm::ChatMessage::Assistant { content: marker_b, .. },
            ] if first == "first"
                && second == "second"
                && marker_a == "[session closed by user]"
                && marker_b == "[session closed by user]"
        ));
    }

    #[tokio::test]
    async fn shutdown_applies_queued_clear_context_and_acks_after_persist() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, _events) = command_proposal_session(&state, "tab");
        let conversation_id = pending.info().conversation_id.clone();
        reservation.claim_conversation(&conversation_id).unwrap();
        let (message_ack_tx, message_ack_rx) = tokio::sync::oneshot::channel();
        let (clear_ack_tx, clear_ack_rx) = tokio::sync::oneshot::channel();

        pending
            .info()
            .action_tx
            .send(crate::ai::session::UserAction::Message {
                text: "discard me".into(),
                ack: Some(message_ack_tx),
            })
            .unwrap();
        pending
            .info()
            .action_tx
            .send(crate::ai::session::UserAction::ClearContext {
                ack: Some(clear_ack_tx),
            })
            .unwrap();
        pending.info().shutdown_tx.send_replace(true);
        reservation.activate(pending).unwrap();

        message_ack_rx.await.unwrap().unwrap();
        clear_ack_rx.await.unwrap().unwrap();
        let row = crate::db::ai_conversation::get(&state.db, &conversation_id)
            .unwrap()
            .unwrap();
        let history: Vec<crate::ai::llm::ChatMessage> =
            serde_json::from_str(&row.history_json).unwrap();
        assert!(history.is_empty());

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
    }

    #[tokio::test]
    async fn user_message_command_returns_only_after_history_is_persisted() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, _events) = command_proposal_session(&state, "tab");
        let conversation_id = pending.info().conversation_id.clone();
        reservation.claim_conversation(&conversation_id).unwrap();
        reservation.activate(pending).unwrap();

        crate::ai::commands::ai_user_message_impl(&state, "tab", &owner, "diagnose".into(), None)
            .await
            .unwrap();

        let row = crate::db::ai_conversation::get(&state.db, &conversation_id)
            .unwrap()
            .unwrap();
        let history: Vec<crate::ai::llm::ChatMessage> =
            serde_json::from_str(&row.history_json).unwrap();
        assert!(matches!(
            history.first(),
            Some(crate::ai::llm::ChatMessage::User { content }) if content == "diagnose"
        ));

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
    }

    #[tokio::test]
    async fn processed_actions_report_rejection_while_a_tool_is_pending() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, mut events) = command_proposal_session(&state, "tab");
        reservation
            .claim_conversation(&pending.info().conversation_id)
            .unwrap();
        reservation.activate(pending).unwrap();
        crate::ai::commands::ai_user_message_impl(&state, "tab", &owner, "diagnose".into(), None)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some((event, _)) = events.recv().await {
                if event == "ai:command_proposed:tab" {
                    return;
                }
            }
            panic!("actor exited before proposing the command");
        })
        .await
        .expect("actor never reached the approval wait");

        let message_error = crate::ai::commands::ai_user_message_impl(
            &state,
            "tab",
            &owner,
            "too soon".into(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(message_error.code(), "ai_action_rejected_pending_tool");
        let clear_error =
            crate::ai::commands::ai_session_clear_context_impl(&state, "tab", &owner, None)
                .await
                .unwrap_err();
        assert_eq!(clear_error.code(), "ai_action_rejected_pending_tool");

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
    }

    #[tokio::test]
    async fn prepare_stop_commits_queued_command_result_before_releasing_lease() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, mut events) = command_proposal_session(&state, "tab");
        let conversation_id = pending.info().conversation_id.clone();
        reservation.claim_conversation(&conversation_id).unwrap();
        reservation.activate(pending).unwrap();
        crate::ai::commands::ai_user_message_impl(&state, "tab", &owner, "diagnose".into(), None)
            .await
            .unwrap();
        let command_id = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some((event, payload)) = events.recv().await {
                if event == "ai:command_proposed:tab" {
                    assert_eq!(payload["tool_call_id"], payload["id"]);
                    return payload["id"].as_str().unwrap().to_owned();
                }
            }
            panic!("actor exited before proposing the command");
        })
        .await
        .expect("actor never reached the approval wait");

        crate::ai::commands::ai_send_action(
            &state,
            "tab",
            &owner,
            None,
            crate::ai::session::UserAction::CommandResult {
                command_id,
                exit_code: 0,
                output: "observed output".into(),
                timed_out: false,
                early_terminated: false,
                ack: None,
            },
        )
        .unwrap();
        let terminal = prepare_ai_session_stop(&state, "tab", &owner, None)
            .await
            .unwrap();
        assert!(terminal.iter().any(|mutation| {
            mutation.kind == "command_completed" && mutation.payload["output"] == "observed output"
        }));

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
        let error = crate::ai::commands::ai_send_action(
            &state,
            "tab",
            &owner,
            None,
            crate::ai::session::UserAction::Message {
                text: "too late".into(),
                ack: None,
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), "ai_session_stopped");
        let error = {
            let sessions = state.ai_sessions.lock().unwrap();
            crate::ai::commands::ensure_ai_running(sessions.get("tab").unwrap()).unwrap_err()
        };
        assert_eq!(error.code(), "ai_session_stopped");
        let mut competing = reserve_ai_owner(&state, "other-tab".into(), owner.clone()).unwrap();
        let error = competing.claim_conversation(&conversation_id).unwrap_err();
        assert_eq!(error.code(), "conversation_in_use");
        drop(competing);

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
        assert!(!state.ai_session_owners.lock().unwrap().contains_key("tab"));
        let row = crate::db::ai_conversation::get(&state.db, &conversation_id)
            .unwrap()
            .unwrap();
        let history: Vec<crate::ai::llm::ChatMessage> =
            serde_json::from_str(&row.history_json).unwrap();
        assert!(history.iter().any(|message| matches!(
            message,
            crate::ai::llm::ChatMessage::ToolResult {
                tool_call_id,
                content,
                ..
            } if tool_call_id == "tool-call" && content.contains("observed output")
        )));
    }

    #[tokio::test]
    async fn command_result_ack_follows_terminal_mutation_processing() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, mut events) = command_proposal_session(&state, "tab");
        let conversation_id = pending.info().conversation_id.clone();
        reservation.claim_conversation(&conversation_id).unwrap();
        reservation.activate(pending).unwrap();
        crate::ai::commands::ai_user_message_impl(&state, "tab", &owner, "diagnose".into(), None)
            .await
            .unwrap();
        let command_id = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some((event, payload)) = events.recv().await {
                if event == "ai:command_proposed:tab" {
                    assert_eq!(payload["tool_call_id"], payload["id"]);
                    return payload["id"].as_str().unwrap().to_owned();
                }
            }
            panic!("actor exited before proposing the command");
        })
        .await
        .expect("actor never reached the approval wait");

        crate::ai::commands::ai_command_result_impl(
            &state,
            "tab",
            &owner,
            command_id,
            0,
            "processed output".into(),
            false,
            false,
            None,
        )
        .await
        .unwrap();

        let terminal = state
            .ai_sessions
            .lock()
            .unwrap()
            .get("tab")
            .unwrap()
            .terminal_mutations
            .lock()
            .unwrap()
            .clone();
        assert!(terminal.iter().any(|mutation| {
            mutation.kind == "command_completed" && mutation.payload["output"] == "processed output"
        }));

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
    }

    #[tokio::test]
    async fn file_op_result_ack_precedes_prepare_without_losing_terminal_mutation() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, mut events) = file_op_session(&state, "tab");
        let conversation_id = pending.info().conversation_id.clone();
        reservation.claim_conversation(&conversation_id).unwrap();
        reservation.activate(pending).unwrap();
        crate::ai::commands::ai_user_message_impl(&state, "tab", &owner, "inspect".into(), None)
            .await
            .unwrap();

        let probe_id = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let (event, payload) = events.recv().await.expect("actor event channel closed");
                if event == "ai:internal_command:tab" {
                    assert_eq!(payload["tool_call_id"], payload["id"]);
                    return payload["id"].as_str().unwrap().to_owned();
                }
            }
        })
        .await
        .expect("actor never proposed the capability probe");
        crate::ai::commands::ai_command_result_impl(
            &state,
            "tab",
            &owner,
            probe_id,
            0,
            "py3=1 perl=0 diff=1".into(),
            false,
            false,
            None,
        )
        .await
        .unwrap();

        let card_id = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let (event, payload) = events.recv().await.expect("actor event channel closed");
                if event == "ai:command_proposed:tab" {
                    return payload["id"].as_str().unwrap().to_owned();
                }
            }
        })
        .await
        .expect("actor never proposed the file operation");
        let stale_error = crate::ai::commands::ai_command_result_impl(
            &state,
            "tab",
            &owner,
            "previous-step-card".into(),
            0,
            "stale output".into(),
            false,
            false,
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(stale_error.code(), "ai_tool_call_not_pending");
        crate::ai::commands::ai_command_result_impl(
            &state,
            "tab",
            &owner,
            card_id.clone(),
            0,
            "invalid marker payload".into(),
            false,
            false,
            None,
        )
        .await
        .unwrap();

        // This is the old cancellation window: close immediately after the
        // invoke ack. The ack now proves run_file_op already recorded its
        // command_completed terminal mutation, so prepare cannot erase it.
        let terminal = prepare_ai_session_stop(&state, "tab", &owner, None)
            .await
            .unwrap();
        assert!(terminal.iter().any(|mutation| {
            mutation.kind == "command_completed" && mutation.payload["id"] == card_id
        }));

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
    }

    #[tokio::test]
    async fn prepare_stop_returns_cancelled_assistant_terminal_snapshot() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, mut events) = streaming_session(&state, "tab");
        let conversation_id = pending.info().conversation_id.clone();
        reservation.claim_conversation(&conversation_id).unwrap();
        reservation.activate(pending).unwrap();
        crate::ai::commands::ai_user_message_impl(&state, "tab", &owner, "diagnose".into(), None)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some((event, _)) = events.recv().await {
                if event == "ai:assistant_delta:tab" {
                    return;
                }
            }
            panic!("actor exited before streaming a response");
        })
        .await
        .expect("actor never started streaming");

        let terminal = prepare_ai_session_stop(&state, "tab", &owner, None)
            .await
            .unwrap();
        assert!(terminal.iter().any(|mutation| {
            mutation.kind == "assistant_message_end"
                && mutation.payload["text"] == "partial response"
                && mutation.payload["cancelled"] == true
        }));
        assert!(state.ai_sessions.lock().unwrap().contains_key("tab"));
        assert!(state
            .ai_session_owners
            .lock()
            .unwrap()
            .get("tab")
            .is_some_and(|record| record.conversation_id.as_deref() == Some(&conversation_id)));

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
    }

    #[tokio::test]
    async fn clear_context_starts_a_new_terminal_snapshot_epoch() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, mut events) = streaming_session(&state, "tab");
        let conversation_id = pending.info().conversation_id.clone();
        reservation.claim_conversation(&conversation_id).unwrap();
        reservation.activate(pending).unwrap();
        crate::ai::commands::ai_user_message_impl(
            &state,
            "tab",
            &owner,
            "first epoch".into(),
            None,
        )
        .await
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some((event, payload)) = events.recv().await {
                if event == "ai:assistant_delta:tab" {
                    assert_eq!(payload["context_epoch"], 0);
                    return;
                }
            }
            panic!("actor exited before streaming a response");
        })
        .await
        .expect("actor never started streaming");

        let cancel = crate::ai::commands::ai_cancel_slot(&state, "tab", &owner, None)
            .unwrap()
            .lock()
            .unwrap()
            .as_ref()
            .cloned()
            .expect("stream cancel slot was not installed");
        cancel.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some((event, payload)) = events.recv().await {
                if event == "ai:assistant_message_end:tab" {
                    assert_eq!(payload["context_epoch"], 0);
                    return;
                }
            }
            panic!("actor exited before ending the cancelled response");
        })
        .await
        .expect("cancelled response never emitted its terminal event");

        crate::ai::commands::ai_session_clear_context_impl(&state, "tab", &owner, None)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some((event, payload)) = events.recv().await {
                if event == "ai:context_cleared:tab" {
                    assert_eq!(payload["context_epoch"], 1);
                    return;
                }
            }
            panic!("actor exited before emitting the clear epoch");
        })
        .await
        .expect("context clear event was not emitted");
        let terminal = prepare_ai_session_stop(&state, "tab", &owner, None)
            .await
            .unwrap();
        assert!(terminal.is_empty());

        close_ai_session(&state, "tab", &owner, None).await.unwrap();
    }

    #[tokio::test]
    async fn close_ai_session_exits_while_command_is_waiting_for_approval() {
        let state = Arc::new(empty_state());
        let owner = SessionOwner::Window("main".into());
        let mut reservation = reserve_ai_owner(&state, "tab".into(), owner.clone()).unwrap();
        let (pending, mut events) = command_proposal_session(&state, "tab");
        reservation
            .claim_conversation(&pending.info().conversation_id)
            .unwrap();
        reservation.activate(pending).unwrap();
        state
            .ai_sessions
            .lock()
            .unwrap()
            .get("tab")
            .unwrap()
            .action_tx
            .send(crate::ai::session::UserAction::Message {
                text: "diagnose".into(),
                ack: None,
            })
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some((event, _)) = events.recv().await {
                if event == "ai:command_proposed:tab" {
                    return;
                }
            }
            panic!("actor exited before proposing the command");
        })
        .await
        .expect("actor never reached the approval wait");

        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            close_ai_session(&state, "tab", &owner, None),
        )
        .await
        .expect("close hung while a tool was waiting for approval")
        .unwrap();

        assert!(state.ai_sessions.lock().unwrap().is_empty());
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
