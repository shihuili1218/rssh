use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{AppError, AppResult};
use crate::models::Credential;
use crate::ssh::client;

/// i18n code returned when the user cancels a transfer. The frontend
/// (transfers.svelte.ts) matches this literal via `errStr.includes(...)` to
/// flip the status to "cancelled". Keep the constant in sync across both ends.
pub const CANCELLED_CODE: &str = "transfer_cancelled";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    /// unix epoch seconds; 0 means the server did not provide an mtime
    pub mtime: u64,
}

/// Flat walk output. `rel_path` is always '/'-separated (even when the host is
/// Windows); the frontend swaps separators when joining the local path.
#[derive(Debug, Clone, Serialize)]
pub struct WalkEntry {
    pub rel_path: String,
    pub size: u64,
}

/// Maximum recursion depth, counting the root as depth 0. With CAP = 32 the
/// walker accepts paths up to 32 segments deep (root through depth 31) and
/// fails the whole command at depth 32 — guards against symlink cycles and
/// pathological server-side trees.
const WALK_DEPTH_CAP: u32 = 32;

/// Join a remote path segment. Special-cases dir == "/" so the result never
/// contains "//foo"; matches the convention used by the existing callers.
fn join_remote(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), name)
    }
}

pub struct SftpHandle {
    sftp: russh_sftp::client::SftpSession,
    /// 父 SSH 会话 id；None 表示这条 SFTP 走的是独立 TCP（`SftpHandle::connect`）。
    /// SSH 关闭时会按这个字段反向找到所有 children 并清理。
    parent_ssh_id: Option<String>,
}

impl SftpHandle {
    pub fn parent_ssh_id(&self) -> Option<&str> {
        self.parent_ssh_id.as_deref()
    }

