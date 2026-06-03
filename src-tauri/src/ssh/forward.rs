use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use russh::Disconnect;
use serde::Serialize;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use crate::error::{locked, AppError, AppResult};
use crate::models::Profile;
use crate::models::{Credential, Forward};
use crate::ssh::client::{self, LogFn};
use std::sync::Arc as StdArc;

/// Everything needed to open an authenticated SSH connection to a forward's
/// target host through its bastion chain. `start_local` / `start_remote` /
/// `start_dynamic` all took these six parameters identically — they are one
/// concept ("how to reach the endpoint"), so they travel as one value.
pub struct ConnTarget {
    pub host: String,
    pub port: u16,
    pub credential: Credential,
    pub bastion_chain: Vec<(Profile, Credential)>,
    pub known_hosts_path: PathBuf,
    pub timeout_secs: u64,
}

pub struct ForwardHandle {
    abort: tokio::task::AbortHandle,
    /// Notify-based disconnect signal. `stop()` fires this; the accept-loop
    /// task `select!`s on it and runs `handle.disconnect(...)` before
    /// breaking out. Without this, abort()ing the task drops the future
    /// holding the SSH `Handle` — russh never sends the `SSH_MSG_DISCONNECT`
    /// and the server leaks a half-open session until TCP keepalive expires.
    disconnect: Arc<Notify>,
    pub bytes_tx: Arc<AtomicU64>,
    pub bytes_rx: Arc<AtomicU64>,
    pub connections: Arc<AtomicU32>,
}

impl ForwardHandle {
    pub fn stop(&self) {
        self.disconnect.notify_one();
        // Give the task up to 2 s to send the disconnect message before
        // we force-abort. Picked over a hard sync-wait so `stop()` stays
        // non-blocking for the Tauri command thread; picked over no abort
        // at all so a wedged disconnect await (e.g. dead remote) can't
        // strand the forward task forever.
        let abort = self.abort.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            abort.abort();
        });
    }
}

#[derive(Serialize)]
pub struct ForwardStats {
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    pub connections: u32,
}

/// Bind a local-forward listener on loopback for both families. IPv4
/// (127.0.0.1) is required; IPv6 (::1) is best-effort and simply skipped on
/// hosts without an IPv6 stack. Both stay loopback-only — a local forward is
/// never exposed to the network.
async fn bind_loopback(port: u16) -> AppResult<(TcpListener, Option<TcpListener>)> {
    let v4 = TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| {
        AppError::ssh("ssh_port_bind_failed", json!({ "port": port, "err": e.to_string() }))
    })?;
    // Bind v6 to v4's *actual* port so an ephemeral request (port 0) lands on
    // the same port for both families. If v4's port can't be read, skip v6
    // rather than risk binding it to a different (e.g. random) port.
    let v6 = match v4.local_addr() {
        Ok(a) => TcpListener::bind(("::1", a.port())).await.ok(),
        Err(_) => None,
    };
    Ok((v4, v6))
}

/// `accept()` on an optional listener; pends forever when the listener is
/// absent so it can sit in a `select!` arm that never fires.
async fn accept_opt(
    listener: &Option<TcpListener>,
) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    match listener {
        Some(l) => l.accept().await,
        None => std::future::pending().await,
    }
}

impl ForwardHandle {
    pub fn stats(&self) -> ForwardStats {
        ForwardStats {
            bytes_tx: self.bytes_tx.load(Ordering::Relaxed),
            bytes_rx: self.bytes_rx.load(Ordering::Relaxed),
            connections: self.connections.load(Ordering::Relaxed),
        }
    }
}

async fn counted_copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    reader: &mut R,
    writer: &mut W,
    counter: &AtomicU64,
) -> std::io::Result<()> {
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).await?;
        counter.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(())
}

/// Open an authenticated SSH connection to a forward's endpoint through its
/// bastion chain. `start_local` / `start_remote` / `start_dynamic` all opened
/// the connection with this exact prologue; keeping the dial policy
/// (known_hosts / timeout / log / prompt-ctx) in one place stops the three
/// from drifting apart. `start_remote` is the only caller that uses the
/// returned `ForwardedChannelSender`; the others discard it.
async fn connect_authed(
    target: ConnTarget,
) -> AppResult<(russh::client::Handle<client::SshHandler>, client::ForwardedChannelSender)> {
    let ConnTarget { host, port, credential, bastion_chain, known_hosts_path, timeout_secs } =
        target;
    let log: LogFn = StdArc::new(|_: String| ());
    let (mut handle, fwd_sender) = client::establish_via_chain(
        bastion_chain,
        host,
        port,
        known_hosts_path,
        timeout_secs,
        log,
        None,
    )
    .await?;
    client::authenticate(&mut handle, credential, None).await?;
    Ok((handle, fwd_sender))
}

pub async fn start_local(forward: Forward, target: ConnTarget) -> AppResult<ForwardHandle> {
    let (mut handle, _fwd) = connect_authed(target).await?;

    let remote_host = forward.remote_host.clone();
    let remote_port = forward.remote_port;
    let local_port = forward.local_port;

    let (listener, listener6) = bind_loopback(local_port).await?;

    let bytes_tx = Arc::new(AtomicU64::new(0));
    let bytes_rx = Arc::new(AtomicU64::new(0));
    let connections = Arc::new(AtomicU32::new(0));

    let tx = bytes_tx.clone();
    let rx = bytes_rx.clone();
    let conns = connections.clone();
    let disconnect = Arc::new(Notify::new());
    let disconnect_task = disconnect.clone();

    let task = tokio::spawn(async move {
        loop {
            // Accept from either loopback family; handling is identical.
            let tcp_stream = tokio::select! {
                _ = disconnect_task.notified() => break,
                res = listener.accept() => match res { Ok((s, _)) => s, Err(_) => break },
                res = accept_opt(&listener6) => match res { Ok((s, _)) => s, Err(_) => break },
            };

            let rh = remote_host.clone();
            let channel = match handle
                .channel_open_direct_tcpip(&rh, remote_port as u32, "127.0.0.1", local_port as u32)
                .await
            {
                Ok(c) => c,
                Err(_) => continue,
            };

            let stream = channel.into_stream();
            let c_tx = tx.clone();
            let c_rx = rx.clone();
            let c_conns = conns.clone();

            c_conns.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                let (mut tcp_r, mut tcp_w) = tokio::io::split(tcp_stream);
                let (mut ssh_r, mut ssh_w) = tokio::io::split(stream);
                let _ = tokio::join!(
                    counted_copy(&mut tcp_r, &mut ssh_w, &c_tx),
                    counted_copy(&mut ssh_r, &mut tcp_w, &c_rx),
                );
                c_conns.fetch_sub(1, Ordering::Relaxed);
            });
        }
        // Tell the server we're done so it can free the session immediately
        // instead of waiting on TCP keepalive. Errors are deliberately
        // swallowed — the connection may already be torn down, in which
        // case the disconnect message is moot.
        let _ = handle
            .disconnect(Disconnect::ByApplication, "rssh forward stopped", "")
            .await;
    });

    Ok(ForwardHandle {
        abort: task.abort_handle(),
        disconnect,
        bytes_tx,
        bytes_rx,
        connections,
    })
}

// ---------------------------------------------------------------------------
// Remote port forwarding
// ---------------------------------------------------------------------------

pub async fn start_remote(forward: Forward, target: ConnTarget) -> AppResult<ForwardHandle> {
    let (mut handle, fwd_sender) = connect_authed(target).await?;

    // Register a channel to receive forwarded connections from the Handler
    let (ch_tx, mut ch_rx) = tokio::sync::mpsc::unbounded_channel();
    {
        let mut guard = locked(&fwd_sender)?;
        *guard = Some(ch_tx);
    }

    // Ask the server to listen on remote_port. Empty bind address = all
    // address families (RFC 4254 §7.1), so the forward is reachable over both
    // IPv4 and IPv6 (subject to the server's GatewayPorts policy).
    let _bound_port = handle
        .tcpip_forward("", forward.remote_port as u32)
        .await
        .map_err(|e| AppError::ssh("ssh_tcpip_forward_failed", json!({ "err": e.to_string() })))?;

    let local_host = forward.remote_host.clone();
    let local_port = forward.local_port;

    let bytes_tx = Arc::new(AtomicU64::new(0));
    let bytes_rx = Arc::new(AtomicU64::new(0));
    let connections = Arc::new(AtomicU32::new(0));

    let tx = bytes_tx.clone();
    let rx = bytes_rx.clone();
    let conns = connections.clone();
    let disconnect = Arc::new(Notify::new());
    let disconnect_task = disconnect.clone();

    let task = tokio::spawn(async move {
        // Hold `handle` so dropping the future doesn't kill the SSH session
        // before the disconnect message goes out.
        let handle = handle;
        loop {
            tokio::select! {
                _ = disconnect_task.notified() => break,
                msg = ch_rx.recv() => {
                    let Some(channel) = msg else { break };
                    let lh = local_host.clone();
                    let c_tx = tx.clone();
                    let c_rx = rx.clone();
                    let c_conns = conns.clone();

                    c_conns.fetch_add(1, Ordering::Relaxed);
                    tokio::spawn(async move {
                        let local = match TcpStream::connect((lh.as_str(), local_port)).await {
                            Ok(s) => s,
                            Err(_) => {
                                c_conns.fetch_sub(1, Ordering::Relaxed);
                                return;
                            }
                        };
                        let ssh_stream = channel.into_stream();
                        let (mut tcp_r, mut tcp_w) = tokio::io::split(local);
                        let (mut ssh_r, mut ssh_w) = tokio::io::split(ssh_stream);
                        let _ = tokio::join!(
                            counted_copy(&mut tcp_r, &mut ssh_w, &c_tx),
                            counted_copy(&mut ssh_r, &mut tcp_w, &c_rx),
                        );
                        c_conns.fetch_sub(1, Ordering::Relaxed);
                    });
                }
            }
        }
        // Final disconnect — also tries to cancel the remote port forward.
        // Server-side: `tcpip_forward` registration is released when the
        // session ends, so an explicit disconnect is the cleanest signal.
        let _ = handle
            .disconnect(Disconnect::ByApplication, "rssh forward stopped", "")
            .await;
    });

    Ok(ForwardHandle {
        abort: task.abort_handle(),
        disconnect,
        bytes_tx,
        bytes_rx,
        connections,
    })
}

// ---------------------------------------------------------------------------
// Dynamic SOCKS5 forwarding
// ---------------------------------------------------------------------------

/// Parse a SOCKS5 connection request and return (target_host, target_port).
async fn socks5_handshake(stream: &mut TcpStream) -> std::io::Result<(String, u16)> {
    // 1. Read greeting: version + nmethods + methods
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await?;
    if header[0] != 0x05 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Not SOCKS5",
        ));
    }
    let nmethods = header[1] as usize;
    let mut methods = vec![0u8; nmethods];
    stream.read_exact(&mut methods).await?;

    // 2. Reply: no auth required
    stream.write_all(&[0x05, 0x00]).await?;

    // 3. Read connect request: ver(1) + cmd(1) + rsv(1) + atyp(1) + addr + port(2)
    let mut req = [0u8; 4];
    stream.read_exact(&mut req).await?;
    if req[0] != 0x05 || req[1] != 0x01 {
        // Only CONNECT (0x01) supported
        stream
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Only CONNECT supported",
        ));
    }

    let (host, port) = match req[3] {
        0x01 => {
            // IPv4
            let mut addr = [0u8; 4];
            stream.read_exact(&mut addr).await?;
            let mut port_buf = [0u8; 2];
            stream.read_exact(&mut port_buf).await?;
            let host = format!("{}.{}.{}.{}", addr[0], addr[1], addr[2], addr[3]);
            let port = u16::from_be_bytes(port_buf);
            (host, port)
        }
        0x03 => {
            // Domain name
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut domain = vec![0u8; len[0] as usize];
            stream.read_exact(&mut domain).await?;
            let mut port_buf = [0u8; 2];
            stream.read_exact(&mut port_buf).await?;
            let host = String::from_utf8_lossy(&domain).to_string();
            let port = u16::from_be_bytes(port_buf);
            (host, port)
        }
        0x04 => {
            // IPv6
            let mut addr = [0u8; 16];
            stream.read_exact(&mut addr).await?;
            let mut port_buf = [0u8; 2];
            stream.read_exact(&mut port_buf).await?;
            let host = format!(
                "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
                u16::from_be_bytes([addr[0], addr[1]]),
                u16::from_be_bytes([addr[2], addr[3]]),
                u16::from_be_bytes([addr[4], addr[5]]),
                u16::from_be_bytes([addr[6], addr[7]]),
                u16::from_be_bytes([addr[8], addr[9]]),
                u16::from_be_bytes([addr[10], addr[11]]),
                u16::from_be_bytes([addr[12], addr[13]]),
                u16::from_be_bytes([addr[14], addr[15]]),
            );
            let port = u16::from_be_bytes(port_buf);
            (host, port)
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unknown address type",
            ));
        }
    };

    // 4. Reply: success
    stream
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;

    Ok((host, port))
}

