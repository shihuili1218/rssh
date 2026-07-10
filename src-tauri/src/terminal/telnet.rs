//! Minimal telnet client transport (RFC 854 NVT + option negotiation).
//!
//! Same skeleton as `serial.rs`: a blocking socket + reader thread + CloseGuard,
//! output pushed through a sink. The telnet-specific piece is `Negotiator` — a
//! pure byte state machine (unit-tested without sockets) that strips IAC
//! sequences from the inbound stream and produces the protocol replies.
//!
//! Negotiation policy is deliberately small and purely reactive (we never
//! initiate options; every real-world telnetd — BusyBox, Cisco/Huawei/H3C,
//! inetd telnetd — initiates within the first packet):
//!   accept from server: ECHO, SGA, BINARY        (reply DO)
//!   agree to enable:    NAWS, TTYPE, SGA, BINARY (reply WILL)
//!   everything else:    refuse (DONT / WONT)
//! NAWS gives us window-size reporting (which serial can't have) and TTYPE
//! reports "xterm-256color" to match the frontend terminal.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::error::{locked, AppError, AppResult};

/// Telnet output destined for the host. Mirrors `serial::SerialOut` — its own
/// type for the same reason: unrelated transports shouldn't couple just to
/// share a 2-variant enum. The Tauri command turns these into
/// `telnet:data:<id>` / `telnet:close:<id>`.
pub enum TelnetOut {
    Data(Vec<u8>),
    /// Whether the peer now owns echoing. The frontend uses this to switch
    /// automatic local echo without guessing from profile defaults.
    RemoteEcho(bool),
    Close,
}

/// Sink the reader thread invokes for each chunk (`&str` = session id).
pub type TelnetSink = Arc<dyn Fn(&str, TelnetOut) + Send + Sync>;

// ── Telnet protocol bytes ──

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

const OPT_BINARY: u8 = 0;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;

/// TTYPE subnegotiation verbs (RFC 1091).
const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;

/// Reported to the server on TTYPE SEND; matches the xterm.js frontend.
const TERM_TYPE: &[u8] = b"xterm-256color";

/// Subnegotiation payloads we care about are tiny (TTYPE SEND = 1 byte). Cap
/// the buffer so a hostile/broken server can't grow it unboundedly; overflow
/// bytes are dropped (the subneg is then simply not recognized).
const SB_CAP: usize = 64;

/// Options we accept the server enabling on its side (server WILL → our DO).
fn remote_ok(opt: u8) -> bool {
    matches!(opt, OPT_ECHO | OPT_SGA | OPT_BINARY)
}

/// Options we agree to enable on our side (server DO → our WILL).
fn local_ok(opt: u8) -> bool {
    matches!(opt, OPT_NAWS | OPT_TTYPE | OPT_SGA | OPT_BINARY)
}

enum NState {
    /// Plain data flow.
    Data,
    /// Saw IAC; next byte decides.
    Iac,
    /// Saw IAC WILL/WONT/DO/DONT (the u8); next byte is the option.
    Verb(u8),
    /// Saw IAC SB; next byte is the option being subnegotiated.
    SbOpt,
    /// Inside a subnegotiation, collecting payload into `sb_buf`.
    SbData,
    /// Saw IAC inside a subnegotiation (IAC IAC = literal 0xFF, IAC SE = end).
    SbIac,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NegotiationEvent {
    RemoteEcho(bool),
}

/// Pure telnet option state machine. Accepted options remember YES so repeated
/// WILL/DO do not re-trigger a reply. Unsupported requests are refused every
/// time while remaining in RFC 1143 NO; this client never initiates options, so
/// the WANTNO/WANTYES queue states are unnecessary.
pub struct Negotiator {
    state: NState,
    sb_opt: u8,
    sb_buf: Vec<u8>,
    /// remote_on[opt]: server said WILL opt and we accepted (DO sent).
    remote_on: [bool; 256],
    /// local_on[opt]: server said DO opt and we agreed (WILL sent).
    local_on: [bool; 256],
    /// Non-BINARY NVT decoding needs one byte of look-behind: CR NUL is a
    /// literal carriage return and CR LF is a newline, even when a socket read
    /// splits the pair.
    remote_pending_cr: bool,
    cols: u16,
    rows: u16,
}

impl Negotiator {
    pub fn new() -> Self {
        Self {
            state: NState::Data,
            sb_opt: 0,
            sb_buf: Vec::new(),
            remote_on: [false; 256],
            local_on: [false; 256],
            remote_pending_cr: false,
            // Standard NVT assumption until the frontend's first fit/resize.
            cols: 80,
            rows: 24,
        }
    }

