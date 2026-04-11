# RSSH

SSH connection manager — desktop app (Windows/macOS/Linux/Android) + CLI tool.

## CLI

### Install

**From desktop app:** Settings > CLI Tool > Install (auto-configures completions).

**From source:**
```bash
cd src-tauri
cargo install --path . --features cli --bin rssh-cli
ln -sf ~/.cargo/bin/rssh-cli /usr/local/bin/rssh
```

**From GitHub Release:** download `rssh-<target>` binary, rename to `rssh`, place in PATH.

### Shell Completions

The desktop app installer auto-configures completions. To set up manually:

```bash
# zsh
mkdir -p ~/.zsh/completions
rssh completions zsh > ~/.zsh/completions/_rssh
# ensure ~/.zshrc has: fpath=(~/.zsh/completions $fpath) && autoload -Uz compinit && compinit

# bash
rssh completions bash >> ~/.bashrc

# fish
rssh completions fish > ~/.config/fish/completions/rssh.fish

# PowerShell
rssh completions powershell >> $PROFILE
```

### Usage

```
rssh                              # list all profiles (default)
rssh ls [query]                   # list/search profiles
rssh ls cred                      # list credentials
rssh ls fwd                       # list port forwards

rssh open <name>                  # connect via SSH
rssh open fwd <name>              # start port forward

rssh add profile                  # interactive: create profile
rssh add cred                     # interactive: create credential
rssh add fwd                      # interactive: create forward

rssh edit profile <name>          # edit profile
rssh edit cred <name>             # edit credential
rssh edit fwd <name>              # edit forward

rssh rm profile <name>            # delete profile
rssh rm cred <name>               # delete credential
rssh rm fwd <name>                # delete forward

rssh config export <file>         # encrypted backup to file
rssh config import <file>         # restore from encrypted file
rssh config set                   # configure GitHub sync settings
rssh config push                  # push encrypted config to GitHub
rssh config pull                  # pull config from GitHub

rssh completions <shell>          # generate completion script (zsh|bash|fish|powershell)
```

### Data

CLI and desktop app share the same database:

```
~/.rssh/rssh.db        # SQLite database
~/.rssh/known_hosts    # SSH host key store
~/.rssh/snippets.json  # command snippets
```

## Desktop App

### Dev

```bash
npm install
npm run tauri dev
```

### Build

```bash
npm run tauri build
```

### Release

Push a tag `v*` to trigger the GitHub Actions workflow. It builds:

| Platform | Artifacts |
|----------|-----------|
| macOS (Apple Silicon) | `.dmg` + standalone `rssh` CLI |
| macOS (Intel) | `.dmg` + standalone `rssh` CLI |
| Linux (x64) | `.deb` / `.AppImage` + standalone `rssh` CLI |
| Windows (x64) | `.msi` / `.exe` installer + standalone `rssh.exe` CLI |
| Android | `.apk` |

Desktop installers bundle the CLI binary. Install via Settings > CLI Tool.
