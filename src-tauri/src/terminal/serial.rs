use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};

use crate::error::{locked, AppError, AppResult};

/// Serial output destined for the host. Mirrors `pty::PtyOut`, but kept as its
/// own type so the serial module carries no dependency on the pty module — two
/// unrelated transports shouldn't couple just to share a 2-variant enum. The
/// Tauri command turns these into `serial:data:<id>` / `serial:close:<id>`.
pub enum SerialOut {
    Data(Vec<u8>),
    Close,
}

/// Sink the reader thread invokes for each chunk. The `&str` is the session id,
/// so one sink can serve any number of serial sessions (same shape as PtySink).
pub type SerialSink = Arc<dyn Fn(&str, SerialOut) + Send + Sync>;

/// Wire config from the frontend. Strings (not enums) for parity/flow because it
/// arrives as JSON; the string→enum mapping with fail-soft defaults lives here
/// (pure, unit-tested below). snake_case keys match the SerialProfile model and
/// the tab meta, so a saved profile's fields feed `serial_open` with no remap.
#[derive(Clone, serde::Deserialize)]
pub struct SerialConfig {
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default)]
    pub parity: String,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    #[serde(default)]
    pub flow_control: String,
    /// IXANY termios flag (unix only; Windows no-op). The rest of the Tabby-style
    /// settings live on SerialProfile and are applied in the frontend terminal.
    #[serde(default)]
    pub xany: bool,
}

fn default_data_bits() -> u8 {
    8
}
fn default_stop_bits() -> u8 {
    1
}

/// 8 data bits is the universal default; anything unrecognized falls back to it
/// rather than erroring — the line should open on 8N1 even if the UI sends junk.
fn map_data_bits(n: u8) -> DataBits {
    match n {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    }
}

fn map_parity(s: &str) -> Parity {
    match s {
        "odd" => Parity::Odd,
        "even" => Parity::Even,
        _ => Parity::None,
    }
}

fn map_stop_bits(n: u8) -> StopBits {
    match n {
        2 => StopBits::Two,
        _ => StopBits::One,
    }
}

fn map_flow_control(s: &str) -> FlowControl {
    match s {
        "software" => FlowControl::Software,
        "hardware" => FlowControl::Hardware,
        _ => FlowControl::None,
    }
}

/// Trips the shared close flag when the last `SerialHandle` clone is dropped
/// (tab closed / window closed → handle removed from the session map). Unlike a
/// PTY there is no child process whose death yields EOF, and dropping the writer
/// fd does NOT close the reader's `try_clone`d fd — so the reader thread must be
/// told to stop explicitly. It checks the flag once per read cycle (≤ timeout).
struct CloseGuard {
    closed: Arc<AtomicBool>,
}

impl Drop for CloseGuard {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

/// Serial session handle. Clone + Send + Sync, like `PtyHandle`. No `resize`:
/// a serial line has no rows/cols, so the frontend simply never calls it.
#[derive(Clone)]
pub struct SerialHandle {
    writer: Arc<Mutex<Box<dyn SerialPort>>>,
    /// Last clone dropped → guard drops → close flag set → reader thread exits.
    _guard: Arc<CloseGuard>,
    port_name: Arc<str>,
}

/// Map any serial I/O failure (write / control-line / break) to a uniform error.
fn serial_op_err(e: impl std::fmt::Display) -> AppError {
    AppError::pty(
        "serial_op_failed",
        serde_json::json!({ "err": e.to_string() }),
    )
}

impl SerialHandle {
    pub fn write(&self, data: &[u8]) -> AppResult<()> {
        locked(&self.writer)?.write_all(data).map_err(serial_op_err)
    }

    /// Drive the DTR control line. Manual DTR/RTS toggling resets MCUs (Arduino
    /// auto-reset), enters bootloaders (the ESP32 DTR/RTS dance), and signals
    /// modems. `level` is the logical assert state (`true` = asserted).
    pub fn set_dtr(&self, level: bool) -> AppResult<()> {
        locked(&self.writer)?
            .write_data_terminal_ready(level)
            .map_err(serial_op_err)
    }

    /// Drive the RTS control line (see `set_dtr`).
    pub fn set_rts(&self, level: bool) -> AppResult<()> {
        locked(&self.writer)?
            .write_request_to_send(level)
            .map_err(serial_op_err)
    }

    /// Send a serial BREAK: hold the line in the break condition ~250ms, then
    /// release. Devices that watch for it (U-Boot, kernel SysRq-over-serial,
    /// telco gear) treat it as an attention/interrupt. The writer lock is held
    /// across the pulse so no bytes interleave — a break mid-frame is meaningless.
    pub fn send_break(&self) -> AppResult<()> {
        let port = locked(&self.writer)?;
        port.set_break().map_err(serial_op_err)?;
        std::thread::sleep(Duration::from_millis(250));
        port.clear_break().map_err(serial_op_err)
    }

    /// Port path actually opened (e.g. `/dev/cu.usbserial-1420`, `COM3`).
    pub fn port_name(&self) -> &str {
        &self.port_name
    }
}

/// Serial ports available on this machine (`/dev/cu.usbserial-*`, `COM3`, …).
/// Enumeration failure (no permission / platform quirk) degrades to an empty
/// list rather than erroring — the UI shows "no ports" and the user can retry.
pub fn available_ports() -> Vec<String> {
    let ports = serialport::available_ports()
        .map(|ports| ports.into_iter().map(|p| p.port_name).collect::<Vec<_>>())
        .unwrap_or_default();
    prefer_callout_ports(ports)
}

/// macOS exposes BOTH device nodes for every serial port: the call-out
/// `/dev/cu.*` and the dial-in `/dev/tty.*`. For an interactive terminal you
/// always want `cu.*` — opening `tty.*` blocks waiting for carrier-detect (DCD).
/// Drop the `tty.*` twins so the picker shows one usable entry per device
/// instead of a doubled list with a hang-trap in it.
#[cfg(target_os = "macos")]
fn prefer_callout_ports(mut ports: Vec<String>) -> Vec<String> {
    // Drop a dial-in node only when its call-out twin was also enumerated:
    // `/dev/tty.foo` is dropped iff `/dev/cu.foo` exists. A device that exposes
    // only `tty.*` (no cu twin) is still shown rather than silently hidden.
    let callouts: std::collections::HashSet<String> = ports
        .iter()
        .filter_map(|p| p.strip_prefix("/dev/cu.").map(str::to_owned))
        .collect();
    ports.retain(|p| match p.strip_prefix("/dev/tty.") {
        Some(name) => !callouts.contains(name),
        None => true,
    });
    ports
}

/// Non-macOS: device nodes aren't duplicated this way, so pass through.
#[cfg(not(target_os = "macos"))]
fn prefer_callout_ports(ports: Vec<String>) -> Vec<String> {
    ports
}

/// Open a serial port and spawn a reader thread that pushes bytes to `sink`.
/// Returns `(session_id, handle)`.
fn open_err(port: &str, e: serialport::Error) -> AppError {
    AppError::pty(
        "serial_open_failed",
        serde_json::json!({ "port": port, "err": e.to_string() }),
    )
}

/// Open the port and, on unix, set the IXANY termios flag when `xany` is on — the
/// serialport crate doesn't expose it, so we reach the raw fd via libc.
#[cfg(unix)]
fn open_with_xany(
    builder: serialport::SerialPortBuilder,
    xany: bool,
    port: &str,
) -> AppResult<Box<dyn SerialPort>> {
    use std::os::unix::io::AsRawFd;
    let tty = builder.open_native().map_err(|e| open_err(port, e))?;
    if xany {
        // SAFETY: fd is valid for this call; tcgetattr/tcsetattr touch only `t`.
        unsafe {
            let fd = tty.as_raw_fd();
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut t) == 0 {
                t.c_iflag |= libc::IXANY;
                let _ = libc::tcsetattr(fd, libc::TCSANOW, &t);
            }
        }
    }
    Ok(Box::new(tty))
}

/// Windows: IXANY has no DCB equivalent exposed by the serialport crate, so the
/// `xany` flag is a no-op. Documented limitation.
#[cfg(not(unix))]
fn open_with_xany(
    builder: serialport::SerialPortBuilder,
    _xany: bool,
    port: &str,
) -> AppResult<Box<dyn SerialPort>> {
    builder.open().map_err(|e| open_err(port, e))
}

pub fn open(port: &str, cfg: SerialConfig, sink: SerialSink) -> AppResult<(String, SerialHandle)> {
    let builder = serialport::new(port, cfg.baud_rate)
        .data_bits(map_data_bits(cfg.data_bits))
        .parity(map_parity(&cfg.parity))
        .stop_bits(map_stop_bits(cfg.stop_bits))
        .flow_control(map_flow_control(&cfg.flow_control))
        // Read timeout paces the reader loop and lets it notice unplug + honor the
        // close flag. NOT a data deadline — on TimedOut we just loop again.
        .timeout(Duration::from_millis(100));
    let opened = open_with_xany(builder, cfg.xany, port)?;

    // Separate read handle so the blocking read (up to the timeout) doesn't hold
    // the writer's lock. try_clone dups the underlying fd / handle.
    let reader = opened.try_clone().map_err(|e| {
        AppError::pty(
            "serial_op_failed",
            serde_json::json!({ "err": e.to_string() }),
        )
    })?;

    let closed = Arc::new(AtomicBool::new(false));
    let reader_closed = closed.clone();

    let id = uuid::Uuid::new_v4().to_string();
    let handle = SerialHandle {
        writer: Arc::new(Mutex::new(opened)),
        _guard: Arc::new(CloseGuard { closed }),
        port_name: Arc::from(port),
    };

    // Reader thread: serial RX → sink. Exits on close flag, on unplug (read
    // error), and emits Close on the way out so the frontend can mark the tab.
    let sid = id.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            if reader_closed.load(Ordering::Relaxed) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => continue, // no bytes this interval
                Ok(n) => sink(&sid, SerialOut::Data(buf[..n].to_vec())),
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(_) => break, // port gone (unplugged) / fatal IO error
            }
        }
        sink(&sid, SerialOut::Close);
    });

    Ok((id, handle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_bits_maps_known_and_defaults_junk_to_eight() {
        assert!(matches!(map_data_bits(5), DataBits::Five));
        assert!(matches!(map_data_bits(6), DataBits::Six));
        assert!(matches!(map_data_bits(7), DataBits::Seven));
        assert!(matches!(map_data_bits(8), DataBits::Eight));
        assert!(matches!(map_data_bits(0), DataBits::Eight));
        assert!(matches!(map_data_bits(99), DataBits::Eight));
    }

    #[test]
    fn parity_maps_known_and_defaults_unknown_to_none() {
        assert!(matches!(map_parity("odd"), Parity::Odd));
        assert!(matches!(map_parity("even"), Parity::Even));
        assert!(matches!(map_parity("none"), Parity::None));
        assert!(matches!(map_parity(""), Parity::None));
        assert!(matches!(map_parity("garbage"), Parity::None));
    }

    #[test]
    fn stop_bits_maps_two_else_one() {
        assert!(matches!(map_stop_bits(2), StopBits::Two));
        assert!(matches!(map_stop_bits(1), StopBits::One));
        assert!(matches!(map_stop_bits(0), StopBits::One));
    }

    #[test]
    fn flow_control_maps_known_and_defaults_unknown_to_none() {
        assert!(matches!(
            map_flow_control("software"),
            FlowControl::Software
        ));
        assert!(matches!(
            map_flow_control("hardware"),
            FlowControl::Hardware
        ));
        assert!(matches!(map_flow_control("none"), FlowControl::None));
        assert!(matches!(map_flow_control("xyz"), FlowControl::None));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_drops_tty_only_when_cu_twin_exists() {
        let got = prefer_callout_ports(vec![
            "/dev/cu.usbserial-1".into(),
            "/dev/tty.usbserial-1".into(), // twin of a listed cu → dropped
            "/dev/tty.orphan".into(),      // no cu twin → kept (not hidden)
            "/dev/cu.standalone".into(),
        ]);
        assert!(got.contains(&"/dev/cu.usbserial-1".to_string()));
        assert!(!got.contains(&"/dev/tty.usbserial-1".to_string()));
        assert!(got.contains(&"/dev/tty.orphan".to_string()));
        assert!(got.contains(&"/dev/cu.standalone".to_string()));
    }
}
