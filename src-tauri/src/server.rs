//! Headless WebSocket server — a second adapter over the SAME rssh engine the
//! Tauri app uses (INV-4); only the transport differs. It lets the unchanged
//! web frontend run outside Tauri (a browser, or IntelliJ's JCEF) by speaking
//! the wire protocol the IPC shim expects. Gated behind the `server` feature.
//!
//! Protocol (JSON over one WS):
//!   →  { type:"invoke",   id, cmd, args }
//!   ←  { type:"response",  id, ok, result|error }
//!   ←  { type:"event",     event, payload }
//!
//! Each dispatch arm calls the SAME `crate::db::*` / engine function the
//! matching `#[tauri::command]` calls, so there is no behaviour divergence.
//! Wired so far: local PTY + the DB-backed CRUD that populates the GUI.
//! SSH / SFTP / AI follow the identical pattern.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http;
use tokio_tungstenite::tungstenite::Message;

use crate::ai::session::UserAction;
use crate::error::{locked, AppError, AppResult};
use crate::models::{Credential, Forward, Group, HighlightRule, Profile, Snippet};
use crate::state::AppState;
use crate::terminal::pty::{self, PtyOut, PtySink};

type ConnResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// The built frontend, embedded at compile time so the server is self-contained
/// (the IDEA plugin just spawns the binary + points JCEF at `http://127.0.0.1:<port>/`).
/// Requires `npm run build` (→ ../dist) before `cargo build --features server`.
static UI: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/../dist");

/// Construct the engine state exactly like the Tauri app's `setup()` does:
/// open the DB at the shared data dir, open the secret store, run migrations.
/// Both adapters sit on this identical state (INV-3 shared data dir).
fn build_state() -> AppResult<AppState> {
    let data_dir = crate::db::data_dir()?;
    pty::init_available_shells();
    let db = Arc::new(crate::db::Db::open(&data_dir)?);
    let secret_system = crate::secret::open(db.clone(), &data_dir)?;
    if let Err(e) = crate::migration::run_migrations(
        &db,
        secret_system.raw_keyring.as_deref(),
        secret_system.store.as_ref(),
    ) {
        log::warn!("migration failed (will retry on next startup): {e}");
    }
    Ok(AppState {
        db,
        secret_store: secret_system.store,
        sessions: Mutex::new(HashMap::new()),
        pty_sessions: Mutex::new(HashMap::new()),
        sftp_sessions: Mutex::new(HashMap::new()),
        transfer_cancels: Mutex::new(HashMap::new()),
        active_forwards: Mutex::new(HashMap::new()),
        auth_waiters: Mutex::new(HashMap::new()),
        passphrase_waiters: Mutex::new(HashMap::new()),
        host_key_waiters: Mutex::new(HashMap::new()),
        passphrase_cache: Mutex::new(HashMap::new()),
        window_sessions: Mutex::new(HashMap::new()),
        ai_sessions: Mutex::new(HashMap::new()),
        ai_remote_shell_cache: Mutex::new(HashMap::new()),
        data_dir,
    })
}

/// Bind loopback on a random port, print `{"port":..,"token":..}` as a JSON line
/// on stdout (the launcher reads it), then serve until the process is killed.
pub async fn run() -> std::io::Result<()> {
    let state = Arc::new(
        build_state().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?,
    );
    let token = uuid::Uuid::new_v4().to_string();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    println!("{}", json!({ "port": port, "token": token }));
    {
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
    }

    loop {
        let (stream, _) = listener.accept().await?;
        let expected = token.clone();
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, expected, state).await {
                log::warn!("rssh-server: connection ended: {e}");
            }
        });
    }
}

async fn handle_conn(stream: TcpStream, expected: String, state: Arc<AppState>) -> ConnResult {
    // Sniff the request without consuming it (peek): a WebSocket upgrade takes the
    // ws path below; any other GET is served the embedded frontend over HTTP, so a
    // single port serves both the UI and the IPC socket.
    let mut sniff = [0u8; 2048];
    let n = stream.peek(&mut sniff).await?;
    let head = String::from_utf8_lossy(&sniff[..n]).to_string();
    if !head.to_ascii_lowercase().contains("upgrade: websocket") {
        return serve_static(stream, &head).await.map_err(Into::into);
    }

    // Loopback bind + per-launch token are the guard (INV-3). Browsers can't set
    // headers on a WS, so the token rides the query string: ws://..:port/?token=..
    let ws = tokio_tungstenite::accept_hdr_async(stream, move |req: &Request, resp: Response| {
        let ok = req
            .uri()
            .query()
            .and_then(query_token)
            .map(|t| t == expected)
            .unwrap_or(false);
        if ok {
            Ok(resp)
        } else {
            let err: ErrorResponse = http::Response::builder()
                .status(http::StatusCode::UNAUTHORIZED)
                .body(Some("invalid token".to_string()))
                .unwrap();
            Err(err)
        }
    })
    .await?;

    let (mut ws_tx, mut ws_rx) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    // Broadcasts "connection gone" to in-flight invoke tasks. A task blocked on a
    // frontend response (SSH auth / passphrase / host-key prompt) would otherwise
    // wait forever after the socket drops — the headless emit is a sink no-op, so
    // no reply ever comes. On close we flip this; each task selects against it,
    // gets dropped, and its RAII guard releases the waiter + SSH handle. Aborting
    // the writer alone never touches those blocked dispatch tasks.
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);

    // Single writer task: serializes responses + pushed events onto the socket.
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if ws_tx.send(m).await.is_err() {
                break;
            }
        }
    });

    while let Some(msg) = ws_rx.next().await {
        // `break`, NOT `?`: an abnormal close (TCP reset, protocol error, JCEF
        // process killed) yields `Some(Err(_))`. Returning early here would skip
        // the cleanup below, leaving SSH prompt workers parked forever. Break so
        // every exit path — clean or abnormal — runs the shutdown + waiter-clear.
        let msg = match msg {
            Ok(m) => m,
            Err(_) => break,
        };
        if !(msg.is_text() || msg.is_binary()) {
            continue;
        }
        let data = msg.into_data();
        let req: Value = match serde_json::from_slice(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if req.get("type").and_then(Value::as_str) != Some("invoke") {
            continue;
        }
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let cmd = req.get("cmd").and_then(Value::as_str).unwrap_or("").to_string();
        let args = req.get("args").cloned().unwrap_or_else(|| json!({}));

        // Each invoke runs concurrently: a slow ssh_connect must not block the
        // ssh_auth_respond on the SAME connection that unblocks its auth prompt.
        let state = state.clone();
        let tx = tx.clone();
        let mut shutdown = shutdown_tx.subscribe();
        tokio::spawn(async move {
            tokio::select! {
                // Connection closed mid-flight: drop the dispatch future so its
                // RAII guards clean up. There's no reply to send — socket is gone.
                _ = shutdown.changed() => {}
                resp = dispatch_async(&state, &cmd, args, &tx) => {
                    let resp = match resp {
                        Ok(result) => json!({ "type": "response", "id": id, "ok": true, "result": result }),
                        Err(error) => json!({ "type": "response", "id": id, "ok": false, "error": error }),
                    };
                    let _ = tx.send(Message::Text(resp.to_string().into()));
                }
            }
        });
    }

    // Connection closed: the frontend is gone, so nothing it was asked to answer
    // will ever come back. Release every holder tied to it:
    //   1. Inline-blocking dispatch futures (e.g. an SFTP transfer streaming to the
    //      now-dead sink) live inside the spawned task → the shutdown signal drops
    //      them so their CancelGuard RAII fires.
    //   2. SSH auth / passphrase / host-key prompts block on `rx.await` inside the
    //      DEDICATED SSH worker (client::spawn_ssh → spawn_local), unreachable by
    //      dropping the dispatch future. Two sub-cases:
    //      - worker reaches a prompt AFTER close: drop the writer (and its receiver)
    //        FIRST so the sink's `tx.send` fails → Host::emit returns Err → the
    //        prompt path bails before parking (its guard removes the sender).
    //      - worker already parked before close: clear the waiter maps to drop the
    //        sender → its `rx.await` errors, the worker unwinds and drops the partial
    //        SSH handle (same mechanism as ssh_disconnect).
    //   Order matters: the writer must be gone BEFORE we clear, else a worker could
    //   emit-OK, register, and park in the gap right after the clear.
    //   (One WS per headless server in practice, so clearing globally is correct.)
    let _ = shutdown_tx.send(true);
    writer.abort();
    let _ = writer.await;
    if let Ok(mut m) = state.auth_waiters.lock() { m.clear(); }
    if let Ok(mut m) = state.passphrase_waiters.lock() { m.clear(); }
    if let Ok(mut m) = state.host_key_waiters.lock() { m.clear(); }
    Ok(())
}

/// Synchronous command dispatch. Each arm mirrors the matching `#[tauri::command]`.
fn dispatch(
    state: &AppState,
    cmd: &str,
    args: Value,
    tx: &mpsc::UnboundedSender<Message>,
) -> Result<Value, Value> {
    match cmd {
        // ---- profiles ----
        "list_profiles" => ok(crate::db::profile::list(&state.db)),
        "get_profile" => ok(crate::db::profile::get(&state.db, &arg::<String>(&args, "id")?)),
        "create_profile" => ok(crate::db::profile::insert(&state.db, &arg::<Profile>(&args, "profile")?)),
        "update_profile" => ok(crate::db::profile::update(&state.db, &arg::<Profile>(&args, "profile")?)),
        "delete_profile" => ok(crate::db::profile::delete(&state.db, &arg::<String>(&args, "id")?)),
        // Parse-only (no native dialog): the frontend reads ~/.ssh/config text and
        // sends it; returns the parsed entries. import_ssh_config returns a bare Vec.
        "import_ssh_config" => ok(Ok::<_, AppError>(crate::commands::profile::import_ssh_config(
            arg::<String>(&args, "content")?,
        ))),
        // List the local shells the host offers (pure engine, no UI).
        "refresh_shells" => ok(crate::commands::pty::refresh_shells()),

        // ---- credentials (secret via SecretStore, metadata via DB) ----
        "list_credentials" => ok(crate::db::credential::list(&state.db)),
        "get_credential" => {
            let id: String = arg(&args, "id")?;
            let mut cred = crate::db::credential::get(&state.db, &id).map_err(err_value)?;
            cred.secret = state
                .secret_store
                .get(&crate::secret::cred_secret_key(&id))
                .map_err(err_value)?;
            ok::<Credential>(Ok(cred))
        }
        "create_credential" => {
            let c: Credential = arg(&args, "credential")?;
            crate::db::credential::insert(&state.db, &c).map_err(err_value)?;
            save_cred_secret(state, &c).map(|_| Value::Null)
        }
        "update_credential" => {
            let c: Credential = arg(&args, "credential")?;
            crate::db::credential::update(&state.db, &c).map_err(err_value)?;
            save_cred_secret(state, &c).map(|_| Value::Null)
        }
        "delete_credential" => {
            let id: String = arg(&args, "id")?;
            crate::db::credential::delete(&state.db, &id).map_err(err_value)?;
            state
                .secret_store
                .delete(&crate::secret::cred_secret_key(&id))
                .map_err(err_value)?;
            Ok(Value::Null)
        }

        // ---- groups ----
        "list_groups" => ok(crate::db::group::list(&state.db)),
        "create_group" => ok(crate::db::group::insert(&state.db, &arg::<Group>(&args, "group")?)),
        "update_group" => ok(crate::db::group::update(&state.db, &arg::<Group>(&args, "group")?)),
        "delete_group" => ok(crate::db::group::delete(&state.db, &arg::<String>(&args, "id")?)),

        // ---- forwards (CRUD; active start/stop need SSH — deferred) ----
        "list_forwards" => ok(crate::db::forward::list(&state.db)),
        "get_forward" => ok(crate::db::forward::get(&state.db, &arg::<String>(&args, "id")?)),
        "create_forward" => ok(crate::db::forward::insert(&state.db, &arg::<Forward>(&args, "forward")?)),
        "update_forward" => ok(crate::db::forward::update(&state.db, &arg::<Forward>(&args, "forward")?)),
        "delete_forward" => ok(crate::db::forward::delete(&state.db, &arg::<String>(&args, "id")?)),

        // ---- settings / snippets / highlights ----
        "get_setting" => {
            let key: String = arg(&args, "key")?;
            if crate::secret::is_secret_setting(&key) {
                ok(state.secret_store.get(&crate::secret::setting_key(&key)))
            } else {
                ok(crate::db::settings::get(&state.db, &key))
            }
        }
        "set_setting" => {
            let key: String = arg(&args, "key")?;
            let value: String = arg(&args, "value")?;
            let r = if crate::secret::is_secret_setting(&key) {
                if value.is_empty() {
                    state.secret_store.delete(&crate::secret::setting_key(&key))
                } else {
                    state.secret_store.set(&crate::secret::setting_key(&key), &value)
                }
            } else {
                crate::db::settings::set(&state.db, &key, &value)
            };
            ok(r)
        }
        "list_highlights" => ok(crate::db::highlight::list(&state.db)),
        "add_highlight" => ok(crate::db::highlight::insert(&state.db, &arg::<HighlightRule>(&args, "rule")?)),
        "remove_highlight" => ok(crate::db::highlight::delete_by_keyword(&state.db, &arg::<String>(&args, "keyword")?)),
        "update_highlight" => ok(crate::db::highlight::update(
            &state.db,
            &arg::<String>(&args, "oldKeyword")?,
            &arg::<HighlightRule>(&args, "rule")?,
        )),
        "reset_highlights" => ok(crate::db::highlight::reset_defaults(&state.db)),
        "load_snippets" => ok(crate::db::snippet::load(&state.data_dir)),
        "save_snippets" => ok(crate::db::snippet::save(&state.data_dir, &arg::<Vec<Snippet>>(&args, "snippets")?)),
        "secret_backend" => Ok(json!(state.secret_store.backend_name())),

        // ---- local PTY ----
        "list_shells" => Ok(json!(pty::available_shells())),
        "pty_spawn" => {
            let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(80) as u16;
            let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(24) as u16;
            let shell = crate::db::settings::get(&state.db, "local_shell")
                .ok()
                .flatten()
                .filter(|s| !s.is_empty());
            let tx = tx.clone();
            let sink: PtySink = Arc::new(move |id: &str, out: PtyOut| {
                let msg = match out {
                    PtyOut::Data(b) => {
                        json!({ "type": "event", "event": format!("pty:data:{id}"), "payload": b })
                    }
                    PtyOut::Close => {
                        json!({ "type": "event", "event": format!("pty:close:{id}"), "payload": Value::Null })
                    }
                };
                let _ = tx.send(Message::Text(msg.to_string().into()));
            });
            let (id, handle) = pty::spawn(cols, rows, sink, shell).map_err(err_value)?;
            locked(&state.pty_sessions).map_err(err_value)?.insert(id.clone(), handle);
            Ok(json!(id))
        }
        "pty_write" => {
            let sid: String = arg(&args, "sessionId")?;
            let data: Vec<u8> = arg(&args, "data")?;
            let handle = locked(&state.pty_sessions).map_err(err_value)?.get(&sid).cloned();
            match handle {
                Some(h) => h.write(&data).map(|_| Value::Null).map_err(err_value),
                None => Err(json!("pty_not_found")),
            }
        }
        "pty_resize" => {
            let sid: String = arg(&args, "sessionId")?;
            let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(80) as u16;
            let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(24) as u16;
            let handle = locked(&state.pty_sessions).map_err(err_value)?.get(&sid).cloned();
            match handle {
                Some(h) => h.resize(cols, rows).map(|_| Value::Null).map_err(err_value),
                None => Err(json!("pty_not_found")),
            }
        }
        "pty_close" => {
            let sid: String = arg(&args, "sessionId")?;
            locked(&state.pty_sessions).map_err(err_value)?.remove(&sid);
            Ok(Value::Null)
        }

        // ---- recordings (asciicast playback) ----
        "list_recordings" => ok(crate::commands::settings::list_recordings_impl(state)),
        "read_recording" => {
            ok(crate::commands::settings::read_recording_impl(state, arg(&args, "name")?))
        }

        // ---- ssh config import ----
        "read_ssh_config_default" => ok(crate::commands::profile::read_ssh_config_default()),
        "import_ssh_entries" => ok(crate::commands::profile::do_import_ssh_entries(
            &state.db,
            state.secret_store.as_ref(),
            arg(&args, "entries")?,
        )),

        // ---- config import/export (JSON-string core; the *_to_file / *_from_file
        //      dialog variants are handled browser-side by the IPC shim) ----
        "export_config" => ok(crate::commands::sync::export_config_impl(state)),
        "import_config" => {
            ok(crate::commands::sync::import_config_impl(state, arg(&args, "json")?))
        }

        // ---- port forwarding: stop + live stats (start is async, see dispatch_async) ----
        "forward_stop" => {
            let active_id: String = arg(&args, "activeId")?;
            match locked(&state.active_forwards).map_err(err_value)?.remove(&active_id) {
                Some(h) => {
                    h.stop();
                    Ok(Value::Null)
                }
                None => Err(json!("fwd_not_found")),
            }
        }
        "forward_stats" => {
            let active_id: String = arg(&args, "activeId")?;
            let forwards = locked(&state.active_forwards).map_err(err_value)?;
            let handle = forwards.get(&active_id).ok_or_else(|| json!("fwd_not_found"))?;
            ok(Ok::<_, AppError>(handle.stats()))
        }

        // ---- CLI: PATH-based status; install is host-managed in embedded mode ----
        "cli_status" => ok(Ok::<_, AppError>(crate::commands::cli::cli_status_headless())),
        "cli_install" => Err(json!("cli_install_not_applicable_embedded")),

        // ---- AI: audit save to a server-side path + remote-shell cache write ----
        "ai_audit_save" => {
            let tab_id: String = arg(&args, "tabId")?;
            let file_path: String = arg(&args, "filePath")?;
            let audit = locked(&state.ai_sessions)
                .map_err(err_value)?
                .get(&tab_id)
                .map(|s| s.audit.clone())
                .ok_or_else(|| json!("ai_session_not_found"))?;
            let g = audit.lock().map_err(|_| json!("lock_poisoned"))?;
            g.save_to_file(&std::path::PathBuf::from(file_path))
                .map_err(|e| json!(e.to_string()))?;
            Ok(Value::Null)
        }
        "ai_cache_remote_shell" => {
            let target_id: String = arg(&args, "targetId")?;
            let shell: crate::ai::shell::ShellKind = arg(&args, "shell")?;
            if let Some(profile_id) = locked(&state.sessions)
                .map_err(err_value)?
                .get(&target_id)
                .map(|h| h.profile_id().to_string())
            {
                locked(&state.ai_remote_shell_cache).map_err(err_value)?.insert(profile_id, shell);
            }
            Ok(Value::Null)
        }

        // ---- orphan-session reap on (re)mount: the server outlives a page reload,
        //      so stale ssh/sftp/forward/pty from before the reload get cleaned ----
        "reconcile_sessions" => {
            let active_ids: Vec<String> = args
                .get("activeIds")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            ok(crate::commands::lifecycle::reconcile_sessions_impl(state, active_ids))
        }

        // `getVersion()` (@tauri-apps/api/app) → plugin:app|version. Desktop has
        // the Tauri app plugin; headless answers with the crate version (synced
        // across manifests at release) so About + update checks work embedded.
        "plugin:app|version" => Ok(json!(env!("CARGO_PKG_VERSION"))),

        other => Err(json!(format!("unknown command: {other}"))),
    }
}

