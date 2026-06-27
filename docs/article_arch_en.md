# Inside RSSH: one Rust crate, three binaries, and the Tauri lessons along the way

[RSSH](https://github.com/shihuili1218/rssh) is a cross-platform SSH client — desktop GUI, a first-class CLI, mobile, and an in-IDE mode — with an AI ops-diagnosis assistant that keeps the human in the loop. It's built in Rust on top of Tauri 2. This is a writeup of how it's structured, which crates we leaned on (and what they cost us), and the Tauri sharp edges we hit shipping to macOS, Windows, Linux, Android and JetBrains.

## What RSSH does

Three things define the product; the rest of this article is how they're built.

### 🤖 AI triage, with you always in the loop
Not another chat box. It reads what is **actually happening** in your terminal, proposes **read-first** commands, each annotated with its side effects and gated behind an explicit "Run" click. Before any payload leaves your machine it passes a shape validator and local redaction — your keys and internal addresses never go out verbatim.

### 🎨 Color-coded command blocks
Every command and its output become a block with a color-coded left edge. In a thousand-line scrollback you spot the previous command's output at a glance. Rendered **fully locally** — zero remote dependency, no agent installed on the server.

### ⌨️ Configure once, use everywhere
`rssh open prod` launches a session from any terminal — the CLI and GUI share one SQLite store. The same hosts and keys also run on mobile and inside a JetBrains tool window.

## One crate, three binaries

The whole backend is a single library crate that compiles into three feature-gated binaries plus a linkable lib for mobile:

```toml
[lib]
name = "rssh_lib"
crate-type = ["staticlib", "cdylib", "lib"]

[[bin]]                       # the Tauri GUI app (desktop + mobile)
name = "rssh"
path = "src/main.rs"

[[bin]]                       # the CLI: `rssh open prod`
name = "rssh-cli"
path = "src/bin/rssh/main.rs"
required-features = ["cli"]

[[bin]]                       # headless WebSocket server for the JetBrains plugin
name = "rssh-server"
path = "src/server_main.rs"
required-features = ["server"]
```

The GUI and the CLI talk to the *same* core and the *same* on-disk SQLite database (`rusqlite` with the `bundled` feature, so there's no system SQLite to chase across platforms). `rssh open prod` from any terminal opens the host you configured in the GUI, because there is no second source of truth.

The headless `rssh-server` is "a second adapter over the same RSSH engine" — it lets a JetBrains plugin run the full UI inside an IDE tool window. To stay self-contained it embeds the built frontend straight into the binary:

```rust
static UI: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/../dist");
```

It peeks at each incoming request without consuming it and decides between a WebSocket upgrade and serving that static UI.

The frontend (Svelte) reaches the core through **143 `#[tauri::command]` handlers** — that's the entire GUI↔Rust boundary. By line count the modules break down as `ai/` (~9.4k, the diagnosis engine and its redaction/validation layers), `ssh/` (~4k), `commands/` (~3.6k), `db/` (~2.8k), then `sync/`, `secret/`, `terminal/` and `migration/`.

Mobile reuses everything. The desktop `main` and the Android/iOS entry point are the same function:

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() { /* ... */ }
```

Platform differences are handled with `cfg` gates and graceful degradation rather than forks — e.g. PTY and serial support are desktop-only, and on Android the OS keychain isn't available so the secret store falls back to the database.

## Crate selection, and the prices we paid

**SSH: `russh` (+ `russh-sftp`).** A pure-Rust SSH stack means no OpenSSH/libssh2 to cross-compile, which matters a lot once Android and Windows are in scope. The cost showed up in the lockfile: `russh` 0.60.x pulls in *prerelease* RustCrypto APIs, so we had to pin a whole row of crates to exact release-candidate versions or Cargo would happily resolve them forward into incompatible `pkcs8` APIs:

```toml
russh = "0.60.1"
ecdsa  = { version = "=0.17.0-rc.16", default-features = false }
pkcs8  = { version = "=0.11.0-rc.11", default-features = false }
rsa    = { version = "=0.10.0-rc.16", default-features = false }
# ed25519, elliptic-curve, p256/p384/p521, pkcs5 … all pinned the same way
```

If you adopt a fast-moving crypto-heavy crate, budget for this. The payoff is real RSA server-sig-algs support (`PrivateKeyWithHashAlg` + `best_supported_rsa_hash()`) against modern servers.

**TLS: `reqwest` with `rustls-tls`, not `native-tls`.** The AI assistant makes HTTPS calls to whatever model endpoint you configure; choosing rustls keeps OpenSSL out of the build entirely, which keeps Windows and mobile cross-compiles boring (the good kind).

**Secrets and backup.** Credentials live in the OS keychain via `keyring` v3, wired per-target so each platform uses its native backend:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
keyring = { version = "3", features = ["apple-native"] }
[target.'cfg(target_os = "windows")'.dependencies]
keyring = { version = "3", features = ["windows-native"] }
[target.'cfg(target_os = "linux")'.dependencies]
keyring = { version = "3", features = ["sync-secret-service", "crypto-rust"] }
```

Encrypted config backup is plain, boring RustCrypto — `argon2` (Argon2id) to derive a key, `chacha20poly1305` for the AEAD, `getrandom` for salts and nonces, and `zeroize` to wipe the in-process passphrase cache on drop. No hand-rolled crypto.

**Terminal and serial.** `portable-pty` drives the local shell on desktop. Serial console is a third byte-stream alongside SSH and PTY via `serialport` — which on Linux needs `libudev-dev` at build time, uses IOKit on macOS and Win32 on Windows. One flag we needed (`IXANY`) isn't exposed by the crate, so on Unix we drop to raw `termios` through `libc`.

A couple more deliberate picks: `tree-sitter` + `tree-sitter-bash` for shell-aware parsing of command structure, and `regex` + `regex-syntax` — the latter so we can reject a redaction rule whose pattern can match zero-width *at save time* (a `minimum_len() == 0` pattern replaces at every position, which is catastrophic over-redaction).

## Tauri 2: the good parts and the sharp edges

Tauri gave us one UI codebase across five targets. The sharp edges were worth writing down.

**1. Build a window from an `async` command, or deadlock on Windows.** Creating a second window from a synchronous command deadlocks on Windows (`WebviewWindowBuilder::build()`, see wry#583). The command that opens a tab in a new window carries a comment that it MUST stay `async`:

```rust
// MUST stay `async`: on Windows, `WebviewWindowBuilder::build()` deadlocks
// when called from a synchronous command. See wry#583.
pub async fn open_tab_in_new_window(/* ... */) -> Result<…> { … }
```

**2. The bundler auto-discovers `src/bin/`.** The Tauri bundler treats every `src/bin/<name>` as an app binary to ship — *ignoring* `required-features`. That would force the GUI build to bundle a `rssh-server` it never compiled. The fix is mundane and easy to miss: keep that binary's source at `src/server_main.rs`, outside `src/bin/`, and point the `[[bin]]` entry at it.

**3. WebKitGTK on Linux/Wayland.** On some NVIDIA/wlroots setups WebKitGTK's DMABUF renderer fails *before* the Tauri window is even created, and a globally-exported `GBM_BACKEND` (notably `nvidia-drm`) triggers "Failed to create GBM buffer" under Hyprland. At startup on a Wayland session we default `WEBKIT_DISABLE_DMABUF_RENDERER=1` and unset `GBM_BACKEND`, each with an explicit env-var opt-out so we never trap users whose stack actually needs them.

**4. Window events as real logic.** Beyond plumbing, the `on_window_event` handler is where directionally-opened windows are bound so dragging one moves its group — with the binding deliberately suspended at OS boundaries (a window animating into fullscreen or minimizing fires a `Moved` we must not propagate).

One design stance that paid off: when the keychain backend is marked as available but the system can't actually produce it (corrupt keychain, dead D-Bus), we *hard-fail startup* rather than silently fall back to a file store — a silent fallback would mint a new master key and leave every existing ciphertext undecryptable. Failing loudly is the safer bug.

---

RSSH is MIT-licensed and the issue tracker is open. If pure-Rust SSH, Tauri across five targets, or local-first AI ops tooling is your thing, contributions and bug reports are welcome: <https://github.com/shihuili1218/rssh>.
