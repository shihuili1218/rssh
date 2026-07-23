#[cfg(desktop)]
pub mod cli;
pub mod discovery;
pub mod external;
pub mod forward;
pub mod group;
pub mod lifecycle;
pub mod profile;
#[cfg(desktop)]
pub mod pty;
#[cfg(desktop)]
pub mod serial;
pub mod session;
pub mod settings;
pub mod sftp;
pub mod sync;
pub mod telnet;
pub mod update;
#[cfg(desktop)]
pub mod window;