/// Async dispatch: the `async` commands (ssh / sftp / ai) live here; everything
/// else falls through to the sync `dispatch`. Mirrors the matching commands.
async fn dispatch_async(
    state: &Arc<AppState>,
    cmd: &str,
    args: Value,
    tx: &mpsc::UnboundedSender<Message>,
) -> Result<Value, Value> {
    match cmd {
        "ssh_connect" => ssh_connect(state, args, tx).await,
        "ssh_write" => {
            let sid: String = arg(&args, "sessionId")?;
            let data: Vec<u8> = arg(&args, "data")?;
            ssh_session(state, &sid)?.write(&data).map(|_| Value::Null).map_err(err_value)
        }
        "ssh_resize" => {
            let sid: String = arg(&args, "sessionId")?;
            let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(80) as u32;
            let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(24) as u32;
            ssh_session(state, &sid)?.resize(cols, rows).map(|_| Value::Null).map_err(err_value)
        }
        "ssh_disconnect" => {
            let sid: String = arg(&args, "sessionId")?;
            if let Some(tid) = args.get("tabId").and_then(Value::as_str) {
                let _ = locked(&state.auth_waiters).map(|mut m| m.remove(tid));
                let _ = locked(&state.passphrase_waiters).map(|mut m| m.remove(tid));
                let _ = locked(&state.host_key_waiters).map(|mut m| m.remove(tid));
            }
            {
                let mut sftp = locked(&state.sftp_sessions).map_err(err_value)?;
                sftp.retain(|_, h| h.parent_ssh_id() != Some(&sid));
            }
            match locked(&state.sessions).map_err(err_value)?.remove(&sid) {
                Some(s) => {
                    s.force_disconnect();
                    Ok(Value::Null)
                }
                None => Err(json!("session_not_found")),
            }
        }
        "ssh_auth_respond" => {
            let tab_id: String = arg(&args, "tabId")?;
            let responses: Vec<String> = arg(&args, "responses")?;
            let w = locked(&state.auth_waiters).map_err(err_value)?.remove(&tab_id);
            w.ok_or_else(|| json!("no_pending_auth"))?
                .send(responses)
                .map(|_| Value::Null)
                .map_err(|_| json!("auth_channel_closed"))
        }
        "ssh_passphrase_respond" => {
            let tab_id: String = arg(&args, "tabId")?;
            let passphrase: String = arg(&args, "passphrase")?;
            let w = locked(&state.passphrase_waiters).map_err(err_value)?.remove(&tab_id);
            w.ok_or_else(|| json!("no_pending_passphrase"))?
                .send(passphrase)
                .map(|_| Value::Null)
                .map_err(|_| json!("passphrase_channel_closed"))
        }
        "ssh_host_key_respond" => {
            let tab_id: String = arg(&args, "tabId")?;
            let answer: String = arg(&args, "answer")?;
            let w = locked(&state.host_key_waiters).map_err(err_value)?.remove(&tab_id);
            w.ok_or_else(|| json!("no_pending_hostkey"))?
                .send(answer)
                .map(|_| Value::Null)
                .map_err(|_| json!("hostkey_channel_closed"))
        }
        "ssh_auth_cancel" => waiter_cancel(&state.auth_waiters, &args),
        "ssh_passphrase_cancel" => waiter_cancel(&state.passphrase_waiters, &args),
        "ssh_host_key_cancel" => waiter_cancel(&state.host_key_waiters, &args),

        // ---- SFTP (core ops + streaming to/from a caller-supplied local path;
        //      the native pick dialogs that supply that path are host-provided) ----
        "sftp_connect" => sftp_connect(state, args).await,
        "sftp_connect_session" => sftp_connect_session(state, args).await,
        "sftp_home" => ok(sftp_handle(state, &arg::<String>(&args, "sftpId")?)?.home_dir().await),
        "sftp_list" => {
            let h = sftp_handle(state, &arg::<String>(&args, "sftpId")?)?;
            ok(h.list_dir(&arg::<String>(&args, "path")?).await)
        }
        "sftp_walk_remote_dir" => {
            let h = sftp_handle(state, &arg::<String>(&args, "sftpId")?)?;
            ok(h.walk_files(&arg::<String>(&args, "remoteRoot")?).await)
        }
        "walk_local_dir" => {
            ok(crate::commands::sftp::walk_local_dir(arg::<String>(&args, "localRoot")?).await)
        }
        "sftp_download" => {
            let h = sftp_handle(state, &arg::<String>(&args, "sftpId")?)?;
            ok(h.download(&arg::<String>(&args, "path")?).await)
        }
        "sftp_upload" => {
            let h = sftp_handle(state, &arg::<String>(&args, "sftpId")?)?;
            ok(h.upload(&arg::<String>(&args, "path")?, &arg::<Vec<u8>>(&args, "data")?).await)
        }
        "sftp_mkdir" => {
            let h = sftp_handle(state, &arg::<String>(&args, "sftpId")?)?;
            ok(h.mkdir(&arg::<String>(&args, "path")?).await)
        }
        "sftp_close" => {
            locked(&state.sftp_sessions).map_err(err_value)?.remove(&arg::<String>(&args, "sftpId")?);
            Ok(Value::Null)
        }
        "sftp_cancel_transfer" => {
            use std::sync::atomic::Ordering;
            let tid: String = arg(&args, "transferId")?;
            if let Some(flag) = locked(&state.transfer_cancels).map_err(err_value)?.get(&tid) {
                flag.store(true, Ordering::SeqCst);
            }
            Ok(Value::Null)
        }
        "sftp_download_to" => {
            let sftp = sftp_handle(state, &arg::<String>(&args, "sftpId")?)?;
            let remote_path: String = arg(&args, "remotePath")?;
            let local_path: String = arg(&args, "localPath")?;
            let transfer_id: String = arg(&args, "transferId")?;
            // RAII: unregisters the cancel flag on return / `?` / panic (matches desktop).
            let (_guard, flag) = crate::commands::sftp::CancelGuard::register(state, transfer_id.clone())
                .map_err(err_value)?;
            let host = headless_host(state, tx);
            ok(sftp
                .download_streaming(&remote_path, std::path::Path::new(&local_path), &host, &transfer_id, flag)
                .await
                .map(|_| ()))
        }
        "sftp_upload_from" => {
            let sftp = sftp_handle(state, &arg::<String>(&args, "sftpId")?)?;
            let local_path: String = arg(&args, "localPath")?;
            let remote_path: String = arg(&args, "remotePath")?;
            let transfer_id: String = arg(&args, "transferId")?;
            let (_guard, flag) = crate::commands::sftp::CancelGuard::register(state, transfer_id.clone())
                .map_err(err_value)?;
            let host = headless_host(state, tx);
            ok(sftp
                .upload_streaming(std::path::Path::new(&local_path), &remote_path, &host, &transfer_id, flag)
                .await
                .map(|_| ()))
        }

        // ---- AI (settings + models + sessions + messaging + skills + audit) ----
        "ai_settings_get" => ok(crate::ai::commands::ai_settings_get_impl(
            state,
            args.get("provider").and_then(Value::as_str).map(str::to_string),
        )
        .await),
        "ai_settings_set" => {
            ok(crate::ai::commands::ai_settings_set_impl(state, arg(&args, "patch")?).await)
        }
        "ai_list_models" => ok(crate::ai::commands::ai_list_models_impl(
            state,
            arg(&args, "provider")?,
            args.get("apiKey").and_then(Value::as_str).map(str::to_string),
            args.get("endpoint").and_then(Value::as_str).map(str::to_string),
        )
        .await),
        "ai_list_sessions" => {
            let g = locked(&state.ai_sessions).map_err(err_value)?;
            let infos: Vec<crate::ai::commands::AiSessionInfo> =
                g.values().map(crate::ai::commands::AiSessionInfo::from).collect();
            ok(Ok::<_, AppError>(infos))
        }
        "ai_session_start" => {
            let host = headless_host(state, tx);
            ok(crate::ai::commands::ai_session_start_impl(
                state,
                host,
                arg(&args, "tabId")?,
                arg(&args, "target")?,
                args.get("skill").and_then(Value::as_str).unwrap_or("general").to_string(),
                arg(&args, "provider")?,
                arg(&args, "model")?,
                args.get("locale").and_then(Value::as_str).map(str::to_string),
            )
            .await)
        }
        "ai_user_message" => {
            ai_send(state, &arg::<String>(&args, "tabId")?, UserAction::Message(arg(&args, "text")?))
        }
        "ai_command_result" => ai_send(
            state,
            &arg::<String>(&args, "tabId")?,
            UserAction::CommandResult {
                tool_call_id: arg(&args, "toolCallId")?,
                exit_code: args.get("exitCode").and_then(Value::as_i64).unwrap_or(0) as i32,
                output: arg(&args, "output")?,
                timed_out: args.get("timedOut").and_then(Value::as_bool).unwrap_or(false),
                early_terminated: args.get("earlyTerminated").and_then(Value::as_bool).unwrap_or(false),
            },
        ),
        "ai_command_reject" => ai_send(
            state,
            &arg::<String>(&args, "tabId")?,
            UserAction::RejectCommand {
                tool_call_id: arg(&args, "toolCallId")?,
                reason: arg(&args, "reason")?,
            },
        ),
        "ai_session_clear_context" => {
            ai_send(state, &arg::<String>(&args, "tabId")?, UserAction::ClearContext)
        }
        "ai_session_stop" => {
            let tab_id: String = arg(&args, "tabId")?;
            match locked(&state.ai_sessions).map_err(err_value)?.remove(&tab_id) {
                Some(s) => {
                    let _ = s.action_tx.send(UserAction::Stop);
                    Ok(Value::Null)
                }
                None => Err(json!("ai_session_not_found")),
            }
        }
        "ai_cancel_stream" => {
            let tab_id: String = arg(&args, "tabId")?;
            let slot = locked(&state.ai_sessions)
                .map_err(err_value)?
                .get(&tab_id)
                .map(|s| s.cancel_slot.clone())
                .ok_or_else(|| json!("ai_session_not_found"))?;
            let notify = { slot.lock().map_err(|_| json!("lock_poisoned"))?.as_ref().cloned() };
            if let Some(n) = notify {
                n.notify_one();
            }
            Ok(Value::Null)
        }
        "ai_session_rebind_target" => ai_rebind(state, args),
        "ai_list_skills" => ok(crate::ai::skills::list_all(&state.db)),
        "ai_get_skill" => ok(crate::ai::skills::get(&state.db, &arg::<String>(&args, "id")?)),
        "ai_save_skill" => ok(crate::ai::skills::save_user(
            &state.db,
            &crate::ai::skills::SkillRecord {
                id: arg(&args, "id")?,
                name: arg(&args, "name")?,
                description: arg(&args, "description")?,
                content: arg(&args, "content")?,
                builtin: false,
            },
        )),
        "ai_delete_skill" => {
            ok(crate::ai::skills::delete_user(&state.db, &arg::<String>(&args, "id")?))
        }
        "ai_audit_get" => {
            let tab_id: String = arg(&args, "tabId")?;
            let audit = locked(&state.ai_sessions)
                .map_err(err_value)?
                .get(&tab_id)
                .map(|s| s.audit.clone())
                .ok_or_else(|| json!("ai_session_not_found"))?;
            let g = audit.lock().map_err(|_| json!("lock_poisoned"))?;
            ok(Ok::<_, AppError>(g.clone()))
        }
        "ai_remote_shell_probe_needed" => ok(
            crate::ai::commands::ai_remote_shell_probe_needed_impl(state, arg(&args, "targetId")?),
        ),

        // ---- GitHub config sync ----
        "github_push" => {
            ok(crate::commands::sync::github_push_impl(state, arg(&args, "password")?).await)
        }
        "github_pull" => {
            ok(crate::commands::sync::github_pull_impl(state, arg(&args, "password")?).await)
        }

        // ---- port forwarding: start (stop is sync, in dispatch) ----
        "forward_start" => {
            ok(crate::commands::forward::forward_start_impl(state, arg(&args, "forwardId")?).await)
        }

        // ---- update check ----
        "fetch_latest_release_tag" => {
            ok(crate::commands::update::fetch_latest_release_tag(arg(&args, "repo")?).await)
        }

        // ---- fonts (fontdb scan; the impl runs it on a blocking thread) ----
        "list_fonts" => ok(Ok::<_, AppError>(crate::commands::settings::list_fonts().await)),

        _ => dispatch(state, cmd, args, tx),
    }
}

fn waiter_cancel<T>(
    map: &std::sync::Mutex<std::collections::HashMap<String, T>>,
    args: &Value,
) -> Result<Value, Value> {
    let tab_id: String = arg(args, "tabId")?;
    locked(map).map_err(err_value)?.remove(&tab_id);
    Ok(Value::Null)
}

fn ssh_session(state: &AppState, sid: &str) -> Result<crate::ssh::client::SessionHandle, Value> {
    locked(&state.sessions)
        .map_err(err_value)?
        .get(sid)
        .cloned()
        .ok_or_else(|| json!("session_not_found"))
}

/// Build a `Host::Headless` whose event sink pushes `{type:"event",..}` frames
/// onto this connection's writer — the headless counterpart of `Host::Tauri`'s
/// `app.emit`. Shared by every dispatch arm that drives engine events
/// (ssh_connect, ai_session_start, sftp streaming).
fn headless_host(
    state: &Arc<AppState>,
    tx: &mpsc::UnboundedSender<Message>,
) -> crate::emitter::Host {
    let tx = tx.clone();
    crate::emitter::Host::Headless {
        sink: Arc::new(move |event: &str, payload: Value| {
            // `is_ok()` is false once the writer's receiver is dropped (connection
            // closed) → Host::emit returns Err so prompt paths stop waiting.
            tx.send(Message::Text(
                json!({ "type": "event", "event": event, "payload": payload }).to_string().into(),
            ))
            .is_ok()
        }),
        state: state.clone(),
    }
}

/// Compute the asciicast recording path for a new SSH session, mirroring the
/// desktop `ssh_connect`: honors `recording_enabled` / `recording_dir` and
/// stamps the file with profile name + timestamp. `None` when recording is off.
fn recording_path_for(
    state: &AppState,
    profile_name: &str,
) -> AppResult<Option<std::path::PathBuf>> {
    let enabled = crate::db::settings::get(&state.db, "recording_enabled")?
        .map(|v| v == "true")
        .unwrap_or(false);
    if !enabled {
        return Ok(None);
    }
    let dir_str = crate::db::settings::get(&state.db, "recording_dir")?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            dirs::document_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("rssh-recordings")
                .to_string_lossy()
                .into_owned()
        });
    let dir = std::path::PathBuf::from(&dir_str);
    std::fs::create_dir_all(&dir).ok();
    // Reduce the user-controlled profile name to a safe filename component:
    // neutralize separators and dots so it can't inject `..` or extra path
    // segments and escape the recordings dir.
    let safe = profile_name.replace(['/', '\\', '.', ' '], "_");
    let name = format!(
        "{}_{}.cast",
        safe,
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    Ok(Some(dir.join(name)))
}

/// Server-side `ssh_connect` — a faithful copy of the Tauri command body, but
/// the engine emits through a `Host::Headless` sink (ws push) and we skip the
/// per-window session bookkeeping (there are no windows).
async fn ssh_connect(
    state: &Arc<AppState>,
    args: Value,
    tx: &mpsc::UnboundedSender<Message>,
) -> Result<Value, Value> {
    use crate::ssh::client;

    let profile_id: String = arg(&args, "profileId")?;
    let log_session_id: Option<String> =
        args.get("logSessionId").and_then(Value::as_str).map(str::to_string);
    let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(80) as u32;
    let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(24) as u32;

    let profile = crate::db::profile::get(&state.db, &profile_id).map_err(err_value)?;
    let mut credential =
        crate::db::credential::get(&state.db, &profile.credential_id).map_err(err_value)?;
    credential.secret = state
        .secret_store
        .get(&crate::secret::cred_secret_key(&credential.id))
        .map_err(err_value)?;

    let chain_profiles = crate::ssh::bastion::resolve_chain(&state.db, &profile).map_err(err_value)?;
    let mut chain = Vec::with_capacity(chain_profiles.len());
    for hop in chain_profiles {
        let mut bc = crate::db::credential::get(&state.db, &hop.credential_id).map_err(err_value)?;
        bc.secret = state
            .secret_store
            .get(&crate::secret::cred_secret_key(&bc.id))
            .map_err(err_value)?;
        chain.push((hop, bc));
    }

    let verbose_log = crate::db::settings::get(&state.db, "verbose_log")
        .map_err(err_value)?
        .map(|v| v == "true")
        .unwrap_or(true);
    let timeout_secs: u64 = crate::db::settings::get(&state.db, "connect_timeout")
        .map_err(err_value)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(client::DEFAULT_CONNECT_TIMEOUT);
    let effective_log_id = if verbose_log { log_session_id } else { None };
    let known_hosts_path = crate::ssh::known_hosts::path_for(&state.data_dir);
    let init_command = profile.init_command.clone();
    let recording_path = recording_path_for(state, &profile.name).map_err(err_value)?;
    let emitter = headless_host(state, tx);

    let result = client::run_blocking_ssh(move || async move {
        client::connect(
            profile,
            credential,
            chain,
            cols,
            rows,
            emitter,
            recording_path,
            effective_log_id,
            known_hosts_path,
            timeout_secs,
        )
        .await
    })
    .await
    .map_err(err_value)?;

    if let Some(ref cmd) = init_command {
        if !cmd.is_empty() {
            result.handle.write(format!("{}\n", cmd).as_bytes()).map_err(err_value)?;
        }
    }
    locked(&state.sessions).map_err(err_value)?.insert(result.session_id.clone(), result.handle);
    Ok(json!(result.session_id))
}

/// Serve an embedded frontend asset over HTTP/1.1 (one-shot, Connection: close).
/// Unknown paths fall back to index.html for client-side routing.
async fn serve_static(mut stream: TcpStream, head: &str) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let target = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/").trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (body, ctype): (&[u8], &str) = match UI.get_file(path) {
        Some(f) => (f.contents(), mime_for(path)),
        None => match UI.get_file("index.html") {
            Some(f) => (f.contents(), "text/html; charset=utf-8"),
            None => (b"not found".as_slice(), "text/plain"),
        },
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.write_all(body).await?;
    let _ = stream.shutdown().await;
    Ok(())
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn ai_send(state: &AppState, tab_id: &str, action: UserAction) -> Result<Value, Value> {
    let tx = locked(&state.ai_sessions)
        .map_err(err_value)?
        .get(tab_id)
        .map(|s| s.action_tx.clone())
        .ok_or_else(|| json!("ai_session_not_found"))?;
    tx.send(action).map(|_| Value::Null).map_err(|_| json!("ai_session_stopped"))
}

fn ai_rebind(state: &AppState, args: Value) -> Result<Value, Value> {
    use crate::ai::commands::AiTarget;
    let target: AiTarget = arg(&args, "target")?;
    let tab_id: String = arg(&args, "tabId")?;
    let ssh_handle = match &target {
        AiTarget::Ssh(id) => Some(
            locked(&state.sessions)
                .map_err(err_value)?
                .get(id)
                .ok_or_else(|| json!("ssh_session_not_found"))?
                .ssh_handle()
                .clone(),
        ),
        AiTarget::Local(id) => {
            if !locked(&state.pty_sessions).map_err(err_value)?.contains_key(id) {
                return Err(json!("local_pty_not_found"));
            }
            None
        }
    };
    let target_id = target.id().to_string();
    let tx = {
        let mut g = locked(&state.ai_sessions).map_err(err_value)?;
        let s = g.get_mut(&tab_id).ok_or_else(|| json!("ai_session_not_found"))?;
        s.target_id = target_id.clone();
        s.action_tx.clone()
    };
    tx.send(UserAction::RebindTarget { target_id, ssh_handle })
        .map(|_| Value::Null)
        .map_err(|_| json!("ai_session_stopped"))
}

fn sftp_handle(
    state: &AppState,
    id: &str,
) -> Result<std::sync::Arc<crate::ssh::sftp::SftpHandle>, Value> {
    locked(&state.sftp_sessions)
        .map_err(err_value)?
        .get(id)
        .cloned()
        .ok_or_else(|| json!("sftp_session_not_found"))
}

async fn sftp_connect(state: &Arc<AppState>, args: Value) -> Result<Value, Value> {
    use crate::models::{Credential, CredentialType};
    use crate::ssh::sftp::SftpHandle;
    let host: String = arg(&args, "host")?;
    let port = args.get("port").and_then(Value::as_u64).unwrap_or(22) as u16;
    let username: String = arg(&args, "username")?;
    let auth_type: String = arg(&args, "authType")?;
    let secret: Option<String> = args.get("secret").and_then(Value::as_str).map(str::to_string);
    let cred = Credential {
        id: String::new(),
        name: String::new(),
        username,
        credential_type: CredentialType::from_str(&auth_type),
        secret,
        save_to_remote: false,
    };
    let timeout_secs: u64 = crate::db::settings::get(&state.db, "connect_timeout")
        .map_err(err_value)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(crate::ssh::client::DEFAULT_CONNECT_TIMEOUT);
    let known_hosts_path = crate::ssh::known_hosts::path_for(&state.data_dir);
    let handle = crate::ssh::client::run_blocking_ssh(move || async move {
        SftpHandle::connect(host, port, cred, known_hosts_path, timeout_secs).await
    })
    .await
    .map_err(err_value)?;
    let id = uuid::Uuid::new_v4().to_string();
    locked(&state.sftp_sessions).map_err(err_value)?.insert(id.clone(), std::sync::Arc::new(handle));
    Ok(json!(id))
}

async fn sftp_connect_session(state: &Arc<AppState>, args: Value) -> Result<Value, Value> {
    use crate::ssh::sftp::SftpHandle;
    let session_id: String = arg(&args, "sessionId")?;
    let ssh_handle = {
        let sessions = locked(&state.sessions).map_err(err_value)?;
        sessions
            .get(&session_id)
            .ok_or_else(|| json!("ssh_session_not_found"))?
            .ssh_handle()
            .clone()
    };
    let parent = session_id.clone();
    let handle = crate::ssh::client::run_blocking_ssh(move || async move {
        SftpHandle::from_handle(&ssh_handle, parent).await
    })
    .await
    .map_err(err_value)?;
    let id = uuid::Uuid::new_v4().to_string();
    locked(&state.sftp_sessions).map_err(err_value)?.insert(id.clone(), std::sync::Arc::new(handle));
    Ok(json!(id))
}

/// Map an `AppResult<T>` into the wire's Ok(result) / Err(error) JSON.
fn ok<T: serde::Serialize>(r: AppResult<T>) -> Result<Value, Value> {
    match r {
        Ok(v) => serde_json::to_value(v).map_err(|e| json!(e.to_string())),
        Err(e) => Err(err_value(e)),
    }
}

/// Deserialize one named argument (camelCase, as Tauri sends it on the wire).
fn arg<T: serde::de::DeserializeOwned>(args: &Value, key: &str) -> Result<T, Value> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|e| json!(format!("bad arg '{key}': {e}")))
}

fn save_cred_secret(state: &AppState, c: &Credential) -> Result<(), Value> {
    let key = crate::secret::cred_secret_key(&c.id);
    match c.secret.as_deref() {
        Some(s) if !s.is_empty() => state.secret_store.set(&key, s).map_err(err_value),
        _ => state.secret_store.delete(&key).map_err(err_value),
    }
}

fn err_value(e: AppError) -> Value {
    json!(e.to_string())
}

fn query_token(q: &str) -> Option<String> {
    q.split('&')
        .find_map(|kv| kv.strip_prefix("token="))
        .map(|s| s.to_string())
}
