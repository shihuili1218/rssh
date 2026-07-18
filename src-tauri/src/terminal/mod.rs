#[cfg(desktop)]
pub mod pty;
pub mod recorder;
#[cfg(desktop)]
pub mod serial;
// No mobile gate: telnet is plain TCP, so it works on every platform.
pub mod telnet;