    /// Open SFTP subsystem on an existing SSH connection.
    pub async fn from_handle(
        ssh_handle: &crate::ssh::client::SshHandle,
        parent_ssh_id: String,
    ) -> AppResult<Self> {
        let channel = {
            let h = ssh_handle.lock().await;
            h.channel_open_session()
                .await
                .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "open channel", "err": e.to_string() })))?
        };

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "request_subsystem", "err": e.to_string() })))?;

        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| AppError::sftp("sftp_init_failed", json!({ "err": e.to_string() })))?;

        Ok(Self {
            sftp,
            parent_ssh_id: Some(parent_ssh_id),
        })
    }

    pub async fn connect(
        host: String,
        port: u16,
        credential: Credential,
        known_hosts_path: PathBuf,
        timeout_secs: u64,
    ) -> AppResult<Self> {
        let dial = client::DialCtx {
            config: crate::ssh::client::default_client_config(),
            known_hosts_path,
            timeout_secs,
            log: crate::ssh::client::null_logger(),
            prompt_ctx: None,
        };
        let mut handle = client::ssh_connect(dial, host, port).await?;

        client::authenticate(&mut handle, credential, None).await?;

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "open channel", "err": e.to_string() })))?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "request_subsystem", "err": e.to_string() })))?;

        let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| AppError::sftp("sftp_init_failed", json!({ "err": e.to_string() })))?;

        Ok(Self {
            sftp,
            parent_ssh_id: None,
        })
    }

    pub async fn home_dir(&self) -> AppResult<String> {
        self.sftp
            .canonicalize(".")
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "canonicalize", "err": e.to_string() })))
    }

    pub async fn list_dir(&self, path: &str) -> AppResult<Vec<RemoteEntry>> {
        let entries = self
            .sftp
            .read_dir(path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "read_dir", "err": e.to_string() })))?;

        let mut result: Vec<RemoteEntry> = entries
            .map(|e| {
                let name = e.file_name();
                let ft = e.file_type();
                let meta = e.metadata();
                RemoteEntry {
                    name,
                    is_dir: ft.is_dir(),
                    is_symlink: ft.is_symlink(),
                    size: meta.size.unwrap_or(0),
                    mtime: meta.mtime.map(u64::from).unwrap_or(0),
                }
            })
            .collect();

        result.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(result)
    }

    /// Recursively list every downloadable file under `root`, returning a flat
    /// list of (relative path, size).
    ///
    /// - BFS to avoid blowing the stack on deep trees.
    /// - Symlink-to-file: follow once for size, treat as a regular file.
    ///   Symlink-to-dir: skipped to prevent loops.
    /// - Depth exceeding `WALK_DEPTH_CAP` fails the whole command.
    /// - Per-file failures are not handled here: each file is later dispatched
    ///   as an independent Transfer, which owns its own retry/cancel surface.
    ///   This function only builds the work list.
    pub async fn walk_files(&self, root: &str) -> AppResult<Vec<WalkEntry>> {
        let root_norm = root.trim_end_matches('/').to_string();
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        queue.push_back((root.to_string(), 0));
        let mut result: Vec<WalkEntry> = Vec::new();

        while let Some((dir, depth)) = queue.pop_front() {
            if depth >= WALK_DEPTH_CAP {
                return Err(AppError::sftp(
                    "sftp_tree_too_deep",
                    json!({ "path": dir, "depth": depth, "limit": WALK_DEPTH_CAP }),
                ));
            }
            let entries = self.sftp.read_dir(&dir).await.map_err(|e| {
                AppError::sftp(
                    "sftp_io_failed",
                    json!({ "op": "read_dir", "path": dir, "err": e.to_string() }),
                )
            })?;
            for e in entries {
                let name = e.file_name();
                let full = join_remote(&dir, &name);
                let rel = full
                    .strip_prefix(&root_norm)
                    .unwrap_or(&full)
                    .trim_start_matches('/')
                    .to_string();
                let ft = e.file_type();
                if ft.is_dir() {
                    queue.push_back((full, depth + 1));
                } else if ft.is_file() {
                    result.push(WalkEntry {
                        rel_path: rel,
                        size: e.metadata().size.unwrap_or(0),
                    });
                } else if ft.is_symlink() {
                    // Follow once via STAT to learn what the target is.
                    if let Ok(meta) = self.sftp.metadata(&full).await {
                        if meta.file_type().is_file() {
                            result.push(WalkEntry {
                                rel_path: rel,
                                size: meta.size.unwrap_or(0),
                            });
                        }
                        // symlink-to-dir: skip to avoid cycles.
                        // anything else (block/char/fifo): skip.
                    }
                    // metadata failure (e.g. broken symlink): silently skip.
                }
                // Other types (block/char/fifo) are skipped.
            }
        }
        Ok(result)
    }

    pub async fn download(&self, remote_path: &str) -> AppResult<Vec<u8>> {
        self.sftp
            .read(remote_path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "read", "err": e.to_string() })))
    }

    pub async fn upload(&self, remote_path: &str, data: &[u8]) -> AppResult<()> {
        self.sftp
            .write(remote_path, data)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "write", "err": e.to_string() })))
    }

    pub async fn mkdir(&self, path: &str) -> AppResult<()> {
        self.sftp
            .create_dir(path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "create_dir", "err": e.to_string() })))
    }

    /// Stream-download a remote file to a local path with a hard size cap.
    /// 优先用 metadata.size 早期 bail；metadata.size=None 或服务器撒谎时，
    /// 流式 cap 也会在超限的瞬间中止下载。
    ///
    /// 原子性：下载先写入 `<local_path>.part`，全部成功后 rename 到 local_path。
    /// 任何失败路径（size cap、IO error、metadata 错误）都会清理 .part，
    /// **不会污染 local_path**。无前端进度事件——AI 排障流程用。
    pub async fn download_to_path(
        &self,
        remote_path: &str,
        local_path: &Path,
        max_bytes: u64,
    ) -> AppResult<u64> {
        // 预检：metadata 已知大小且超限就早 bail，省一次 open。size=None
        // 时不能假装它是 0（之前的 unwrap_or(0) 会让任何"未声明大小"的文件
        // 直接绕过 max_bytes）—— 交给下面的 streaming cap 兜底。
        let meta = self
            .sftp
            .metadata(remote_path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "metadata", "err": e.to_string() })))?;
        if let Some(size) = meta.size {
            if size > max_bytes {
                return Err(AppError::sftp(
                    "sftp_file_too_large",
                    json!({ "path": remote_path, "size": size, "limit": max_bytes }),
                ));
            }
        }

        // tmp_path = "<local_path>.part"。用 with_file_name 保留目录前缀。
        let file_name = local_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| AppError::sftp("sftp_io_failed", json!({ "op": "create", "err": "invalid local path" })))?;
        let tmp_path = local_path.with_file_name(format!("{file_name}.part"));

        let result: AppResult<u64> = async {
            let mut remote_file = self
                .sftp
                .open(remote_path)
                .await
                .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "open", "err": e.to_string() })))?;
            let mut local_file = tokio::fs::File::create(&tmp_path).await?;

            let mut transferred: u64 = 0;
            let mut buf = vec![0u8; 32768];
            loop {
                let n = remote_file
                    .read(&mut buf)
                    .await
                    .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "read", "err": e.to_string() })))?;
                if n == 0 {
                    break;
                }
                // 运行时 cap：metadata 缺失 / 撒谎 / 下载途中文件增长都靠这里兜底。
                // 这是 max_bytes 的唯一权威检查点，预检只是优化。
                let next = transferred + n as u64;
                if next > max_bytes {
                    // 主动关闭 server-side handle，免得让远端 fd 等到 session drop 才回收。
                    let _ = remote_file.shutdown().await;
                    return Err(AppError::sftp(
                        "sftp_file_too_large",
                        json!({ "path": remote_path, "size": next, "limit": max_bytes }),
                    ));
                }
                local_file.write_all(&buf[..n]).await?;
                transferred = next;
            }

            remote_file
                .shutdown()
                .await
                .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "close", "err": e.to_string() })))?;
            local_file.shutdown().await?;
            Ok(transferred)
        }
        .await;

        match result {
            Ok(n) => {
                // Windows `MoveFileExW` 默认不覆盖已存在目标 —— ai/session.rs
                // 在同一 session 内反复用同一 local_path，第二次 rename 就会
                // 失败。Unix 上 rename 本来就覆盖，预先 remove 是 no-op。
                let _ = tokio::fs::remove_file(local_path).await;
                tokio::fs::rename(&tmp_path, local_path).await?;
                Ok(n)
            }
            Err(e) => {
                // best-effort cleanup — 即使 unlink 失败，也只是留一个 .part，
                // 不会污染 local_path（消费端约定的产出路径）。
                let _ = tokio::fs::remove_file(&tmp_path).await;
                Err(e)
            }
        }
    }

    /// Stream-download a remote file to a local path, emitting progress events.
    ///
    /// `cancel` 是用户取消的旗子；streaming 循环每个 chunk 之间查一次。
    /// 命中即提前返回 `Sftp(CANCELLED_MSG)`，下游靠这条文本识别"取消"和"出错"。
    pub async fn download_streaming(
        &self,
        remote_path: &str,
        local_path: &Path,
        host: &crate::emitter::Host,
        transfer_id: &str,
        cancel: Arc<AtomicBool>,
    ) -> AppResult<u64> {
        // Get file size for progress
        let meta = self
            .sftp
            .metadata(remote_path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "metadata", "err": e.to_string() })))?;
        let total = meta.size.unwrap_or(0);

        let mut remote_file = self
            .sftp
            .open(remote_path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "open", "err": e.to_string() })))?;

        // For multi-select downloads, local_path may live inside a subdirectory
        // we haven't created yet (e.g. <pick_dir>/<root>/<subdir>/file.txt).
        // For a single-file download the parent already exists, so
        // create_dir_all is a no-op.
        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Atomicity: write to `<local_path>.part` first; rename to final name
        // only on full success. Cancel / read-error / write-error all leave
        // only a partial `.part` (cleaned up below) — never a truncated file
        // sitting where users might pick it up. Mirrors `download_to_path`.
        let file_name = local_path
            .file_name()
            .ok_or_else(|| AppError::sftp("sftp_invalid_filename", json!({})))?
            .to_string_lossy()
            .into_owned();
        let tmp_path = local_path.with_file_name(format!("{file_name}.part"));

        let result = self
            .stream_to_part(&mut remote_file, &tmp_path, total, host, transfer_id, cancel)
            .await;
        match result {
            Ok(transferred) => {
                // Best-effort: remove any pre-existing destination so rename
                // doesn't fail on Windows (Unix rename overwrites silently).
                let _ = tokio::fs::remove_file(local_path).await;
                tokio::fs::rename(&tmp_path, local_path).await?;
                let _ = remote_file
                    .shutdown()
                    .await
                    .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "close", "err": e.to_string() })));
                Ok(transferred)
            }
            Err(e) => {
                // Best-effort cleanup; even if unlink fails we just leak `.part`.
                let _ = tokio::fs::remove_file(&tmp_path).await;
                Err(e)
            }
        }
    }

    /// Inner streaming loop for `download_streaming`. Writes to the supplied
    /// temp path and emits progress events; the caller decides whether to
    /// promote `.part` to the final name or clean it up.
    async fn stream_to_part(
        &self,
        remote_file: &mut russh_sftp::client::fs::File,
        tmp_path: &Path,
        total: u64,
        host: &crate::emitter::Host,
        transfer_id: &str,
        cancel: Arc<AtomicBool>,
    ) -> AppResult<u64> {
        let mut local_file = tokio::fs::File::create(tmp_path).await?;
        let mut transferred: u64 = 0;
        let mut buf = vec![0u8; 32768];
        // Build the event name once; we emit on every chunk and want to avoid
        // repeated String allocations inside the loop.
        let event = format!("sftp:progress:{transfer_id}");

        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AppError::sftp(CANCELLED_CODE, json!({})));
            }
            let n = remote_file
                .read(&mut buf)
                .await
                .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "read", "err": e.to_string() })))?;
            if n == 0 {
                break;
            }
            local_file.write_all(&buf[..n]).await?;
            transferred += n as u64;
            let _ = host.emit(
                &event,
                serde_json::json!({ "transferred": transferred, "total": total }),
            );
        }
        Ok(transferred)
    }

    /// Remote `mkdir -p`: ensure every ancestor directory of `path` exists.
    ///
    /// Concurrency note: under MAX_CONCURRENT uploads several tasks may race
    /// to create the same shared subdirectory. We therefore *blindly attempt*
    /// `create_dir` per segment and, on failure, fall back to a metadata probe.
    /// If a directory is now present, treat it as success (a peer created it);
    /// otherwise propagate the original create error. This trades a few extra
    /// SFTP round-trips on the cold path for race-freeness — `mkdir_p` is
    /// already off the hot single-file path because callers short-circuit
    /// when the parent already exists.
    async fn mkdir_p(&self, path: &str) -> AppResult<()> {
        if path.is_empty() || path == "/" {
            return Ok(());
        }
        if let Ok(meta) = self.sftp.metadata(path).await {
            // A non-directory occupying the target path would cause obscure
            // failures later when the upload tries to write under it.
            if !meta.is_dir() {
                return Err(AppError::sftp(
                    "sftp_io_failed",
                    json!({ "op": "mkdir_p", "path": path, "err": "path exists but is not a directory" }),
                ));
            }
            return Ok(());
        }
        let mut current = String::new();
        for part in path.split('/').filter(|s| !s.is_empty()) {
            current.push('/');
            current.push_str(part);
            if let Err(create_err) = self.sftp.create_dir(&current).await {
                // Race-safe fallback: verify a directory now exists.
                match self.sftp.metadata(&current).await {
                    Ok(meta) if meta.is_dir() => {} // created by a concurrent peer
                    _ => {
                        return Err(AppError::sftp(
                            "sftp_io_failed",
                            json!({ "op": "mkdir_p", "path": &current, "err": create_err.to_string() }),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Stream-upload a local file to a remote path, emitting progress events.
    pub async fn upload_streaming(
        &self,
        local_path: &Path,
        remote_path: &str,
        host: &crate::emitter::Host,
        transfer_id: &str,
        cancel: Arc<AtomicBool>,
    ) -> AppResult<u64> {
        let local_meta = tokio::fs::metadata(local_path).await?;
        let total = local_meta.len();

        let mut local_file = tokio::fs::File::open(local_path).await?;

        // When uploading a folder via multi-select, remote_path may live in a
        // remote subdirectory we haven't created yet. For a single-file upload
        // the parent already exists, so mkdir_p hits the hot-path early return.
        if let Some((parent, _)) = remote_path.rsplit_once('/') {
            if !parent.is_empty() {
                self.mkdir_p(parent).await?;
            }
        }

        let mut remote_file = self
            .sftp
            .create(remote_path)
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "create", "err": e.to_string() })))?;

        let mut transferred: u64 = 0;
        let mut buf = vec![0u8; 32768];
        // 事件名每个 chunk emit 一次；预算一次避免循环里反复 String 分配。
        let event = format!("sftp:progress:{transfer_id}");

        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AppError::sftp(CANCELLED_CODE, json!({})));
            }
            let n = local_file.read(&mut buf).await?;
            if n == 0 {
                break;
            }

            remote_file
                .write_all(&buf[..n])
                .await
                .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "write", "err": e.to_string() })))?;
            transferred += n as u64;

            let _ = host.emit(
                &event,
                serde_json::json!({ "transferred": transferred, "total": total }),
            );
        }

        remote_file
            .shutdown()
            .await
            .map_err(|e| AppError::sftp("sftp_io_failed", json!({ "op": "close", "err": e.to_string() })))?;

        Ok(transferred)
    }
}
