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
~/.rssh/rssh.db        # SQLite
~/.rssh/known_hosts    # SSH host keys
~/.rssh/snippets.json  # command snippets
```

## Keyboard Shortcuts

| Key | Action |
|---|---|
| Cmd+K | Toggle sidebar |
| Cmd+F | Terminal search |
| Cmd+E | Snippet picker |
| Cmd+O | SFTP browser |

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT
