//! `rssh-server` — headless ws server entry point. Reuses the rssh engine via
//! `rssh_lib::server`; the IDEA plugin (or a dev script) spawns this and reads
//! the `{"port":..,"token":..}` line it prints on stdout to point the frontend.

fn main() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    if let Err(e) = rt.block_on(rssh_lib::server::run()) {
        eprintln!("rssh-server failed: {e}");
        std::process::exit(1);
    }
}
