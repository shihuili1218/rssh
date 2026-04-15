# RSSH

SSH connection manager with a desktop GUI and built-in CLI.

macOS / Windows / Linux / Android.

## Features

- **SSH** -- password, private key, keyboard-interactive, jump host (ProxyJump)
- **Terminal** -- xterm emulation, 10 000-line scrollback, keyword highlighting, search
- **SFTP** -- remote file browser, upload/download
- **Port Forwarding** -- local and remote, named configs, real-time stats
- **Local Terminal** -- auto-detect zsh/bash/PowerShell
- **Session Recording** -- asciicast v2 format, variable-speed playback
- **Profiles & Credentials** -- SQLite storage, import from `~/.ssh/config`
- **Sync** -- encrypted export/import, GitHub backup
- **Snippets** -- reusable command shortcuts (Cmd+E)
- **Mobile** -- virtual keybar (Ctrl/Alt/arrows/Tab/Esc), safe area, stack navigation

## Install

Download from [Releases](../../releases):

| Platform | File | Notes |
|---|---|---|
| macOS Apple Silicon | `rssh-{ver}-macos-aarch64.dmg` | |
| macOS Intel | `rssh-{ver}-macos-x86_64.dmg` | |
| Linux (deb) | `rssh-{ver}-linux-x86_64.deb` | Debian/Ubuntu |
| Linux (rpm) | `rssh-{ver}-linux-x86_64.rpm` | Fedora/RHEL |
| Linux (AppImage) | `rssh-{ver}-linux-x86_64.AppImage` | Any distro |
| Windows | `rssh-{ver}-windows-x86_64.msi` | Silent install: `msiexec /i` |
| Windows | `rssh-{ver}-windows-x86_64-setup.exe` | GUI installer |
| Android | `rssh-{ver}-android-universal.apk` | |

## CLI

The CLI binary is bundled inside the desktop app. Install it from **Settings > CLI Tool > Install** (auto-configures shell completions).

To build from source:

```bash
cd src-tauri
cargo install --path . --features cli --bin rssh-cli
ln -sf ~/.cargo/bin/rssh-cli /usr/local/bin/rssh
```

### Usage

```
rssh                              # list profiles
rssh ls [query]                   # search profiles
rssh ls cred                      # list credentials
rssh ls fwd                       # list forwards

rssh open <name>                  # SSH connect
rssh open fwd <name>              # start forward

rssh add profile|cred|fwd         # create (interactive)
rssh edit profile|cred|fwd <name> # edit
rssh rm profile|cred|fwd <name>   # delete

rssh config export <file>         # encrypted backup
rssh config import <file>         # restore
rssh config set                   # configure GitHub sync
rssh config push                  # push to GitHub
rssh config pull                  # pull from GitHub

rssh completions <shell>          # zsh | bash | fish | powershell
```

### Data

CLI and desktop app share the same database:

```
~/.rssh/rssh.db        # SQLite (profiles, credentials, forwards, settings)
~/.rssh/snippets.json  # command snippets
~/.ssh/known_hosts     # OpenSSH-standard host key store (shared with `ssh`)
```

Host key trust is stored in the standard OpenSSH `known_hosts` file, so trust
established with `ssh` is reused by rssh and vice versa. Use `ssh-keygen -R <host>`
to remove an entry; use `ssh-keygen -F <host>` to inspect.

## Keyboard Shortcuts

| Key | Action |
|---|---|
| Ctrl+Tab / Ctrl+Shift+Tab | Switch between tabs |
| Cmd+W | Close current tab |
| Cmd+F | Terminal search |
| Cmd+S | Command snippet |
| Cmd+O | SFTP browser |

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT

