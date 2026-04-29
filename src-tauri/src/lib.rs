mod ai;
mod commands;
pub mod crypto;
pub mod db;
pub mod error;
pub mod models;
pub mod secret;
mod ssh;
pub use ssh::bastion;
mod state;
pub mod sync;
mod terminal;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 默认 info；用 RUST_LOG=debug 等覆盖
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    tauri::Builder::default()
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let state = window.state::<AppState>();
                let label = window.label();
                // Close only sessions belonging to this window.
                commands::lifecycle::close_window_sessions(&state, label);
            }
        })
        .setup(|app| {
            #[cfg(target_os = "android")]
            let data_dir = app.path().app_data_dir()?;
            #[cfg(not(target_os = "android"))]
            let data_dir = db::data_dir();
            let db = Arc::new(db::Db::open(&data_dir)?);
            let secret_store = secret::open(db.clone());

            // 一次性清理：旧版曾把私钥 passphrase 落盘到 SecretStore。
            // 新流程改为终端内交互输入 + 进程内缓存，旧条目永不读，全部删掉。
            // 失败不致命：删不掉的项最坏情况是占几个字节，不影响功能。
            if let Ok(creds) = db::credential::list(&db) {
                for c in creds {
                    let _ = secret_store.delete(&secret::cred_passphrase_key(&c.id));
                }
            }

            app.manage(AppState {
                db,
                secret_store,
                sessions: Mutex::new(HashMap::new()),
                #[cfg(not(target_os = "android"))]
                pty_sessions: Mutex::new(HashMap::new()),
                sftp_sessions: Mutex::new(HashMap::new()),
                transfer_cancels: Mutex::new(HashMap::new()),
                active_forwards: Mutex::new(HashMap::new()),
                auth_waiters: Mutex::new(HashMap::new()),
                passphrase_waiters: Mutex::new(HashMap::new()),
                passphrase_cache: Mutex::new(HashMap::new()),
                window_sessions: Mutex::new(HashMap::new()),
                ai_sessions: Mutex::new(HashMap::new()),
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
            commands::settings::load_snippets,
            commands::settings::save_snippets,
            commands::settings::reset_highlights,
            commands::settings::list_recordings,
            commands::settings::read_recording,
            commands::settings::secret_backend,
            // SSH session
            commands::session::ssh_connect,
            commands::session::ssh_write,
            commands::session::ssh_resize,
            commands::session::ssh_disconnect,
            commands::session::ssh_auth_respond,
            commands::session::ssh_passphrase_respond,
            commands::session::ssh_passphrase_cancel,
            // session lifecycle
            commands::lifecycle::reconcile_sessions,
            // PTY (desktop only)
            #[cfg(not(target_os = "android"))]
            commands::pty::list_shells,
            #[cfg(not(target_os = "android"))]
            commands::pty::pty_spawn,
            #[cfg(not(target_os = "android"))]
            commands::pty::pty_write,
            #[cfg(not(target_os = "android"))]
            commands::pty::pty_resize,
            #[cfg(not(target_os = "android"))]
            commands::pty::pty_close,
            // SFTP
            commands::sftp::sftp_connect,
            commands::sftp::sftp_connect_session,
            commands::sftp::sftp_home,
            commands::sftp::sftp_list,
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
            commands::sftp::sftp_download_to,
            #[cfg(not(target_os = "android"))]
            commands::sftp::sftp_upload_from,
            commands::sftp::sftp_cancel_transfer,
            // CLI install
            commands::cli::cli_status,
            commands::cli::cli_install,
            // multi-window (desktop only)
            #[cfg(not(target_os = "android"))]
            commands::window::open_tab_in_new_window,
            #[cfg(not(target_os = "android"))]
            commands::window::open_external_url,
            #[cfg(not(target_os = "android"))]
            commands::window::clipboard_read,
            #[cfg(not(target_os = "android"))]
            commands::window::fetch_latest_release_tag,
            // sync
            commands::sync::export_config,
            commands::sync::import_config,
            commands::sync::github_push,
            commands::sync::github_pull,
            // AI 排障
            ai::commands::ai_list_skills,
            ai::commands::ai_get_skill,
            ai::commands::ai_save_skill,
            ai::commands::ai_delete_skill,
            ai::commands::ai_session_start,
            ai::commands::ai_session_stop,
            ai::commands::ai_user_message,
            ai::commands::ai_command_result,
            ai::commands::ai_command_reject,
            ai::commands::ai_audit_save,
            ai::commands::ai_audit_save_pick,
            ai::commands::ai_audit_get,
            ai::commands::ai_list_sessions,
            ai::commands::ai_settings_get,
            ai::commands::ai_settings_set,
        ])
        .run(tauri::generate_context!())
        .expect("启动 RSSH 失败");
}
