use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::error::{AppError, AppResult};
use crate::models::{Credential, Forward};
use crate::models::Profile;
use crate::ssh::client::{self, LogFn};
use std::sync::Arc as StdArc;

pub struct ForwardHandle {
    abort: tokio::task::AbortHandle,
    pub bytes_tx: Arc<AtomicU64>,
    pub bytes_rx: Arc<AtomicU64>,
    pub connections: Arc<AtomicU32>,
}

impl ForwardHandle {
    pub fn stop(&self) {
        self.abort.abort();
    }
}

#[derive(Serialize)]
pub struct ForwardStats {
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    pub connections: u32,
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

pub async fn start_local(
    forward: Forward,
    host: String,
    port: u16,
    credential: Credential,
    bastion_chain: Vec<(Profile, Credential)>,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
) -> AppResult<ForwardHandle> {
    let log: LogFn = StdArc::new(|_: String| ());
    let (mut handle, _fwd) = client::establish_via_chain(
        bastion_chain, host, port, known_hosts_path, timeout_secs, log,
    ).await?;
    client::authenticate(&mut handle, credential).await?;

    let remote_host = forward.remote_host.clone();
    let remote_port = forward.remote_port;
    let local_port = forward.local_port;

    let listener = TcpListener::bind(format!("127.0.0.1:{local_port}"))
        .await
        .map_err(|e| AppError::Ssh(format!("端口 {local_port} 绑定失败: {e}")))?;

    let bytes_tx = Arc::new(AtomicU64::new(0));
    let bytes_rx = Arc::new(AtomicU64::new(0));
    let connections = Arc::new(AtomicU32::new(0));

    let tx = bytes_tx.clone();
    let rx = bytes_rx.clone();
    let conns = connections.clone();

    let task = tokio::spawn(async move {
        loop {
            let (tcp_stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => break,
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
    });

    Ok(ForwardHandle {
        abort: task.abort_handle(),
        bytes_tx,
        bytes_rx,
        connections,
    })
}

// ---------------------------------------------------------------------------
// Remote port forwarding
// ---------------------------------------------------------------------------

pub async fn start_remote(
    forward: Forward,
    host: String,
    port: u16,
    credential: Credential,
    bastion_chain: Vec<(Profile, Credential)>,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
) -> AppResult<ForwardHandle> {
    let log: LogFn = StdArc::new(|_: String| ());
    let (mut handle, fwd_sender) = client::establish_via_chain(
        bastion_chain, host, port, known_hosts_path, timeout_secs, log,
    ).await?;
    client::authenticate(&mut handle, credential).await?;

    // Register a channel to receive forwarded connections from the Handler
    let (ch_tx, mut ch_rx) = tokio::sync::mpsc::unbounded_channel();
    {
        let mut guard = fwd_sender.lock().unwrap();
        *guard = Some(ch_tx);
    }

    // Ask server to listen on remote_port
    let _bound_port = handle
        .tcpip_forward("0.0.0.0", forward.remote_port as u32)
        .await
        .map_err(|e| AppError::Ssh(format!("tcpip_forward 失败: {e}")))?;

    let local_host = forward.remote_host.clone();
    let local_port = forward.local_port;

    let bytes_tx = Arc::new(AtomicU64::new(0));
    let bytes_rx = Arc::new(AtomicU64::new(0));
    let connections = Arc::new(AtomicU32::new(0));

    let tx = bytes_tx.clone();
    let rx = bytes_rx.clone();
    let conns = connections.clone();

    let task = tokio::spawn(async move {
        // Keep handle alive — dropping it kills the SSH connection
        let _handle = handle;
        while let Some(channel) = ch_rx.recv().await {
            let lh = local_host.clone();
            let c_tx = tx.clone();
            let c_rx = rx.clone();
            let c_conns = conns.clone();

            c_conns.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                let local = match TcpStream::connect(format!("{}:{}", lh, local_port)).await {
                    Ok(s) => s,
                    Err(_) => { c_conns.fetch_sub(1, Ordering::Relaxed); return; }
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
    });

    Ok(ForwardHandle {
        abort: task.abort_handle(),
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
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Not SOCKS5"));
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
        stream.write_all(&[0x05, 0x07, 0x00, 0x01, 0,0,0,0, 0,0]).await?;
        return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Only CONNECT supported"));
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
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Unknown address type"));
        }
    };

    // 4. Reply: success
    stream.write_all(&[0x05, 0x00, 0x00, 0x01, 0,0,0,0, 0,0]).await?;

    Ok((host, port))
}

pub async fn start_dynamic(
    forward: Forward,
    host: String,
    port: u16,
    credential: Credential,
    bastion_chain: Vec<(Profile, Credential)>,
    known_hosts_path: PathBuf,
    timeout_secs: u64,
) -> AppResult<ForwardHandle> {
    let log: LogFn = StdArc::new(|_: String| ());
    let (mut handle, _fwd) = client::establish_via_chain(
        bastion_chain, host, port, known_hosts_path, timeout_secs, log,
    ).await?;
    client::authenticate(&mut handle, credential).await?;

    let local_port = forward.local_port;

    let listener = TcpListener::bind(format!("127.0.0.1:{local_port}"))
        .await
        .map_err(|e| AppError::Ssh(format!("端口 {local_port} 绑定失败: {e}")))?;

    let bytes_tx = Arc::new(AtomicU64::new(0));
    let bytes_rx = Arc::new(AtomicU64::new(0));
    let connections = Arc::new(AtomicU32::new(0));

    let c_tx = bytes_tx.clone();
    let c_rx = bytes_rx.clone();
    let c_conns = connections.clone();

    let task = tokio::spawn(async move {
        loop {
            let (mut tcp_stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => break,
            };

            let (target_host, target_port) = match socks5_handshake(&mut tcp_stream).await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let channel = match handle
                .channel_open_direct_tcpip(&target_host, target_port as u32, "127.0.0.1", local_port as u32)
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
    });

    Ok(ForwardHandle {
        abort: task.abort_handle(),
        bytes_tx,
        bytes_rx,
        connections,
    })
}