pub async fn start_dynamic(forward: Forward, target: ConnTarget) -> AppResult<ForwardHandle> {
    let (mut handle, _fwd) = connect_authed(target).await?;

    let local_port = forward.local_port;

    let (listener, listener6) = bind_loopback(local_port).await?;

    let bytes_tx = Arc::new(AtomicU64::new(0));
    let bytes_rx = Arc::new(AtomicU64::new(0));
    let connections = Arc::new(AtomicU32::new(0));

    let c_tx = bytes_tx.clone();
    let c_rx = bytes_rx.clone();
    let c_conns = connections.clone();
    let disconnect = Arc::new(Notify::new());
    let disconnect_task = disconnect.clone();

    let task = tokio::spawn(async move {
        loop {
            // Accept from either loopback family; handling is identical.
            let mut tcp_stream = tokio::select! {
                _ = disconnect_task.notified() => break,
                res = listener.accept() => match res { Ok((s, _)) => s, Err(_) => break },
                res = accept_opt(&listener6) => match res { Ok((s, _)) => s, Err(_) => break },
            };

            let (target_host, target_port) = match socks5_handshake(&mut tcp_stream).await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let channel = match handle
                .channel_open_direct_tcpip(
                    &target_host,
                    target_port as u32,
                    "127.0.0.1",
                    local_port as u32,
                )
                .await
            {
                Ok(c) => c,
                Err(_) => continue,
            };

            let ssh_stream = channel.into_stream();
            let tx = c_tx.clone();
            let rx = c_rx.clone();
            let conns = c_conns.clone();

            conns.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                let (mut tcp_r, mut tcp_w) = tokio::io::split(tcp_stream);
                let (mut ssh_r, mut ssh_w) = tokio::io::split(ssh_stream);
                let _ = tokio::join!(
                    counted_copy(&mut tcp_r, &mut ssh_w, &tx),
                    counted_copy(&mut ssh_r, &mut tcp_w, &rx),
                );
                conns.fetch_sub(1, Ordering::Relaxed);
            });
        }
        let _ = handle
            .disconnect(Disconnect::ByApplication, "rssh forward stopped", "")
            .await;
    });

    Ok(ForwardHandle {
        abort: task.abort_handle(),
        disconnect,
        bytes_tx,
        bytes_rx,
        connections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::{TcpListener, TcpStream};

    /// 起一对 loopback TCP socket：返回 (server_side, client_side)。
    /// 端口 0 让内核分配，避免冲突。
    async fn loopback_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let connect = tokio::spawn(async move { TcpStream::connect(addr).await.unwrap() });
        let (server, _) = listener.accept().await.unwrap();
        let client = connect.await.unwrap();
        (server, client)
    }

    /// SOCKS5 greeting + 吃回 [0x05, 0x00]。封 helper 让每个测试只关心 connect req。
    async fn negotiate_no_auth(client: &mut TcpStream) {
        client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut reply = [0u8; 2];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, [0x05, 0x00]);
    }

    #[tokio::test]
    async fn socks5_ipv4() {
        let (mut server, mut client) = loopback_pair().await;
        let driver = tokio::spawn(async move {
            negotiate_no_auth(&mut client).await;
            // CONNECT 1.2.3.4:80
            client
                .write_all(&[0x05, 0x01, 0x00, 0x01, 1, 2, 3, 4, 0x00, 0x50])
                .await
                .unwrap();
            let mut reply = [0u8; 10];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply[..2], &[0x05, 0x00]);
        });
        let (host, port) = socks5_handshake(&mut server).await.unwrap();
        assert_eq!(host, "1.2.3.4");
        assert_eq!(port, 80);
        driver.await.unwrap();
    }

    #[tokio::test]
    async fn socks5_domain() {
        let (mut server, mut client) = loopback_pair().await;
        let domain = b"example.com";
        let driver = tokio::spawn(async move {
            negotiate_no_auth(&mut client).await;
            let mut req = vec![0x05, 0x01, 0x00, 0x03, domain.len() as u8];
            req.extend_from_slice(domain);
            req.extend_from_slice(&443u16.to_be_bytes());
            client.write_all(&req).await.unwrap();
            let mut reply = [0u8; 10];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply[..2], &[0x05, 0x00]);
        });
        let (host, port) = socks5_handshake(&mut server).await.unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
        driver.await.unwrap();
    }

    #[tokio::test]
    async fn socks5_ipv6() {
        let (mut server, mut client) = loopback_pair().await;
        // 2001:db8::1
        let addr_bytes: [u8; 16] = [
            0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01,
        ];
        let driver = tokio::spawn(async move {
            negotiate_no_auth(&mut client).await;
            let mut req = vec![0x05, 0x01, 0x00, 0x04];
            req.extend_from_slice(&addr_bytes);
            req.extend_from_slice(&8080u16.to_be_bytes());
            client.write_all(&req).await.unwrap();
            let mut reply = [0u8; 10];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(&reply[..2], &[0x05, 0x00]);
        });
        let (host, port) = socks5_handshake(&mut server).await.unwrap();
        // 实现里 IPv6 全 8 段拼接，不压缩
        assert_eq!(host, "2001:db8:0:0:0:0:0:1");
        assert_eq!(port, 8080);
        driver.await.unwrap();
    }

    #[tokio::test]
    async fn socks5_rejects_non_v5() {
        let (mut server, mut client) = loopback_pair().await;
        let driver = tokio::spawn(async move {
            client.write_all(&[0x04, 0x01, 0x00]).await.unwrap();
        });
        let err = socks5_handshake(&mut server).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        driver.await.unwrap();
    }

    #[tokio::test]
    async fn socks5_rejects_bind_command() {
        let (mut server, mut client) = loopback_pair().await;
        let driver = tokio::spawn(async move {
            negotiate_no_auth(&mut client).await;
            // CMD=0x02 (BIND) — 不支持
            client
                .write_all(&[0x05, 0x02, 0x00, 0x01, 1, 2, 3, 4, 0x00, 0x50])
                .await
                .unwrap();
            let mut reply = [0u8; 10];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(reply[1], 0x07); // command not supported
        });
        let err = socks5_handshake(&mut server).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
        driver.await.unwrap();
    }

    #[tokio::test]
    async fn socks5_rejects_unknown_atyp() {
        let (mut server, mut client) = loopback_pair().await;
        let driver = tokio::spawn(async move {
            negotiate_no_auth(&mut client).await;
            // atyp=0xff 不存在 — 实现走 default 分支直接 Err
            client.write_all(&[0x05, 0x01, 0x00, 0xff]).await.unwrap();
        });
        let err = socks5_handshake(&mut server).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        driver.await.unwrap();
    }

    #[tokio::test]
    async fn counted_copy_streams_and_counts() {
        // tokio 没给 std::io::Cursor 实现 AsyncRead/Write，用官方 duplex。
        let (a_read, mut a_write) = tokio::io::duplex(64);
        let (mut b_read, b_write) = tokio::io::duplex(64);
        let counter = Arc::new(AtomicU64::new(0));
        let counter_clone = counter.clone();

        let copier = tokio::spawn(async move {
            let mut a = a_read;
            let mut b = b_write;
            counted_copy(&mut a, &mut b, &counter_clone).await
        });

        let payload = b"the quick brown fox jumps over the lazy dog";
        a_write.write_all(payload).await.unwrap();
        drop(a_write); // EOF → 退出 loop

        let mut received = Vec::new();
        b_read.read_to_end(&mut received).await.unwrap();
        copier.await.unwrap().unwrap();

        assert_eq!(received, payload);
        assert_eq!(counter.load(Ordering::Relaxed), payload.len() as u64);
    }

    #[tokio::test]
    async fn counted_copy_zero_bytes() {
        let (a_read, a_write) = tokio::io::duplex(64);
        let (_b_read, b_write) = tokio::io::duplex(64);
        let counter = AtomicU64::new(0);
        drop(a_write); // 立刻 EOF
        let mut a = a_read;
        let mut b = b_write;
        counted_copy(&mut a, &mut b, &counter).await.unwrap();
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }
}