    fn push_with_events(&mut self, input: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<NegotiationEvent>) {
        let mut data = Vec::with_capacity(input.len());
        let mut reply = Vec::new();
        let mut events = Vec::new();
        for &b in input {
            match self.state {
                NState::Data => match b {
                    IAC => {
                        // A Telnet command is a processing boundary. In
                        // particular, a WILL BINARY here changes how the next
                        // data byte is interpreted, so it cannot remain the
                        // partner of an earlier NVT CR.
                        if std::mem::take(&mut self.remote_pending_cr) {
                            data.push(b'\r');
                        }
                        self.state = NState::Iac;
                    }
                    _ => self.push_data_byte(b, &mut data),
                },
                NState::Iac => match b {
                    IAC => {
                        self.push_data_byte(IAC, &mut data); // escaped literal 0xFF
                        self.state = NState::Data;
                    }
                    WILL | WONT | DO | DONT => self.state = NState::Verb(b),
                    SB => self.state = NState::SbOpt,
                    // NOP / GA / AYT / BRK / … — nothing useful for a client.
                    _ => self.state = NState::Data,
                },
                NState::Verb(verb) => {
                    self.on_verb(verb, b, &mut reply, &mut events);
                    self.state = NState::Data;
                }
                NState::SbOpt => {
                    self.sb_opt = b;
                    self.sb_buf.clear();
                    self.state = NState::SbData;
                }
                NState::SbData => match b {
                    IAC => self.state = NState::SbIac,
                    _ => {
                        if self.sb_buf.len() < SB_CAP {
                            self.sb_buf.push(b);
                        }
                    }
                },
                NState::SbIac => match b {
                    IAC => {
                        // escaped 0xFF inside the subnegotiation payload
                        if self.sb_buf.len() < SB_CAP {
                            self.sb_buf.push(IAC);
                        }
                        self.state = NState::SbData;
                    }
                    SE => {
                        self.on_subneg(&mut reply);
                        self.state = NState::Data;
                    }
                    // Malformed (IAC + something else inside SB): stay lenient,
                    // treat it as the end of the subnegotiation.
                    _ => self.state = NState::Data,
                },
            }
        }
        (data, reply, events)
    }

    fn push_data_byte(&mut self, byte: u8, data: &mut Vec<u8>) {
        if self.remote_pending_cr {
            self.remote_pending_cr = false;
            match byte {
                0 => data.push(b'\r'),
                b'\n' => data.extend_from_slice(b"\r\n"),
                other => {
                    data.push(b'\r');
                    if !self.remote_on[OPT_BINARY as usize] && other == b'\r' {
                        self.remote_pending_cr = true;
                    } else {
                        data.push(other);
                    }
                }
            }
            return;
        }

        if !self.remote_on[OPT_BINARY as usize] && byte == b'\r' {
            self.remote_pending_cr = true;
        } else {
            data.push(byte);
        }
    }

    fn finish_inbound(&mut self) -> Vec<u8> {
        if std::mem::take(&mut self.remote_pending_cr) {
            vec![b'\r']
        } else {
            Vec::new()
        }
    }

    fn encode_outbound(&self, data: &[u8]) -> Vec<u8> {
        if self.local_on[OPT_BINARY as usize] {
            return escape_iac(data);
        }

        let mut nvt = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            let byte = data[i];
            nvt.push(byte);
            if byte == b'\r' {
                if data.get(i + 1) == Some(&b'\n') {
                    nvt.push(b'\n');
                    i += 1;
                } else {
                    nvt.push(0);
                }
            }
            i += 1;
        }
        escape_iac(&nvt)
    }

    fn on_verb(
        &mut self,
        verb: u8,
        opt: u8,
        reply: &mut Vec<u8>,
        events: &mut Vec<NegotiationEvent>,
    ) {
        let i = opt as usize;
        match verb {
            WILL => {
                if remote_ok(opt) {
                    if !self.remote_on[i] {
                        self.remote_on[i] = true;
                        reply.extend([IAC, DO, opt]);
                        if opt == OPT_ECHO {
                            events.push(NegotiationEvent::RemoteEcho(true));
                        }
                    }
                } else {
                    // We remain in RFC 1143 NO. A future WILL is a new request
                    // and must be refused again; "Refused until WONT" is not a
                    // protocol state.
                    reply.extend([IAC, DONT, opt]);
                }
            }
            WONT => {
                // Ack the disable only on an actual on→off transition; WONT for
                // an option that was never on needs no answer (RFC 854 forbids
                // acknowledging a non-change — that's the loop trap).
                if self.remote_on[i] {
                    reply.extend([IAC, DONT, opt]);
                    if opt == OPT_ECHO {
                        events.push(NegotiationEvent::RemoteEcho(false));
                    }
                }
                self.remote_on[i] = false;
            }
            DO => {
                if local_ok(opt) {
                    if !self.local_on[i] {
                        self.local_on[i] = true;
                        reply.extend([IAC, WILL, opt]);
                        // NAWS: the WILL is immediately followed by the current
                        // size (RFC 1073 — the client reports on activation).
                        if opt == OPT_NAWS {
                            reply.extend(naws_subneg(self.cols, self.rows));
                        }
                    }
                } else {
                    reply.extend([IAC, WONT, opt]);
                }
            }
            DONT => {
                if self.local_on[i] {
                    reply.extend([IAC, WONT, opt]);
                }
                self.local_on[i] = false;
            }
            _ => unreachable!("state machine only enters Verb for the 4 verbs"),
        }
    }

    fn on_subneg(&mut self, reply: &mut Vec<u8>) {
        // TTYPE SEND → IS "xterm-256color". The only subnegotiation a server
        // sends that we answer; everything else (incl. stray NAWS) is ignored.
        if self.local_on[OPT_TTYPE as usize]
            && self.sb_opt == OPT_TTYPE
            && self.sb_buf.as_slice() == [TTYPE_SEND]
        {
            reply.extend([IAC, SB, OPT_TTYPE, TTYPE_IS]);
            reply.extend_from_slice(TERM_TYPE);
            reply.extend([IAC, SE]);
        }
    }

    /// Record the terminal size; returns the NAWS report to send when the
    /// option is active (None before the server ever asked for NAWS).
    pub fn set_size(&mut self, cols: u16, rows: u16) -> Option<Vec<u8>> {
        self.cols = cols;
        self.rows = rows;
        self.local_on[OPT_NAWS as usize].then(|| naws_subneg(cols, rows))
    }
}

/// IAC SB NAWS <cols hi/lo> <rows hi/lo> IAC SE, with any 0xFF payload byte
/// doubled (IAC escaping applies inside subnegotiations too — a 255-column
/// window would otherwise truncate the subneg).
fn naws_subneg(cols: u16, rows: u16) -> Vec<u8> {
    let mut v = vec![IAC, SB, OPT_NAWS];
    for b in [(cols >> 8) as u8, cols as u8, (rows >> 8) as u8, rows as u8] {
        v.push(b);
        if b == IAC {
            v.push(IAC);
        }
    }
    v.extend([IAC, SE]);
    v
}

/// Escape outbound user data: 0xFF must go on the wire as IAC IAC. NVT CR
/// mapping is applied separately according to the negotiated BINARY state;
/// this helper is also used for already-binary payloads.
fn escape_iac(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for &b in data {
        out.push(b);
        if b == IAC {
            out.push(IAC);
        }
    }
    out
}

/// Trips the shared close flag when the last `TelnetHandle` clone is dropped —
/// same mechanism as serial's guard: dropping the writer stream does NOT close
/// the reader's `try_clone`d socket, so the reader thread must be told to stop.
/// It checks the flag once per read cycle (≤ the 100ms read timeout).
struct CloseGuard {
    closed: Arc<AtomicBool>,
}

impl Drop for CloseGuard {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

/// Telnet session handle. Clone + Send + Sync, like `SerialHandle`. Unlike
/// serial it HAS resize: NAWS reports rows/cols to the server.
#[derive(Clone)]
pub struct TelnetHandle {
    writer: Arc<Mutex<TcpStream>>,
    /// Shared with the reader thread, which feeds inbound bytes through it.
    neg: Arc<Mutex<Negotiator>>,
    /// Last clone dropped → guard drops → close flag set → reader exits.
    _guard: Arc<CloseGuard>,
    /// "host:port" as the user typed it — AI session scope + labels.
    peer: Arc<str>,
    /// Profile-owned line ending used by command-oriented writers such as AI.
    /// Raw terminal writes still pass bytes through unchanged.
    input_newline: Arc<[u8]>,
}

fn telnet_op_err(e: impl std::fmt::Display) -> AppError {
    AppError::pty(
        "telnet_op_failed",
        serde_json::json!({ "err": e.to_string() }),
    )
}

impl TelnetHandle {
    pub fn write(&self, data: &[u8]) -> AppResult<()> {
        // Same neg -> writer lock order as resize and the reader thread. The
        // negotiated BINARY state and the bytes encoded from it are therefore
        // one observation: a concurrent DO/DONT cannot overtake this write.
        let neg = locked(&self.neg)?;
        let encoded = neg.encode_outbound(data);
        locked(&self.writer)?
            .write_all(&encoded)
            .map_err(telnet_op_err)
    }

    /// Report a new window size via NAWS. A no-op until the server activates
    /// the option (DO NAWS) — the size is remembered and reported then.
    ///
    /// The neg lock is held across the write (neg → writer, same order as the
    /// reader thread): released between them, this report could slip in front
    /// of the reader's in-flight activation reply and the server would keep
    /// the stale activation size instead of this one.
    pub fn resize(&self, cols: u16, rows: u16) -> AppResult<()> {
        let mut neg = locked(&self.neg)?;
        match neg.set_size(cols, rows) {
            Some(bytes) => locked(&self.writer)?
                .write_all(&bytes)
                .map_err(telnet_op_err),
            None => Ok(()),
        }
    }

    /// "host:port" this session was opened against.
    pub fn peer(&self) -> &str {
        &self.peer
    }

    pub fn write_line(&self, text: &str) -> AppResult<()> {
        let mut data = Vec::with_capacity(text.len() + self.input_newline.len());
        data.extend_from_slice(text.as_bytes());
        data.extend_from_slice(&self.input_newline);
        self.write(&data)
    }
}

fn open_err(peer: &str, e: impl std::fmt::Display) -> AppError {
    AppError::pty(
        "telnet_open_failed",
        serde_json::json!({ "peer": peer, "err": e.to_string() }),
    )
}

/// Resolve, then try each address with a 10s connect timeout. NOT bounded
/// overall: `to_socket_addrs` blocks on the system resolver (its own timeout
/// policy), and a multi-A/AAAA host pays up to 10s per address. That's fine —
/// the whole call runs on a blocking worker (spawn_blocking in both callers),
/// so a slow resolve delays only this open, never the UI or event loop.
fn connect(host: &str, port: u16) -> AppResult<TcpStream> {
    let peer = format!("{host}:{port}");
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| open_err(&peer, e))?
        .collect();
    let mut last: Option<std::io::Error> = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        Some(e) => open_err(&peer, e),
        None => open_err(&peer, "no addresses resolved"),
    })
}

/// Connect to `host:port` and spawn a reader thread that negotiates telnet
/// options and pushes terminal bytes to `sink`. Returns `(session_id, handle)`.
///
/// `(cols, rows)` seeds the negotiator so the NAWS *activation* reply already
/// carries the caller's real terminal size (same contract as `ssh_connect`) —
/// the server's DO NAWS usually arrives before the frontend's first fit/resize
/// round-trip, and without the seed that reply would report the 80x24 default.
/// `input_newline` is retained by the handle for command-oriented `write_line`.
pub fn open(
    session_id: String,
    host: &str,
    port: u16,
    cols: u16,
    rows: u16,
    input_newline: &str,
    sink: TelnetSink,
) -> AppResult<(String, TelnetHandle)> {
    let input_newline: Arc<[u8]> = match input_newline {
        "cr" => Arc::from(&b"\r"[..]),
        "lf" => Arc::from(&b"\n"[..]),
        "crlf" => Arc::from(&b"\r\n"[..]),
        _ => {
            return Err(AppError::config(
                "telnet_profile_invalid",
                serde_json::json!({ "field": "input_newline" }),
            ));
        }
    };
    let peer = format!("{host}:{port}");
    let stream = connect(host, port)?;
    // Interactive session: a keystroke per packet beats Nagle batching.
    let _ = stream.set_nodelay(true);
    // Bound writes too: without this, a peer that stops reading (device hang,
    // dead link with no RST) eventually fills the send buffer and write_all
    // blocks forever — inside a synchronous Tauri command, i.e. a frozen UI.
    // 5s is generous for a live link; a link that can't drain a keystroke in
    // 5s is dead and the error surfaces to the terminal instead.
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(telnet_op_err)?;

    // Separate read handle so the blocking read doesn't hold the writer's lock.
    // The 100ms read timeout paces the loop so it notices the close flag; NOT a
    // data deadline — on timeout we just loop again (same as serial).
    let reader = stream.try_clone().map_err(telnet_op_err)?;
    reader
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(telnet_op_err)?;

    let closed = Arc::new(AtomicBool::new(false));
    let reader_closed = closed.clone();

    let mut negotiator = Negotiator::new();
    // Pre-activation set_size returns None by contract — nothing on the wire,
    // the size is simply remembered for the activation reply.
    let _ = negotiator.set_size(cols, rows);

    let id = session_id;
    let handle = TelnetHandle {
        writer: Arc::new(Mutex::new(stream)),
        neg: Arc::new(Mutex::new(negotiator)),
        _guard: Arc::new(CloseGuard { closed }),
        peer: Arc::from(peer.as_str()),
        input_newline,
    };

    // Reader thread: socket RX → negotiator → (replies back down the socket,
    // data to the sink). Exits on close flag, on EOF (peer closed — unlike
    // serial, Ok(0) on TCP is a real close), and on fatal IO error; emits
    // Close on the way out so the frontend can mark the tab.
    let sid = id.clone();
    let neg = handle.neg.clone();
    let writer = handle.writer.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            if reader_closed.load(Ordering::Relaxed) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => break, // TCP EOF — peer closed the connection
                Ok(n) => {
                    // neg is held across the reply write (neg → writer, same
                    // order as resize): a concurrent resize() can't interleave
                    // its NAWS report ahead of the activation reply it belongs
                    // after. Same order both sides = no deadlock.
                    let (data, events) = {
                        let mut neg = match neg.lock() {
                            Ok(g) => g,
                            Err(_) => break, // poisoned — a peer panicked; give up
                        };
                        let (data, reply, events) = neg.push_with_events(&buf[..n]);
                        if !reply.is_empty() {
                            // Protocol replies are pre-formed bytes — no IAC escaping.
                            let sent = writer.lock().map(|mut w| w.write_all(&reply));
                            if !matches!(sent, Ok(Ok(()))) {
                                break;
                            }
                        }
                        (data, events)
                    };
                    for event in events {
                        match event {
                            NegotiationEvent::RemoteEcho(enabled) => {
                                sink(&sid, TelnetOut::RemoteEcho(enabled));
                            }
                        }
                    }
                    if !data.is_empty() {
                        sink(&sid, TelnetOut::Data(data));
                    }
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    continue
                }
                Err(_) => break, // connection reset / fatal IO error
            }
        }
        // A malformed/abrupt peer may end after a lone CR. Preserve that byte
        // rather than silently losing terminal output at EOF.
        let trailing = neg
            .lock()
            .map(|mut negotiator| negotiator.finish_inbound())
            .unwrap_or_default();
        if !trailing.is_empty() {
            sink(&sid, TelnetOut::Data(trailing));
        }
        sink(&sid, TelnetOut::Close);
    });

    Ok((id, handle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed one inbound byte-string, expect (data, reply).
    fn push(neg: &mut Negotiator, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let (data, reply, _) = neg.push_with_events(input);
        (data, reply)
    }

    fn open(
        host: &str,
        port: u16,
        cols: u16,
        rows: u16,
        sink: TelnetSink,
    ) -> AppResult<(String, TelnetHandle)> {
        super::open(
            uuid::Uuid::new_v4().to_string(),
            host,
            port,
            cols,
            rows,
            "crlf",
            sink,
        )
    }

    #[test]
    fn plain_data_passes_through() {
        let mut n = Negotiator::new();
        let (data, reply) = push(&mut n, b"login: ");
        assert_eq!(data, b"login: ");
        assert!(reply.is_empty());
    }

    #[test]
    fn iac_iac_is_literal_ff() {
        let mut n = Negotiator::new();
        let (data, reply) = push(&mut n, &[b'a', IAC, IAC, b'b']);
        assert_eq!(data, [b'a', 0xFF, b'b']);
        assert!(reply.is_empty());
    }

    #[test]
    fn will_echo_answered_do_once() {
        let mut n = Negotiator::new();
        let (_, reply, events) = n.push_with_events(&[IAC, WILL, OPT_ECHO]);
        assert_eq!(reply, [IAC, DO, OPT_ECHO]);
        assert_eq!(events, [NegotiationEvent::RemoteEcho(true)]);
        // Repeated WILL for an already-on option: no answer (loop prevention).
        let (_, reply, events) = n.push_with_events(&[IAC, WILL, OPT_ECHO]);
        assert!(reply.is_empty());
        assert!(events.is_empty());

        let (_, reply, events) = n.push_with_events(&[IAC, WONT, OPT_ECHO]);
        assert_eq!(reply, [IAC, DONT, OPT_ECHO]);
        assert_eq!(events, [NegotiationEvent::RemoteEcho(false)]);

        let (_, reply, events) = n.push_with_events(&[IAC, WILL, OPT_ECHO]);
        assert_eq!(reply, [IAC, DO, OPT_ECHO]);
        assert_eq!(events, [NegotiationEvent::RemoteEcho(true)]);
    }

    #[test]
    fn will_unknown_option_refused() {
        let mut n = Negotiator::new();
        // 32 = TSPEED — not in our accept set.
        let (_, reply) = push(&mut n, &[IAC, WILL, 32]);
        assert_eq!(reply, [IAC, DONT, 32]);
    }

    #[test]
    fn do_unknown_option_refused() {
        let mut n = Negotiator::new();
        let (_, reply) = push(&mut n, &[IAC, DO, 32]);
        assert_eq!(reply, [IAC, WONT, 32]);
    }

    #[test]
    fn repeated_unknown_option_request_is_refused_each_time() {
        let mut n = Negotiator::new();
        let (_, first) = push(&mut n, &[IAC, WILL, 32]);
        let (_, repeated) = push(&mut n, &[IAC, WILL, 32]);
        assert_eq!(first, [IAC, DONT, 32]);
        assert_eq!(repeated, [IAC, DONT, 32]);

        let (_, first) = push(&mut n, &[IAC, DO, 33]);
        let (_, repeated) = push(&mut n, &[IAC, DO, 33]);
        assert_eq!(first, [IAC, WONT, 33]);
        assert_eq!(repeated, [IAC, WONT, 33]);
    }

    #[test]
    fn inbound_nvt_cr_mapping_survives_read_boundaries() {
        let mut n = Negotiator::new();
        let (data, reply) = push(&mut n, b"a\r");
        assert_eq!(data, b"a");
        assert!(reply.is_empty());

        let (data, _) = push(&mut n, b"\0b\r");
        assert_eq!(data, b"\rb");
        let (data, _) = push(&mut n, b"\nc");
        assert_eq!(data, b"\r\nc");
    }

    #[test]
    fn inbound_iac_command_ends_pending_cr_before_binary_mode() {
        let mut n = Negotiator::new();
        let (data, reply) = push(&mut n, &[b'\r', IAC, WILL, OPT_BINARY, b'\0']);

        assert_eq!(reply, [IAC, DO, OPT_BINARY]);
        assert_eq!(data, b"\r\0");
    }

    #[test]
    fn inbound_binary_mode_preserves_cr_nul() {
        let mut n = Negotiator::new();
        let (_, reply) = push(&mut n, &[IAC, WILL, OPT_BINARY]);
        assert_eq!(reply, [IAC, DO, OPT_BINARY]);

        let (data, _) = push(&mut n, b"\r\0");
        assert_eq!(data, b"\r\0");
    }

    #[test]
    fn inbound_lone_cr_is_flushed_at_eof() {
        let mut n = Negotiator::new();
        let (data, _) = push(&mut n, b"tail\r");
        assert_eq!(data, b"tail");
        assert_eq!(n.finish_inbound(), b"\r");
        assert!(n.finish_inbound().is_empty());
    }

    #[test]
    fn outbound_nvt_mapping_and_iac_escaping_follow_binary_state() {
        let mut n = Negotiator::new();
        assert_eq!(n.encode_outbound(b"a\rb\r\n\xff"), b"a\r\0b\r\n\xff\xff",);

        let (_, reply) = push(&mut n, &[IAC, DO, OPT_BINARY]);
        assert_eq!(reply, [IAC, WILL, OPT_BINARY]);
        assert_eq!(n.encode_outbound(b"\r\0\xff"), b"\r\0\xff\xff");
    }

    #[test]
    fn do_naws_answers_will_plus_size_report() {
        let mut n = Negotiator::new();
        let (_, reply) = push(&mut n, &[IAC, DO, OPT_NAWS]);
        let mut want = vec![IAC, WILL, OPT_NAWS];
        want.extend(naws_subneg(80, 24)); // default size before any resize
        assert_eq!(reply, want);
    }

    #[test]
    fn set_size_before_activation_is_remembered() {
        let mut n = Negotiator::new();
        // Frontend fits the terminal before the server negotiates.
        assert_eq!(n.set_size(120, 40), None);
        let (_, reply) = push(&mut n, &[IAC, DO, OPT_NAWS]);
        let mut want = vec![IAC, WILL, OPT_NAWS];
        want.extend(naws_subneg(120, 40));
        assert_eq!(reply, want);
        // After activation, resize reports immediately.
        assert_eq!(n.set_size(132, 43), Some(naws_subneg(132, 43)));
    }

    #[test]
    fn naws_subneg_escapes_iac_payload_bytes() {
        // 255 columns: the low byte is 0xFF and must be doubled on the wire.
        let got = naws_subneg(255, 24);
        assert_eq!(got, [IAC, SB, OPT_NAWS, 0, IAC, IAC, 0, 24, IAC, SE]);
    }

    #[test]
    fn ttype_send_answered_with_xterm() {
        let mut n = Negotiator::new();
        let (_, reply) = push(&mut n, &[IAC, DO, OPT_TTYPE]);
        assert_eq!(reply, [IAC, WILL, OPT_TTYPE]);
        let (data, reply) = push(&mut n, &[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
        assert!(data.is_empty());
        let mut want = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        want.extend_from_slice(b"xterm-256color");
        want.extend([IAC, SE]);
        assert_eq!(reply, want);
    }

    #[test]
    fn ttype_send_before_activation_is_ignored() {
        let mut n = Negotiator::new();
        let (data, reply) = push(&mut n, &[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
        assert!(data.is_empty());
        assert!(reply.is_empty());
    }

    #[test]
    fn unknown_subneg_ignored() {
        let mut n = Negotiator::new();
        let (data, reply) = push(&mut n, &[IAC, SB, 32, 1, 2, 3, IAC, SE, b'x']);
        assert_eq!(data, b"x"); // stream resumes cleanly after the subneg
        assert!(reply.is_empty());
    }

    #[test]
    fn negotiation_interleaved_with_data() {
        let mut n = Negotiator::new();
        let mut input = vec![b'a', b'b'];
        input.extend([IAC, WILL, OPT_SGA]);
        input.extend(b"cd");
        input.extend([IAC, DO, OPT_SGA]);
        input.extend(b"ef");
        let (data, reply) = push(&mut n, &input);
        assert_eq!(data, b"abcdef");
        assert_eq!(reply, [IAC, DO, OPT_SGA, IAC, WILL, OPT_SGA]);
    }

    #[test]
    fn sequences_split_across_reads_reassemble() {
        // IAC WILL ECHO delivered one byte per read — state must persist.
        let mut n = Negotiator::new();
        assert_eq!(push(&mut n, &[IAC]), (vec![], vec![]));
        assert_eq!(push(&mut n, &[WILL]), (vec![], vec![]));
        let (_, reply) = push(&mut n, &[OPT_ECHO]);
        assert_eq!(reply, [IAC, DO, OPT_ECHO]);
    }

    #[test]
    fn wont_after_will_acked_dont_and_only_on_transition() {
        let mut n = Negotiator::new();
        push(&mut n, &[IAC, WILL, OPT_ECHO]);
        let (_, reply) = push(&mut n, &[IAC, WONT, OPT_ECHO]);
        assert_eq!(reply, [IAC, DONT, OPT_ECHO]);
        // WONT for an option that was never on: silence, not an ack.
        let (_, reply) = push(&mut n, &[IAC, WONT, OPT_ECHO]);
        assert!(reply.is_empty());
        let (_, reply) = push(&mut n, &[IAC, WONT, OPT_TTYPE]);
        assert!(reply.is_empty());
    }

    #[test]
    fn oversized_subneg_payload_is_capped_not_grown() {
        let mut n = Negotiator::new();
        let mut input = vec![IAC, SB, OPT_TTYPE];
        input.extend(std::iter::repeat_n(b'x', SB_CAP * 4));
        input.extend([IAC, SE]);
        let (data, reply) = push(&mut n, &input);
        assert!(data.is_empty());
        assert!(reply.is_empty()); // not a valid TTYPE SEND → no answer
        assert!(n.sb_buf.len() <= SB_CAP);
    }

    #[test]
    fn escape_iac_doubles_ff_only() {
        assert_eq!(escape_iac(b"abc"), b"abc");
        assert_eq!(escape_iac(&[0xFF]), [0xFF, 0xFF]);
        assert_eq!(escape_iac(&[1, 0xFF, 2]), [1, 0xFF, 0xFF, 2]);
    }

    // ── loopback integration: a scripted telnet server on 127.0.0.1 ──

    use std::net::TcpListener;
    use std::sync::mpsc;

    /// Read from `s` until `want` bytes arrived (or panic after ~5s).
    fn read_exact_timeout(s: &mut TcpStream, want: usize) -> Vec<u8> {
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut got = vec![0u8; want];
        let mut n = 0;
        while n < want {
            match s.read(&mut got[n..]) {
                Ok(0) => panic!("peer closed after {n}/{want} bytes"),
                Ok(k) => n += k,
                Err(e) => panic!("read failed after {n}/{want} bytes: {e}"),
            }
        }
        got
    }

    #[test]
    fn loopback_session_negotiates_relays_and_closes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            // Typical telnetd opener: negotiate, then banner.
            s.write_all(&[IAC, DO, OPT_NAWS, IAC, WILL, OPT_ECHO])
                .unwrap();
            s.write_all(b"login: ").unwrap();
            // Client must answer WILL NAWS + NAWS report + DO ECHO. The report
            // must carry the size `open()` was seeded with (97x31, deliberately
            // non-default) — NOT the 80x24 NVT fallback.
            let mut want = vec![IAC, WILL, OPT_NAWS];
            want.extend(naws_subneg(97, 31));
            want.extend([IAC, DO, OPT_ECHO]);
            let got = read_exact_timeout(&mut s, want.len());
            assert_eq!(got, want);
            // User types; 0xFF arrives doubled and \r\n arrives verbatim.
            let got = read_exact_timeout(&mut s, 8);
            assert_eq!(got, [b'r', b'o', b'o', b't', 0xFF, 0xFF, b'\r', b'\n']);
            s // keep the socket alive until the test drops the handle
        });

        let (tx, rx) = mpsc::channel::<TelnetOut>();
        let sink: TelnetSink = Arc::new(move |_id, out| {
            let _ = tx.send(out);
        });
        let (id, handle) = open("127.0.0.1", port, 97, 31, sink).unwrap();
        assert!(!id.is_empty());
        assert_eq!(handle.peer(), format!("127.0.0.1:{port}"));

        // Banner comes through with IAC negotiation stripped.
        let mut banner = Vec::new();
        let mut saw_remote_echo = false;
        while banner.len() < 7 {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                TelnetOut::Data(b) => banner.extend(b),
                TelnetOut::RemoteEcho(enabled) => saw_remote_echo = enabled,
                TelnetOut::Close => panic!("closed before banner complete"),
            }
        }
        assert_eq!(banner, b"login: ");
        assert!(saw_remote_echo);

        // Write with an embedded 0xFF — must be escaped on the wire.
        handle
            .write(&[b'r', b'o', b'o', b't', 0xFF, b'\r', b'\n'])
            .unwrap();

        let server_sock = server.join().unwrap();

        // Dropping the last handle clone trips the CloseGuard; reader exits and
        // emits Close within a read cycle.
        drop(handle);
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                TelnetOut::Close => break,
                TelnetOut::Data(_) => continue,
                TelnetOut::RemoteEcho(_) => continue,
            }
        }
        drop(server_sock);
    }

    #[test]
    fn loopback_close_emitted_on_server_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(b"bye").unwrap();
            // socket drops here → EOF on the client
        });

        let (tx, rx) = mpsc::channel::<TelnetOut>();
        let sink: TelnetSink = Arc::new(move |_id, out| {
            let _ = tx.send(out);
        });
        let (_id, _handle) = open("127.0.0.1", port, 80, 24, sink).unwrap();
        // Data then Close, in order.
        let mut saw_data = false;
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                TelnetOut::Data(b) => {
                    assert_eq!(b, b"bye");
                    saw_data = true;
                }
                TelnetOut::Close => break,
                TelnetOut::RemoteEcho(_) => continue,
            }
        }
        assert!(saw_data);
    }

    #[test]
    fn connect_refused_maps_to_open_failed() {
        // Bind + drop to get a port that's very likely closed.
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let sink: TelnetSink = Arc::new(|_id, _out| {});
        let err = match open("127.0.0.1", port, 80, 24, sink) {
            Err(e) => e,
            Ok(_) => panic!("connect to a closed port unexpectedly succeeded"),
        };
        assert_eq!(err.code(), "telnet_open_failed");
    }

    #[test]
    fn write_line_uses_profile_input_newline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_exact_timeout(&mut stream, 5)
        });
        let sink: TelnetSink = Arc::new(|_id, _out| {});
        let (_id, handle) = super::open(
            uuid::Uuid::new_v4().to_string(),
            "127.0.0.1",
            port,
            80,
            24,
            "lf",
            sink,
        )
        .unwrap();

        handle.write_line("show").unwrap();

        assert_eq!(server.join().unwrap(), b"show\n");
    }

    #[test]
    fn invalid_input_newline_fails_before_connect() {
        let sink: TelnetSink = Arc::new(|_id, _out| {});
        let err = match super::open(
            uuid::Uuid::new_v4().to_string(),
            "not-resolved.invalid",
            23,
            80,
            24,
            "wat",
            sink,
        ) {
            Ok(_) => panic!("invalid newline unexpectedly opened a connection"),
            Err(err) => err,
        };
        assert_eq!(err.code(), "telnet_profile_invalid");
    }

    #[test]
    fn resize_before_naws_is_silent_after_naws_reports() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Deterministic sequencing: the server stays silent until the client's
        // probe byte, so the first resize() is GUARANTEED to land before NAWS
        // activation; a marker byte after the report gates the second resize.
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            // 1. Wait for the probe — client has already resized to 100x30.
            assert_eq!(read_exact_timeout(&mut s, 1), b"x");
            // 2. Activate NAWS; the report must carry the stored 100x30.
            s.write_all(&[IAC, DO, OPT_NAWS]).unwrap();
            let mut want = vec![IAC, WILL, OPT_NAWS];
            want.extend(naws_subneg(100, 30));
            assert_eq!(read_exact_timeout(&mut s, want.len()), want);
            // 3. Tell the client negotiation is done.
            s.write_all(b"k").unwrap();
            // 4. Explicit resize after activation.
            let want = naws_subneg(120, 40);
            assert_eq!(read_exact_timeout(&mut s, want.len()), want);
            s
        });

        let (tx, rx) = mpsc::channel::<TelnetOut>();
        let sink: TelnetSink = Arc::new(move |_id, out| {
            let _ = tx.send(out);
        });
        let (_id, handle) = open("127.0.0.1", port, 80, 24, sink).unwrap();
        // Fit before any negotiation: must be silent (nothing on the wire, or
        // the server's step-1 read would see it instead of the probe).
        handle.resize(100, 30).unwrap();
        handle.write(b"x").unwrap();
        // Wait for the server's "negotiation done" marker.
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                TelnetOut::Data(b) if b == b"k" => break,
                TelnetOut::Data(_) => continue,
                TelnetOut::RemoteEcho(_) => continue,
                TelnetOut::Close => panic!("closed before negotiation marker"),
            }
        }
        handle.resize(120, 40).unwrap();
        let server_sock = server.join().unwrap();
        drop(server_sock);
        // Drain until Close (server socket dropped → EOF).
        loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                TelnetOut::Close => break,
                TelnetOut::Data(_) => continue,
                TelnetOut::RemoteEcho(_) => continue,
            }
        }
    }
}
