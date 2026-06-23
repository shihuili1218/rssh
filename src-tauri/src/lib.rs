mod ai;
mod commands;
pub mod crypto;
pub mod db;
pub mod emitter;
pub mod error;
pub mod migration;
pub mod models;
pub mod secret;
mod ssh;
pub use ssh::bastion;
#[cfg(all(feature = "server", not(target_os = "android")))]
pub mod server;
mod state;
pub mod sync;
mod terminal;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::Manager;

use state::AppState;

#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn apply_linux_wayland_compat() {
    if std::env::var_os("RSSH_DISABLE_WAYLAND_COMPAT").is_some() {
        return;
    }

    let wayland_session = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);

    if !wayland_session {
        return;
    }

    // Wayland 兼容：部分 NVIDIA / wlroots 环境下，WebKitGTK 的 DMABUF renderer
    // 会在 Tauri 窗口创建前失败。Prefer reliable startup by default; users can
    // still override this variable explicitly.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // GBM 后端兼容：全局导出的 GBM_BACKEND，尤其是 nvidia-drm，可能导致
    // Hyprland 下 GTK / WebKitGTK 报 "Failed to create GBM buffer"。Keep an
    // explicit opt-out for users whose stack needs this variable.
    if std::env::var_os("RSSH_KEEP_GBM_BACKEND").is_none() {
        std::env::remove_var("GBM_BACKEND");
    }
}

#[cfg(any(not(target_os = "linux"), target_os = "android"))]
fn apply_linux_wayland_compat() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    apply_linux_wayland_compat();

    // 默认 info；用 RUST_LOG=debug 等覆盖
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::Destroyed => {
                    let state = window.state::<AppState>();
                    // Close only sessions belonging to this window.
                    commands::lifecycle::close_window_sessions(&state, window.label());
                    // Drop it from any move-together group (survivors stay bound).
                    #[cfg(desktop)]
                    state
                        .window_groups
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(window.label());
                }
                // Live window binding: mirror this window's drag onto its group
                // siblings. Binding SUSPENDS at the OS boundary — a window
                // animating into fullscreen (its own Space on macOS) or
                // minimizing fires a Moved we must not propagate.
                #[cfg(desktop)]
                tauri::WindowEvent::Moved(pos) => {
                    if window.is_fullscreen().unwrap_or(false)
                        || window.is_minimized().unwrap_or(false)
                    {
                        return;
                    }
                    let moves = window
                        .state::<AppState>()
                        .window_groups
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .moved(window.label(), (pos.x, pos.y), std::time::Instant::now());
                    for (label, (x, y)) in moves {
                        if let Some(w) = window.get_webview_window(&label) {
                            let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
                        }
                    }
                }
                _ => {}
            }
        })
        .setup(|app| {
            #[cfg(target_os = "android")]
            let data_dir = app.path().app_data_dir()?;
            #[cfg(not(target_os = "android"))]
            let data_dir = db::data_dir()?;

            // 启动时扫一次本机可用 shell，结果缓存到进程退出。
            // 用户在 Shell 设置页打开时直接读缓存，没冷启动开销。
            // PTY 模块本身就是桌面端独占（android 上没有 portable_pty）。
            #[cfg(not(target_os = "android"))]
            terminal::pty::init_available_shells();
            let db = Arc::new(db::Db::open(&data_dir)?);
            // secret::open 可能失败：sticky backend 标记 keyring 但 keychain 现在
            // 拿不到（系统 keychain 损坏 / D-Bus 挂等）→ 硬 fail 启动。silently
            // fallback file 会用新主密钥让旧密文全部解不开，比启动失败更危险。
            let secret_system = secret::open(db.clone(), &data_dir)?;

            // 启动一次性迁移。失败不阻塞启动（log warn，下次启动重试），跟原
            // passphrase 清理逻辑的"软失败"风格一致。所有 marker 走 settings 表，
            // 已完成的用户启动等价于零成本跳过。
            if let Err(e) = migration::run_migrations(
                &db,
                secret_system.raw_keyring.as_deref(),
                secret_system.store.as_ref(),
            ) {
                log::warn!("migration failed (will retry on next startup): {e}");
            }

            app.manage(AppState {
                db,
                secret_store: secret_system.store,
                sessions: Mutex::new(HashMap::new()),
                #[cfg(not(target_os = "android"))]
                pty_sessions: Mutex::new(HashMap::new()),
                #[cfg(not(target_os = "android"))]
                serial_sessions: Mutex::new(HashMap::new()),
                sftp_sessions: Mutex::new(HashMap::new()),
                transfer_cancels: Mutex::new(HashMap::new()),
                active_forwards: Mutex::new(HashMap::new()),
                auth_waiters: Mutex::new(HashMap::new()),
                passphrase_waiters: Mutex::new(HashMap::new()),
                host_key_waiters: Mutex::new(HashMap::new()),
                passphrase_cache: Mutex::new(HashMap::new()),
                window_sessions: Mutex::new(HashMap::new()),
                #[cfg(desktop)]
                window_groups: Mutex::new(commands::window::WindowGroups::default()),
                ai_sessions: Mutex::new(HashMap::new()),
                ai_remote_shell_cache: Mutex::new(HashMap::new()),
                data_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // profile & credential
            commands::profile::list_profiles,
            commands::profile::get_profile,
            commands::profile::create_profile,
            commands::profile::update_profile,
            commands::profile::delete_profile,
            commands::profile::list_credentials,
            commands::profile::get_credential,
            commands::profile::create_credential,
            commands::profile::update_credential,
            commands::profile::delete_credential,
            commands::profile::import_ssh_config,
            commands::profile::read_ssh_config_default,
            commands::profile::import_ssh_entries,
            commands::profile::pick_private_key_file,
            // groups
            commands::group::list_groups,
            commands::group::create_group,
            commands::group::update_group,
            commands::group::delete_group,
            // forward CRUD
            commands::forward::list_forwards,
            commands::forward::get_forward,
            commands::forward::create_forward,
            commands::forward::update_forward,
            commands::forward::delete_forward,
            // forward active
            commands::forward::forward_start,
            commands::forward::forward_stats,
            commands::forward::forward_stop,
            // settings & snippets & highlights
            commands::settings::get_setting,
            commands::settings::set_setting,
            commands::settings::list_highlights,
            commands::settings::add_highlight,
            commands::settings::remove_highlight,
            commands::settings::update_highlight,
            commands::settings::load_snippets,
            commands::settings::save_snippets,
            commands::settings::reset_highlights,
            commands::settings::list_recordings,
            commands::settings::read_recording,
            commands::settings::secret_backend,
            commands::settings::list_fonts,
            // SSH session
            commands::session::ssh_connect,
            commands::session::ssh_write,
            commands::session::ssh_resize,
            commands::session::ssh_disconnect,
            commands::session::ssh_auth_respond,
            commands::session::ssh_auth_cancel,
            commands::session::ssh_passphrase_respond,
            commands::session::ssh_passphrase_cancel,
            commands::session::ssh_host_key_respond,
            commands::session::ssh_host_key_cancel,
            // session lifecycle
            commands::lifecycle::reconcile_sessions,
            // PTY (desktop only)
            #[cfg(not(target_os = "android"))]
            commands::pty::list_shells,
            #[cfg(not(target_os = "android"))]
            commands::pty::refresh_shells,
            #[cfg(not(target_os = "android"))]
            commands::pty::pty_spawn,
            #[cfg(not(target_os = "android"))]
            commands::pty::pty_write,
            #[cfg(not(target_os = "android"))]
            commands::pty::pty_resize,
            #[cfg(not(target_os = "android"))]
            commands::pty::pty_close,
            // Serial (desktop only)
            #[cfg(not(target_os = "android"))]
            commands::serial::serial_list_ports,
            #[cfg(not(target_os = "android"))]
            commands::serial::serial_open,
            #[cfg(not(target_os = "android"))]
            commands::serial::serial_write,
            #[cfg(not(target_os = "android"))]
            commands::serial::serial_close,
            #[cfg(not(target_os = "android"))]
            commands::serial::serial_set_dtr,
            #[cfg(not(target_os = "android"))]
            commands::serial::serial_set_rts,
            #[cfg(not(target_os = "android"))]
            commands::serial::serial_send_break,
            #[cfg(not(target_os = "android"))]
            commands::serial::list_serial_profiles,
            #[cfg(not(target_os = "android"))]
            commands::serial::get_serial_profile,
            #[cfg(not(target_os = "android"))]
            commands::serial::create_serial_profile,
            #[cfg(not(target_os = "android"))]
            commands::serial::update_serial_profile,
            #[cfg(not(target_os = "android"))]
            commands::serial::delete_serial_profile,
            // SFTP
            commands::sftp::sftp_connect,
            commands::sftp::sftp_connect_session,
            commands::sftp::sftp_home,
            commands::sftp::sftp_list,
            commands::sftp::sftp_walk_remote_dir,
            commands::sftp::walk_local_dir,
            commands::sftp::sftp_download,
            commands::sftp::sftp_upload,
            commands::sftp::sftp_mkdir,
            commands::sftp::sftp_close,
            // SFTP native file transfer (desktop only)
            #[cfg(not(target_os = "android"))]
            commands::sftp::sftp_save_file,
            #[cfg(not(target_os = "android"))]
            commands::sftp::sftp_pick_and_upload,
            #[cfg(not(target_os = "android"))]
            commands::sftp::sftp_pick_save_path,
            #[cfg(not(target_os = "android"))]
            commands::sftp::sftp_pick_open_path,
            #[cfg(not(target_os = "android"))]
            commands::sftp::sftp_pick_folder,
            #[cfg(not(target_os = "android"))]
            commands::sftp::sftp_pick_open_files,
            #[cfg(not(target_os = "android"))]
            commands::sftp::sftp_download_to,
            #[cfg(not(target_os = "android"))]
            commands::sftp::sftp_upload_from,
            commands::sftp::sftp_cancel_transfer,
            commands::sftp::sftp_remove,
            commands::sftp::sftp_rename,
            commands::sftp::sftp_stat,
            // CLI install
            commands::cli::cli_status,
            commands::cli::cli_install,
            // multi-window (desktop only)
            #[cfg(not(target_os = "android"))]
            commands::window::open_tab_in_new_window,
            #[cfg(not(target_os = "android"))]
            commands::window::clipboard_read,
            #[cfg(not(target_os = "android"))]
            commands::window::clipboard_write,
            // external URL opener — cross-platform via tauri-plugin-opener
            commands::external::open_external_url,
            // update check (cross-platform — separate mod from window)
            commands::update::fetch_latest_release_tag,
            // sync
            commands::sync::export_config,
            commands::sync::import_config,
            #[cfg(not(target_os = "android"))]
            commands::sync::export_config_to_file,
            #[cfg(not(target_os = "android"))]
            commands::sync::import_config_from_file,
            commands::sync::github_push,
            commands::sync::github_pull,
            commands::sync::webdav_push,
            commands::sync::webdav_pull,
            // AI 排障
            ai::commands::ai_list_skills,
            ai::commands::ai_get_skill,
            ai::commands::ai_save_skill,
            ai::commands::ai_delete_skill,
            ai::commands::ai_list_redact_rules,
            ai::commands::ai_save_redact_rule,
            ai::commands::ai_delete_redact_rule,
            ai::commands::ai_list_command_blacklist,
            ai::commands::ai_replace_command_blacklist,
            ai::commands::ai_session_start,
            ai::commands::ai_session_stop,
            ai::commands::ai_session_clear_context,
            ai::commands::ai_session_rebind_target,
            ai::commands::ai_remote_shell_probe_needed,
            ai::commands::ai_cache_remote_shell,
            ai::commands::ai_cancel_stream,
            ai::commands::ai_user_message,
            ai::commands::ai_command_result,
            ai::commands::ai_command_reject,
            ai::commands::ai_audit_save,
            ai::commands::ai_audit_save_pick,
            ai::commands::ai_audit_get,
            ai::commands::ai_list_sessions,
            ai::commands::ai_conversations_list,
            ai::commands::ai_conversation_timeline,
            ai::commands::ai_conversation_save_timeline,
            ai::commands::ai_conversation_delete,
            ai::commands::ai_settings_get,
            ai::commands::ai_settings_set,
            ai::commands::ai_list_models,
        ])
        .run(tauri::generate_context!())
        .expect("RSSH startup failed");
}
