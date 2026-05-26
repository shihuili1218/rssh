//! 把 Profile + bastion 链 + 凭证编进 `Command::new("ssh")`。
//!
//! `cmd_open_ssh` 与 `cmd_open_fwd` 之前各自独立写了 ~85 行相同的"建链 +
//! 写入 -i / -o IdentityFile + -p"逻辑。提到这里走一份。

use std::io::Write;
use std::process::Command;

use rssh_lib::error::AppResult;
use rssh_lib::models::{Credential, CredentialType, Profile};

use super::cred::load_cred_secrets;
use crate::ctx::CliCtx;

/// 把 profile（含可选 bastion 链）和 leaf 凭证写进 `cmd`，返回必须挂在 `cmd`
/// 之外、活到 ssh 退出的临时 key 文件列表（caller 必须持有，否则 file 提前 drop
/// = 删，OpenSSH 读不到）。
///
/// 不包含 `forward` 标志（-L/-R/-D）和 `-N` —— 这些由 caller（`cmd_open_fwd`）追加。
/// 也不追加目标 host —— caller 决定带不带 init_command + `-t` 形态。
pub fn build_ssh_command(
    cmd: &mut Command,
    conn: &CliCtx,
    profile: &Profile,
    leaf_cred: &Credential,
) -> AppResult<Vec<tempfile::NamedTempFile>> {
    let chain = rssh_lib::bastion::resolve_chain(conn, profile)?;
    let mut key_files: Vec<tempfile::NamedTempFile> = Vec::new();

    if !chain.is_empty() {
        let mut hops: Vec<String> = Vec::with_capacity(chain.len());
        for hop in &chain {
            // Profile.credential_id 是必填（add/edit 入口强制），跳板机也不例外。
            // 找不到 cred = DB 数据不一致，显式报错而不是降级到"无 user@"。
            let c = load_cred_secrets(
                conn,
                rssh_lib::db::credential::get(conn, &hop.credential_id)?,
            )?;
            let mut s = String::new();
            s.push_str(&c.username);
            s.push('@');
            if c.credential_type == CredentialType::Key {
                if let Some(ref secret) = c.secret {
                    let f = write_temp_key(secret)?;
                    cmd.arg("-o")
                        .arg(format!("IdentityFile={}", f.path().display()));
                    key_files.push(f);
                }
            }
            s.push_str(&hop.host);
            if hop.port != 22 {
                s = format!("{}:{}", s, hop.port);
            }
            hops.push(s);
        }
        cmd.arg("-J").arg(hops.join(","));
    }

    cmd.arg("-l").arg(&leaf_cred.username);
    if leaf_cred.credential_type == CredentialType::Key {
        if let Some(ref secret) = leaf_cred.secret {
            let f = write_temp_key(secret)?;
            cmd.arg("-i").arg(f.path());
            key_files.push(f);
        }
    }

    if profile.port != 22 {
        cmd.arg("-p").arg(profile.port.to_string());
    }

    Ok(key_files)
}

/// Write a PEM to a temp file with 0600 perms (Unix) in a user-private dir.
/// Caller must hold the returned `NamedTempFile` until ssh exits — drop =
/// delete.
///
/// Two changes vs. the obvious `NamedTempFile::new()` + chmod sequence:
///
/// 1. **Atomic 0600**: `Builder::permissions(0o600)` passes the mode to the
///    OS `open(2)` call, so the file is born 0600. The old "create then
///    chmod" leaked a window where the PEM bytes existed on disk at 0644
///    (default umask). Short window, but reproducible on every `rssh open`,
///    and `/tmp` is world-traversable on shared hosts / CI runners.
///
/// 2. **User-private dir** instead of `/tmp`: even with 0600 on the file,
///    the directory listing on `/tmp` exposes filenames; we also can't
///    enumerate other users' temp keys from a 0700 dir. Mirrors the
///    `secret/master_key.rs` "atomic create with mode" pattern.
pub fn write_temp_key(pem: &str) -> AppResult<tempfile::NamedTempFile> {
    let dir = secure_key_tmpdir()?;

    let mut builder = tempfile::Builder::new();
    builder.prefix("rssh-key-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o600));
    }

    let mut f = builder.tempfile_in(&dir)?;
    f.write_all(pem.as_bytes())?;
    if !pem.ends_with('\n') {
        f.write_all(b"\n")?;
    }
    f.flush()?;
    Ok(f)
}

/// `~/.rssh/tmp/`, ensured to exist with 0700 (Unix). We set perms each
/// call rather than only on first-create — defends against a user who
/// chmodded the dir to 0755 thinking it was harmless.
fn secure_key_tmpdir() -> AppResult<std::path::PathBuf> {
    let mut dir = rssh_lib::db::data_dir()?;
    dir.push("tmp");
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}
