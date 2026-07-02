#[cfg(not(target_os = "android"))]
pub mod pty;
pub mod recorder;
#[cfg(not(target_os = "android"))]
pub mod serial;
// No android gate: telnet is plain TCP, it works on every platform.
pub mod telnet;
