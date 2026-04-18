# rssh — An SSH Client That Doesn't Piss in Your House

Most SSH clients on the market share the same problem: they force you to adapt to them. rssh takes the opposite approach — **it fits into your existing Unix toolchain, instead of making you bend to the tool.**

## 1. rssh CLI

The rssh CLI lets you use rssh's data from any terminal. `rssh open [profile]` connects you from anywhere. The key point: the CLI and the GUI app **read the same SQLite database** (`~/.rssh/rssh.db`) — they're fully interoperable. A profile you add in the GUI is instantly available in the CLI, and vice versa.

```
rssh                        # list all profiles
rssh ls prod                # search
rssh open gateway-01        # connect directly
rssh add profile            # interactive creation
rssh open fwd my-tunnel     # start a named forward
```

This means you can drop `rssh open foo` into any script or alias — no need to duplicate SSH configs.

## 2. Command Block Color Sidebar — Zero Remote Configuration

Ever lost track of where the last command started in a wall of terminal output? Tools like Warp solve this, but they require you to install shell integration scripts on the server. rssh does it **purely on the frontend, with zero remote-side intrusion.**

How it works:
- Each command gets a colored vertical bar on the left side — input and output share the same color
- The next command automatically switches color (golden-angle HSL algorithm, maximizing contrast between adjacent colors)
- When you enter a fullscreen program (vim, top, less), the sidebar fades to a translucent gray placeholder
- One toggle in settings to turn it off

The key advantage: **no server-side configuration needed.** It works the moment you connect, even on someone else's production bastion host.

## 3. Shares ~/.ssh/known_hosts with ssh

This is the **most fundamental philosophical difference** between rssh and other GUI SSH clients. Most GUI clients (Termius, Tabby, etc.) maintain their own host key database. This means a host you've already trusted via `ssh` on the command line needs to be re-trusted in the GUI.

rssh **reads and writes the standard `~/.ssh/known_hosts` file directly.** Remove an entry with `ssh-keygen -R <host>`, rssh knows immediately. Trust a new host in rssh, `ssh` can connect to it right away. Seamless interop between CLI and GUI.

## 4. Data Security — rssh Stores Nothing, Yet Supports Multi-Device Sync

**Local keys** are stored in your system keychain. We assume you trust your own keychain more than any third-party software.

**Remote keys** — you can choose not to upload them and provide them to another device by other means. Since keys rarely change, this is minimal effort.

**Remote config data** uses an innovative approach: encrypt your configuration (profiles, credentials, forwarding rules) and push it to **your own GitHub private repo.** Multiple devices can share the same connection configs — no third-party service, no subscription.

```
rssh config set             # configure token and repo
rssh config push            # push
rssh config pull            # pull
```

Under the hood it's base64 encoding + standard GitHub API. Want to audit or migrate? Go right ahead.

## 5. Terminal → GUI: OSC 7337 Integration

Type `rssh open my-server` in any rssh terminal (including the local terminal), and the GUI immediately opens a new tab and connects.

The mechanism: the CLI writes a **standard OSC escape sequence** to stdout, and rssh's xterm parser picks it up and triggers the action. This isn't a custom protocol — it's **a legitimate use of standard terminal capabilities.**

The beauty of this: you can use it in a `.zshrc` alias, in a script, in tmux — anywhere. It's just text.

## 6. asciicast v2 Session Recording

rssh records sessions in asciinema's standard `.cast` NDJSON format — not a proprietary format. Any session you record can be directly uploaded with `asciinema upload`, embedded in a webpage, or consumed by any asciinema-compatible tool.

## 7. Other Built-in Features

- **Keyword highlighting** — custom rules, automatic coloring for `ERROR`/`WARN`/`INFO`, 14 color presets
- **SFTP browser** — Cmd+O to open, drag-and-drop upload/download
- **Command snippets** — Cmd+S to access a reusable command library
- **Port forwarding** — local & remote, real-time traffic stats
- **Cross-platform** — macOS (Intel & Apple Silicon), Windows, Linux (deb/rpm/AppImage), and Android

## Download

All platform installers are available on the [Releases](https://github.com/shihuili1218/rssh/releases) page.

**No subscription. No login. No ads.** Source code is MIT licensed. Issues and PRs welcome.

---

*A tool should serve the way you already work — not force you to work around it.*
