//! Session 生命周期管理：前端 reconcile + 窗口销毁清理。

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use tauri::State;

use crate::error::{locked, AppError, AppResult};
use crate::state::AppState;
use crate::state::SessionSlot;

/// 前端启动 / 重连后调用：把不在 `active_ids` 列表里的所有 session 全部清掉。
///
/// `active_ids` 是前端当前持有的所有 ID（不区分 ssh / sftp / forward —
/// UUID 不会撞）。返回被清理的总数。
#[tauri::command]
pub fn reconcile_sessions(state: State<'_, AppState>, active_ids: Vec<String>) -> AppResult<usize> {
    reconcile_sessions_impl(&state, active_ids)
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

fn ensure_session_id_available(state: &AppState, session_id: &str) -> AppResult<()> {
    let mut occupied = locked(&state.sessions)?.contains_key(session_id);
    #[cfg(not(target_os = "android"))]
    {
        occupied |= locked(&state.pty_sessions)?.contains_key(session_id);
        occupied |= locked(&state.serial_sessions)?.contains_key(session_id);
    }
    occupied |= locked(&state.telnet_sessions)?.contains_key(session_id);
    occupied |= locked(&state.sftp_sessions)?.contains_key(session_id);
    occupied |= locked(&state.active_forwards)?.contains_key(session_id);
    if occupied {
        return Err(AppError::config(
            "session_id_conflict",
            serde_json::json!({ "id": session_id }),
        ));
    }
    Ok(())
}

pub struct SessionReservation<'a, T> {
    publication_lock: &'a Mutex<()>,
    sessions: &'a Mutex<HashMap<String, SessionSlot<T>>>,
    session_id: String,
    nonce: uuid::Uuid,
    /// Desktop commands also register the pending id under a window. Keep the
    /// ownership entry under the same RAII boundary so cancellation cannot
    /// leave either half behind.
    window_sessions: Option<&'a Mutex<HashMap<String, HashSet<String>>>>,
    armed: bool,
}

impl<T> SessionReservation<'_, T> {
    pub fn activate(mut self, handle: T) -> AppResult<()> {
        let _publication = locked(self.publication_lock)?;
        let mut sessions = locked(self.sessions)?;
        let Some(slot) = sessions.get_mut(&self.session_id) else {
            return Err(AppError::not_found(
                "session_reservation_lost",
                serde_json::json!({ "id": self.session_id }),
            ));
        };
        if !matches!(slot, SessionSlot::Pending { nonce } if *nonce == self.nonce) {
            return Err(AppError::not_found(
                "session_reservation_lost",
                serde_json::json!({ "id": self.session_id }),
            ));
        }
        *slot = SessionSlot::Ready(handle);
        self.armed = false;
        Ok(())
    }
}

impl<T> Drop for SessionReservation<'_, T> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(_publication) = self.publication_lock.lock() else {
            return;
        };
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        let mut owners = match self.window_sessions {
            Some(window_sessions) => match window_sessions.lock() {
                Ok(owners) => Some(owners),
                Err(_) => return,
            },
            None => None,
        };
        let owns_slot = matches!(
            sessions.get(&self.session_id),
            Some(SessionSlot::Pending { nonce }) if *nonce == self.nonce
        );
        if !owns_slot {
            return;
        }
        sessions.remove(&self.session_id);
        if let Some(owners) = owners.as_mut() {
            for ids in owners.values_mut() {
                ids.remove(&self.session_id);
            }
            owners.retain(|_, ids| !ids.is_empty());
        }
    }
}

fn reserve_slot<'a, T>(
    publication_lock: &'a Mutex<()>,
    sessions: &'a Mutex<HashMap<String, SessionSlot<T>>>,
    session_id: &str,
) -> AppResult<SessionReservation<'a, T>> {
    let mut slots = locked(sessions)?;
    if slots.contains_key(session_id) {
        return Err(AppError::config(
            "session_id_conflict",
            serde_json::json!({ "id": session_id }),
        ));
    }
    let nonce = uuid::Uuid::new_v4();
    slots.insert(session_id.to_owned(), SessionSlot::Pending { nonce });
    drop(slots);
    Ok(SessionReservation {
        publication_lock,
        sessions,
        session_id: session_id.to_owned(),
        nonce,
        window_sessions: None,
        armed: true,
    })
}

pub fn reserve_session<'a, T>(
    state: &'a AppState,
    sessions: &'a Mutex<HashMap<String, SessionSlot<T>>>,
    session_id: &str,
) -> AppResult<SessionReservation<'a, T>> {
    let _global = locked(&state.session_id_reservation_lock)?;
    ensure_session_id_available(state, session_id)?;
    reserve_slot(&state.session_id_reservation_lock, sessions, session_id)
}

pub fn reserve_window_session<'a, T>(
    state: &'a AppState,
    sessions: &'a Mutex<HashMap<String, SessionSlot<T>>>,
    window_label: &str,
    session_id: &str,
) -> AppResult<SessionReservation<'a, T>> {
    let _global = locked(&state.session_id_reservation_lock)?;
    ensure_session_id_available(state, session_id)?;
    let mut reservation = reserve_slot(&state.session_id_reservation_lock, sessions, session_id)?;
    let mut owners = match locked(&state.window_sessions) {
        Ok(owners) => owners,
        Err(error) => {
            // Reservation cleanup takes the publication lock too. Release the
            // lock held by this constructor before dropping the armed guard.
            drop(_global);
            drop(reservation);
            return Err(error);
        }
    };
    owners
        .entry(window_label.to_owned())
        .or_default()
        .insert(session_id.to_owned());
    drop(owners);
    reservation.window_sessions = Some(&state.window_sessions);
    Ok(reservation)
}

/// Publish a fully-open resource that generates its own UUID. Holding the same
/// global lock as Pending reservation makes the bare id a process-wide key
/// across every UUID-keyed connection map consumed by `reconcile_sessions`.
pub fn publish_session<T>(
    state: &AppState,
    sessions: &Mutex<HashMap<String, T>>,
    session_id: String,
    handle: T,
) -> AppResult<()> {
    let _global = locked(&state.session_id_reservation_lock)?;
    ensure_session_id_available(state, &session_id)?;
    let old = locked(sessions)?.insert(session_id, handle);
    debug_assert!(old.is_none());
    Ok(())
}

/// Atomically publish a ready handle and register its owning window.
pub fn publish_window_session<T>(
    state: &AppState,
    sessions: &Mutex<HashMap<String, T>>,
    window_label: &str,
    session_id: String,
    handle: T,
) -> AppResult<()> {
    let _global = locked(&state.session_id_reservation_lock)?;
    ensure_session_id_available(state, &session_id)?;
    let mut sessions = locked(sessions)?;
    let mut owners = locked(&state.window_sessions)?;
    let old = sessions.insert(session_id.clone(), handle);
    debug_assert!(old.is_none());
    owners
        .entry(window_label.to_owned())
        .or_default()
        .insert(session_id);
    Ok(())
}

/// Atomically remove a window-owned slot and its secondary owner entry.
pub fn take_window_session<T>(
    state: &AppState,
    sessions: &Mutex<HashMap<String, T>>,
    session_id: &str,
) -> AppResult<Option<T>> {
    let _global = locked(&state.session_id_reservation_lock)?;
    let mut sessions = locked(sessions)?;
    let mut owners = locked(&state.window_sessions)?;
    let removed = sessions.remove(session_id);
    for ids in owners.values_mut() {
        ids.remove(session_id);
    }
    owners.retain(|_, ids| !ids.is_empty());
    Ok(removed)
}

/// Remove a non-window-owned slot under the publication lock.
pub fn take_session<T>(
    state: &AppState,
    sessions: &Mutex<HashMap<String, T>>,
    session_id: &str,
) -> AppResult<Option<T>> {
    let _global = locked(&state.session_id_reservation_lock)?;
    Ok(locked(sessions)?.remove(session_id))
}

/// Retain a subset of a session map under the same publication lock used by
/// insert/take. This is used for dependent-resource cleanup such as SFTP
/// children when their parent SSH session closes.
pub fn retain_sessions<T>(
    state: &AppState,
    sessions: &Mutex<HashMap<String, T>>,
    mut keep: impl FnMut(&String, &mut T) -> bool,
) -> AppResult<()> {
    let _global = locked(&state.session_id_reservation_lock)?;
    locked(sessions)?.retain(|id, handle| keep(id, handle));
    Ok(())
}

/// Transport-agnostic body shared by the Tauri command and the headless server.
/// Headless needs this too: the server process outlives a browser/JCEF reload, so
/// the reloaded page's mount calls this to reap orphan sessions from before the
/// reload (events from those would otherwise fire into a dead socket).
pub fn reconcile_sessions_impl(state: &AppState, active_ids: Vec<String>) -> AppResult<usize> {
    // Session publication, Pending -> Ready activation, reconciliation, and
    // window destruction are one lifecycle transaction. Keeping a single lock
    // order (publication -> primary maps -> window index) prevents an id from
    // changing meaning while this pass walks the transport maps.
    let _publication = locked(&state.session_id_reservation_lock)?;
    let alive: HashSet<String> = active_ids.into_iter().collect();
    let mut closed = 0;

    // SSH sessions —— 收集所有要被关掉的 ssh id，先做 SFTP children 联动清理，
    // 再切断 TCP。
    let mut stale_ssh: Vec<String> = Vec::new();
    {
        let sessions = locked(&state.sessions)?;
        for k in sessions.keys() {
            if !alive.contains(k) {
                stale_ssh.push(k.clone());
            }
        }
    }

    // SFTP sessions：本身不在 alive 里的清掉；父 SSH 也要被清的 children 也清掉。
    {
        let mut sftp = locked(&state.sftp_sessions)?;
        let before = sftp.len();
        sftp.retain(|k, h| {
            alive.contains(k)
                && match h.parent_ssh_id() {
                    Some(parent) => !stale_ssh.iter().any(|s| s == parent),
                    None => true,
                }
        });
        closed += before - sftp.len();
    }

    // 现在再切 SSH 的 TCP（避免 children 还在的时候 disconnect 得到无意义的传输报错）
    {
        let mut sessions = locked(&state.sessions)?;
        for k in &stale_ssh {
            if let Some(h) = sessions.remove(k) {
                h.force_disconnect();
                closed += 1;
            }
        }
    }

    // Active forwards
    {
        let mut fwds = locked(&state.active_forwards)?;
        let stale: Vec<String> = fwds
            .keys()
            .filter(|k| !alive.contains(*k))
            .cloned()
            .collect();
        for k in stale {
            if let Some(h) = fwds.remove(&k) {
                h.stop();
                closed += 1;
            }
        }
    }

    // PTY（桌面平台）
    #[cfg(not(target_os = "android"))]
    {
        let mut pty = locked(&state.pty_sessions)?;
        let before = pty.len();
        pty.retain(|k, _| alive.contains(k));
        closed += before - pty.len();
    }

    // Serial（桌面平台）—— 同 PTY：不在 alive 里的移除；drop 最后一份 handle
    // 触发 reader 线程退出（CloseGuard 置位 close flag）。
    #[cfg(not(target_os = "android"))]
    {
        let mut serial = locked(&state.serial_sessions)?;
        let before = serial.len();
        serial.retain(|k, _| alive.contains(k));
        closed += before - serial.len();
    }

    // Telnet —— 同 serial：drop 最后一份 handle 触发 reader 线程退出。
    {
        let mut telnet = locked(&state.telnet_sessions)?;
        let before = telnet.len();
        telnet.retain(|k, _| alive.contains(k));
        closed += before - telnet.len();
    }

    // AI 排障会话：key 也是 tab_id（与其他 session 同处 alive 集合）。不在 alive 里的
    // 先发 Stop 让 actor 退出（同 ai_session_stop），再移除——否则重载后 actor 带着
    // 死事件 sink 残留到进程退出。
    {
        let mut ai = locked(&state.ai_sessions)?;
        let stale: Vec<String> = ai.keys().filter(|k| !alive.contains(*k)).cloned().collect();
        for k in stale {
            if let Some(s) = ai.remove(&k) {
                let _ = s.action_tx.send(crate::ai::session::UserAction::Stop);
                closed += 1;
            }
        }
    }

    // window_sessions is a secondary ownership index over the four primary
    // transport maps. Reconcile used to remove a transport but leave its old
    // owner entry behind; if that UUID was later reused, closing the old window
    // would kill the new session. Rebuild the valid key set after all removals
    // and prune the secondary index in the same reconciliation pass. Hold the
    // publication lock across the snapshot and prune so a concurrent Pending
    // reservation cannot publish between those two operations and lose its
    // freshly-created owner entry.
    let mut live_transport_ids = HashSet::new();
    live_transport_ids.extend(locked(&state.sessions)?.keys().cloned());
    #[cfg(not(target_os = "android"))]
    {
        live_transport_ids.extend(locked(&state.pty_sessions)?.keys().cloned());
        live_transport_ids.extend(locked(&state.serial_sessions)?.keys().cloned());
    }
    live_transport_ids.extend(locked(&state.telnet_sessions)?.keys().cloned());
    let mut owners = locked(&state.window_sessions)?;
    for ids in owners.values_mut() {
        ids.retain(|id| live_transport_ids.contains(id));
    }
    owners.retain(|_, ids| !ids.is_empty());

    Ok(closed)
}

/// 关闭指定窗口拥有的所有 session —— 窗口销毁时调用。
pub fn close_window_sessions(state: &AppState, window_label: &str) {
    // Serialize the whole multi-map delete with publication and activation.
    // Otherwise an id removed from an early map could be reused in a later map
    // and then accidentally deleted by this old window's cleanup pass.
    let _publication = match state.session_id_reservation_lock.lock() {
        Ok(lock) => lock,
        Err(_) => return,
    };
    let ids = match state.window_sessions.lock() {
        Ok(mut ws) => ws.remove(window_label).unwrap_or_default(),
        Err(_) => return,
    };
    if ids.is_empty() {
        return;
    }

    // 先把所有挂在这些 SSH 上的 SFTP children 清掉（基于 parent_ssh_id 反查），
    // 再切 TCP。这样传输的 channel I/O 会被底层 socket 关掉自然 error 退出。
    if let Ok(mut sftp) = state.sftp_sessions.lock() {
        sftp.retain(|sftp_id, h| {
            // ids 里的 SFTP（本身被记录在窗口下的）和 parent_ssh_id 在 ids 里的 children 都清
            !ids.contains(sftp_id)
                && match h.parent_ssh_id() {
                    Some(parent) => !ids.contains(parent),
                    None => true,
                }
        });
    }

    if let Ok(mut sessions) = state.sessions.lock() {
        for id in &ids {
            if let Some(h) = sessions.remove(id) {
                h.force_disconnect();
            }
        }
    }
    #[cfg(not(target_os = "android"))]
    if let Ok(mut pty) = state.pty_sessions.lock() {
        for id in &ids {
            pty.remove(id);
        }
    }
    #[cfg(not(target_os = "android"))]
    if let Ok(mut serial) = state.serial_sessions.lock() {
        for id in &ids {
            serial.remove(id);
        }
    }
    if let Ok(mut telnet) = state.telnet_sessions.lock() {
        for id in &ids {
            telnet.remove(id);
        }
    }
    if let Ok(mut fwds) = state.active_forwards.lock() {
        for id in &ids {
            if let Some(h) = fwds.remove(id) {
                h.stop();
            }
        }
    }
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
            session_id_reservation_lock: Mutex::new(()),
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
            window_sessions: Mutex::new(HashMap::new()),
            #[cfg(desktop)]
            window_groups: Mutex::new(crate::commands::window::WindowGroups::default()),
            ai_sessions: Mutex::new(HashMap::new()),
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
    fn reservation_is_atomic_and_only_pending_slot_can_activate() {
        let publication_lock = Mutex::new(());
        let sessions = Mutex::new(HashMap::<String, SessionSlot<u8>>::new());
        let reservation = reserve_slot(&publication_lock, &sessions, "id").unwrap();
        let duplicate = match reserve_slot(&publication_lock, &sessions, "id") {
            Ok(_) => panic!("duplicate reservation unexpectedly succeeded"),
            Err(e) => e,
        };
        assert_eq!(duplicate.code(), "session_id_conflict");

        reservation.activate(7).unwrap();
        assert_eq!(
            sessions
                .lock()
                .unwrap()
                .get("id")
                .and_then(SessionSlot::ready),
            Some(&7)
        );
        assert!(sessions.lock().unwrap().contains_key("id"));
    }

    #[test]
    fn removed_reservation_cannot_publish_a_late_handle() {
        let publication_lock = Mutex::new(());
        let sessions = Mutex::new(HashMap::<String, SessionSlot<u8>>::new());
        let reservation = reserve_slot(&publication_lock, &sessions, "id").unwrap();
        sessions.lock().unwrap().remove("id");
        let err = reservation.activate(7).unwrap_err();
        assert_eq!(err.code(), "session_reservation_lost");
    }

    #[test]
    fn dropping_reservation_cleans_pending_slot_and_window_owner() {
        let publication_lock = Mutex::new(());
        let sessions = Mutex::new(HashMap::<String, SessionSlot<u8>>::new());
        let owners = Mutex::new(HashMap::from([(
            "main".to_string(),
            HashSet::from(["id".to_string()]),
        )]));
        {
            let mut reservation = reserve_slot(&publication_lock, &sessions, "id").unwrap();
            reservation.window_sessions = Some(&owners);
        }

        assert!(!sessions.lock().unwrap().contains_key("id"));
        assert!(!owners.lock().unwrap().contains_key("main"));
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn same_id_cannot_be_reserved_in_two_transport_maps() {
        let state = empty_state();
        let _pty = reserve_session(&state, &state.pty_sessions, "id").unwrap();

        let err = match reserve_session(&state, &state.telnet_sessions, "id") {
            Ok(_) => panic!("cross-transport duplicate unexpectedly reserved"),
            Err(e) => e,
        };
        assert_eq!(err.code(), "session_id_conflict");

        let unrelated_ready_map = Mutex::new(HashMap::<String, u8>::new());
        assert_eq!(
            publish_session(&state, &unrelated_ready_map, "id".into(), 7)
                .unwrap_err()
                .code(),
            "session_id_conflict"
        );
    }

    #[test]
    fn reconcile_prunes_stale_window_owners_before_id_reuse() {
        let state = empty_state();
        let _live = reserve_window_session(&state, &state.telnet_sessions, "main", "live").unwrap();
        state.window_sessions.lock().unwrap().extend([
            (
                "main".into(),
                HashSet::from(["live".into(), "stale".into()]),
            ),
            ("old".into(), HashSet::from(["stale".into()])),
        ]);

        reconcile_sessions_impl(&state, vec!["live".into()]).unwrap();

        let owners = state.window_sessions.lock().unwrap();
        assert_eq!(owners["main"], HashSet::from(["live".into()]));
        assert!(!owners.contains_key("old"));
    }

    #[test]
    fn publish_and_take_window_session_update_handle_and_owner_atomically() {
        let state = empty_state();
        let sessions = Mutex::new(HashMap::<String, u8>::new());

        publish_window_session(&state, &sessions, "main", "id".into(), 7).unwrap();

        assert_eq!(sessions.lock().unwrap().get("id"), Some(&7));
        assert_eq!(
            state.window_sessions.lock().unwrap()["main"],
            HashSet::from(["id".into()])
        );

        assert_eq!(
            take_window_session(&state, &sessions, "id").unwrap(),
            Some(7)
        );
        assert!(!sessions.lock().unwrap().contains_key("id"));
        assert!(!state.window_sessions.lock().unwrap().contains_key("main"));
    }

    #[test]
    fn old_reservation_cannot_activate_or_drop_a_reused_id() {
        let state = empty_state();
        let sessions = Mutex::new(HashMap::<String, SessionSlot<u8>>::new());
        let old = reserve_window_session(&state, &sessions, "old", "id").unwrap();

        assert!(take_window_session(&state, &sessions, "id")
            .unwrap()
            .is_some());
        let new = reserve_window_session(&state, &sessions, "new", "id").unwrap();

        assert_eq!(
            old.activate(7).unwrap_err().code(),
            "session_reservation_lost"
        );
        assert!(sessions.lock().unwrap().contains_key("id"));
        assert!(state.window_sessions.lock().unwrap()["new"].contains("id"));

        new.activate(8).unwrap();
        assert_eq!(
            sessions
                .lock()
                .unwrap()
                .get("id")
                .and_then(SessionSlot::ready),
            Some(&8)
        );
    }

    #[test]
    fn reconcile_removes_unreported_pending_without_late_drop_touching_reuse() {
        let state = empty_state();
        let old = reserve_window_session(&state, &state.telnet_sessions, "old", "id").unwrap();

        reconcile_sessions_impl(&state, Vec::new()).unwrap();

        assert!(!state.telnet_sessions.lock().unwrap().contains_key("id"));
        assert!(!state.window_sessions.lock().unwrap().contains_key("old"));

        let new = reserve_window_session(&state, &state.telnet_sessions, "new", "id").unwrap();
        drop(old);

        assert!(state.telnet_sessions.lock().unwrap().contains_key("id"));
        assert!(state.window_sessions.lock().unwrap()["new"].contains("id"));
        drop(new);
    }
}
