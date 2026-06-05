# RSSH

[English](README.md) | [中文](README_zh.md)

SSH connection manager with a desktop GUI and built-in CLI.

macOS / Windows / Linux / Android.

> Read the story: [Why RSSH? — An SSH Client Born to be an AI Ops Assistant](docs/article_en.md) ([中文](docs/article_zh.md))

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/shihuili1218/rssh)

## Boom!

- **AI Diagnostics** -- LLM-driven general ops triage on the connected host; every tool call gated by shape validator, your approval, and local redaction before the payload leaves your machine
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
- **IDE Plugin** -- run RSSH inside JetBrains IDEs in a tool window (shared data dir)

## Install

Download from [Releases](../../releases):

| Platform            | File                                  | Notes                        |
|---------------------|---------------------------------------|------------------------------|
| macOS Apple Silicon | `rssh-{ver}-macos-aarch64.dmg`        |                              |
| macOS Intel         | `rssh-{ver}-macos-x86_64.dmg`         |                              |
| Linux (deb)         | `rssh-{ver}-linux-x86_64.deb`         | Debian/Ubuntu                |
| Linux (rpm)         | `rssh-{ver}-linux-x86_64.rpm`         | Fedora/RHEL                  |
| Linux (AppImage)    | `rssh-{ver}-linux-x86_64.AppImage`    | Any distro                   |
| Windows             | `rssh-{ver}-windows-x86_64.msi`       | Silent install: `msiexec /i` |
| Windows             | `rssh-{ver}-windows-x86_64-setup.exe` | GUI installer                |
| Android             | `rssh-{ver}-android-universal.apk`    |                              |
| iOS                 |                                       | No ID, build you self        |

### IntelliJ / JetBrains plugin

Run the full RSSH UI inside a JetBrains IDE tool window — same hosts, keys and
settings as the desktop app (shared `~/.rssh`). Each zip bundles a headless
`rssh-server`, so it's self-contained and per-OS:

| Platform            | File                                             |
|---------------------|--------------------------------------------------|
| macOS Apple Silicon | `rssh-{ver}-macos-aarch64-jetbrains-plugin.zip`  |
| macOS Intel         | `rssh-{ver}-macos-x86_64-jetbrains-plugin.zip`   |
| Linux               | `rssh-{ver}-linux-x86_64-jetbrains-plugin.zip`   |
| Windows             | `rssh-{ver}-windows-x86_64-jetbrains-plugin.zip` |

Install: **Settings → Plugins → ⚙ → Install Plugin from Disk…**, pick the zip for
your OS and restart. Open the **RSSH** tool window (bottom) to start; the ✕ in its
title bar stops the embedded server.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT

