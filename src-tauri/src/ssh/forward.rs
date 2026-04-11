use std::path::PathBuf;
use std::sync::Arc;

use tokio::net::TcpListener;

use crate::error::{AppError, AppResult};
use crate::models::{Credential, Forward};
use crate::ssh::client;

pub struct ForwardHandle {
    abort: tokio::task::AbortHandle,
}

impl ForwardHandle {
    pub fn stop(&self) {
        self.abort.abort();
    }
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

    // 先绑端口，失败直接报错
    let listener = TcpListener::bind(format!("127.0.0.1:{local_port}"))
        .await
        .map_err(|e| AppError::Ssh(format!("端口 {local_port} 绑定失败: {e}")))?;

    let task = tokio::spawn(async move {
        loop {
            let (mut tcp_stream, _) = match listener.accept().await {
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

            let mut stream = channel.into_stream();
            tokio::spawn(async move {
                let _ = tokio::io::copy_bidirectional(&mut tcp_stream, &mut stream).await;
            });
        }
    });

    Ok(ForwardHandle {
        abort: task.abort_handle(),
    })
}
