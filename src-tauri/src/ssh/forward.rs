use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::error::{AppError, AppResult};
use crate::models::{Credential, Forward};
use crate::ssh::client;

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
    forward: &Forward,
    host: &str,
    port: u16,
    credential: &Credential,
    known_hosts_path: PathBuf,
) -> AppResult<ForwardHandle> {
    let config = Arc::new(russh::client::Config::default());
    let mut handle = client::ssh_connect(config, host, port, known_hosts_path).await?;
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
