# RSSH

SSH connection manager with a desktop GUI and built-in CLI.

macOS / Windows / Linux / Android.

## Boom!

- **Command Block Colors** -- zero remote dependency, color-coded command blocks in terminal
- **CLI-First** -- CLI and GUI share one database, `rssh open prod` from any terminal
- **Security & Sync** -- secrets in platform keychain, per-credential sync filter, encrypted backup to your own GitHub repo

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

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT

