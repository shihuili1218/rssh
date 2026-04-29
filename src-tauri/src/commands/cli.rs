use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

#[derive(Serialize)]
pub struct CliStatus {
    pub installed: bool,
    pub path: String,
    pub bundled: bool,
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

#[tauri::command]
pub fn cli_status(app: AppHandle) -> CliStatus {
    let installed = find_installed();
    let bundled = find_bundled(&app).is_some();
    CliStatus {
        installed: installed.is_some(),
        path: installed
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        bundled,
    }
}

#[tauri::command]
pub fn cli_install(app: AppHandle) -> AppResult<String> {
    let src = find_bundled(&app)
        .ok_or_else(|| AppError::Other("CLI binary not bundled in this build. Build with: cargo build --release --features cli --bin rssh-cli".into()))?;

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
            .map_err(|e| AppError::Other(format!("Failed to run osascript: {e}")))?;
        if !status.success() {
            return Err(AppError::Other("Installation cancelled or failed.".into()));
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
            .map_err(|e| AppError::Other(format!("Failed to request privileges: {e}")))?;
        if !status.success() {
            return Err(AppError::Other("Installation cancelled or failed.".into()));
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
