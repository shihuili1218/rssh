#[cfg(not(target_os = "android"))]
pub mod pty;
pub mod recorder;
#[cfg(not(target_os = "android"))]
pub mod serial;
