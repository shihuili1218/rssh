use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

pub const CLI_VERSION: &str = "1.0.0";

#[derive(Serialize)]
pub struct CliStatus {
    pub installed: bool,
    pub path: String,
    pub bundled: bool,
    pub installed_version: Option<String>,
    pub expected_version: &'static str,
    pub needs_update: bool,
}

fn install_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        dirs::data_local_dir()
            .unwrap_or_default()
            .join("Programs")
            .join("rssh")
    } else {
        PathBuf::from("/usr/local/bin")
    }
}

fn cli_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "rssh.exe"
    } else {
        "rssh"
    }
}

fn find_installed() -> Option<PathBuf> {
    let name = cli_name();
    // Check common paths
    let candidates = if cfg!(target_os = "windows") {
        vec![
            install_dir().join(name),
            dirs::home_dir()
                .unwrap_or_default()
                .join(".cargo")
                .join("bin")
                .join(name),
        ]
    } else {
        vec![
            PathBuf::from("/usr/local/bin").join(name),
            dirs::home_dir()
                .unwrap_or_default()
                .join(".local")
                .join("bin")
                .join(name),
            dirs::home_dir()
                .unwrap_or_default()
                .join(".cargo")
                .join("bin")
                .join("rssh-cli"),
        ]
    };
    candidates.into_iter().find(|p| p.exists())
}

fn find_bundled(app: &AppHandle) -> Option<PathBuf> {
    let name = if cfg!(target_os = "windows") {
        "rssh-cli.exe"
    } else {
        "rssh-cli"
    };

    // 1. Production: bundled in app resources
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join("bin").join(name);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Dev: local cargo build output
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("release")
        .join(name);
    if dev.exists() {
        return Some(dev);
    }

    None
}

fn installed_version(path: &PathBuf) -> Option<String> {
    let output = Command::new(path).arg("version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8(output.stdout).ok()?.trim().to_string();
    parse_version(&version).map(|_| version)
}

fn parse_version(version: &str) -> Option<Vec<u64>> {
    let version = version.strip_prefix('v').unwrap_or(version);
    if version.is_empty() {
        return None;
    }
    version.split('.').map(|part| part.parse().ok()).collect()
}

fn version_is_older(installed: &str, expected: &str) -> bool {
    let Some(mut installed) = parse_version(installed) else {
        return true;
    };
    let Some(mut expected) = parse_version(expected) else {
        return false;
    };
    let length = installed.len().max(expected.len());
    installed.resize(length, 0);
    expected.resize(length, 0);
    installed < expected
}

fn build_status(installed: Option<PathBuf>, bundled: bool) -> CliStatus {
    let installed_version = installed.as_ref().and_then(installed_version);
    let needs_update = installed_version
        .as_deref()
        .map(|version| version_is_older(version, CLI_VERSION))
        .unwrap_or(true);

    CliStatus {
        installed: installed.is_some(),
        path: installed
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        bundled,
        installed_version,
        expected_version: CLI_VERSION,
        needs_update,
    }
}

#[tauri::command]
pub fn cli_status(app: AppHandle) -> CliStatus {
    build_status(find_installed(), find_bundled(&app).is_some())
}

/// Headless CLI status: PATH-based install check only. The bundled-resource
/// probe needs a Tauri `AppHandle`, which the headless server doesn't have;
/// the embedded server prepends its own dir to the shell PATH instead, so it
/// reports `bundled: false` and leaves install to the host (IDEA plugin / app).
pub fn cli_status_headless() -> CliStatus {
    build_status(find_installed(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_handles_old_equal_and_newer_cli() {
        assert!(version_is_older("0.9.9", CLI_VERSION));
        assert!(!version_is_older(CLI_VERSION, CLI_VERSION));
        assert!(!version_is_older("1.1.0", CLI_VERSION));
        assert!(!version_is_older("1.0", CLI_VERSION));
    }

    #[test]
    fn malformed_version_is_outdated() {
        assert!(version_is_older("rssh 1.0.0", CLI_VERSION));
        assert!(version_is_older("", CLI_VERSION));
    }
}

#[tauri::command]
pub fn cli_install(app: AppHandle) -> AppResult<String> {
    let src = find_bundled(&app)
        .ok_or_else(|| AppError::other("cli_not_bundled", serde_json::json!({})))?;

    let dest_dir = install_dir();
    let dest = dest_dir.join(cli_name());

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            r#"do shell script "mkdir -p '{}' && cp '{}' '{}' && chmod 755 '{}'" with administrator privileges"#,
            dest_dir.display(),
            src.display(),
            dest.display(),
            dest.display()
        );
        let status = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .status()
            .map_err(|e| {
                AppError::other(
                    "cli_osascript_failed",
                    serde_json::json!({ "err": e.to_string() }),
                )
            })?;
        if !status.success() {
            return Err(AppError::other(
                "cli_install_cancelled",
                serde_json::json!({}),
            ));
        }
    }

    #[cfg(target_os = "linux")]
    {
        let status = Command::new("pkexec")
            .arg("sh")
            .arg("-c")
            .arg(format!(
                "mkdir -p '{}' && cp '{}' '{}' && chmod 755 '{}'",
                dest_dir.display(),
                src.display(),
                dest.display(),
                dest.display()
            ))
            .status()
            .map_err(|e| {
                AppError::other(
                    "cli_priv_request_failed",
                    serde_json::json!({ "err": e.to_string() }),
                )
            })?;
        if !status.success() {
            return Err(AppError::other(
                "cli_install_cancelled",
                serde_json::json!({}),
            ));
        }
    }

    #[cfg(target_os = "windows")]
    {
        std::fs::create_dir_all(&dest_dir)?;
        std::fs::copy(&src, &dest)?;
        // Add to user PATH via PowerShell
        let _ = Command::new("powershell").arg("-Command")
            .arg(format!(
                r#"$p = [Environment]::GetEnvironmentVariable('Path','User'); if ($p -notlike '*{}*') {{ [Environment]::SetEnvironmentVariable('Path', $p + ';{}', 'User') }}"#,
                dest_dir.display(), dest_dir.display()
            ))
            .status();
    }

    // Setup completions (user-level, no admin needed)
    setup_completions(&dest);

    Ok(dest.display().to_string())
}

fn setup_completions(cli: &PathBuf) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return,
    };

    let shell = std::env::var("SHELL").unwrap_or_default();

    if shell.contains("zsh") {
        let dir = home.join(".zsh/completions");
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(out) = Command::new(cli).arg("completions").arg("zsh").output() {
            let _ = std::fs::write(dir.join("_rssh"), &out.stdout);
        }
    } else if shell.contains("bash") {
        let dir = home
            .join(".local")
            .join("share")
            .join("bash-completion")
            .join("completions");
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(out) = Command::new(cli).arg("completions").arg("bash").output() {
            let _ = std::fs::write(dir.join("rssh"), &out.stdout);
        }
    } else if shell.contains("fish") {
        let dir = home.join(".config").join("fish").join("completions");
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(out) = Command::new(cli).arg("completions").arg("fish").output() {
            let _ = std::fs::write(dir.join("rssh.fish"), &out.stdout);
        }
    }

    #[cfg(target_os = "windows")]
    {
        // PowerShell completion — append to profile if not already present
        if let Ok(out) = Command::new(cli)
            .arg("completions")
            .arg("powershell")
            .output()
        {
            let profile = home
                .join("Documents")
                .join("PowerShell")
                .join("Microsoft.PowerShell_profile.ps1");
            let _ = std::fs::create_dir_all(profile.parent().unwrap());
            let existing = std::fs::read_to_string(&profile).unwrap_or_default();
            if !existing.contains("Register-ArgumentCompleter -Native -CommandName rssh") {
                let mut content = existing;
                content.push_str("\n# RSSH completions\n");
                content.push_str(&String::from_utf8_lossy(&out.stdout));
                let _ = std::fs::write(&profile, content);
            }
        }
    }
}
