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
    leaf_cred: Option<&Credential>,
) -> AppResult<Vec<tempfile::NamedTempFile>> {
    let chain = rssh_lib::bastion::resolve_chain(conn, profile)?;
    let mut key_files: Vec<tempfile::NamedTempFile> = Vec::new();

    if !chain.is_empty() {
        let mut hops: Vec<String> = Vec::with_capacity(chain.len());
        for hop in &chain {
            // 同 cmd_open_ssh：credential_id 引用 broken 必须报错，不要静默降级。
            let bc = match hop.credential_id.as_deref().filter(|id| !id.is_empty()) {
                Some(id) => Some(load_cred_secrets(
                    conn,
                    rssh_lib::db::credential::get(conn, id)?,
                )?),
                None => None,
            };
            let mut s = String::new();
            if let Some(ref c) = bc {
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
            }
            s.push_str(&hop.host);
            if hop.port != 22 {
                s = format!("{}:{}", s, hop.port);
            }
            hops.push(s);
        }
        cmd.arg("-J").arg(hops.join(","));
    }

    if let Some(cred) = leaf_cred {
        cmd.arg("-l").arg(&cred.username);
        if cred.credential_type == CredentialType::Key {
            if let Some(ref secret) = cred.secret {
                let f = write_temp_key(secret)?;
                cmd.arg("-i").arg(f.path());
                key_files.push(f);
            }
        }
    }

    if profile.port != 22 {
        cmd.arg("-p").arg(profile.port.to_string());
    }

    Ok(key_files)
}

/// 把 PEM 写到临时文件并设 0600。返回 NamedTempFile 必须由 caller 持有到
/// ssh 退出（drop = 删文件）。
pub fn write_temp_key(pem: &str) -> AppResult<tempfile::NamedTempFile> {
    let mut f = tempfile::NamedTempFile::new()?;
    f.write_all(pem.as_bytes())?;
    if !pem.ends_with('\n') {
        f.write_all(b"\n")?;
    }
    f.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(f)
}
