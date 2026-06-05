# Contributing

## Data

CLI and desktop app share the same database:

```
~/.rssh/rssh.db        # SQLite (profiles, credentials, forwards, settings)
~/.rssh/snippets.json  # command snippets
~/.ssh/known_hosts     # OpenSSH-standard host key store (shared with `ssh`)
```

## Prerequisites

All platforms need:

- **Node.js** >= 20
- **Rust** stable (install via [rustup](https://rustup.rs))
- **npm** (comes with Node)

### macOS

No extra dependencies. Xcode Command Line Tools must be installed:

```bash
xcode-select --install
```

### Linux (Debian/Ubuntu)

```bash
sudo apt-get update
sudo apt-get install -y \
    libgtk-3-dev libwebkit2gtk-4.1-dev \
    libayatana-appindicator3-dev librsvg2-dev
```

### Windows

No extra dependencies. Visual Studio Build Tools with C++ workload must be installed.

### Android

- **JDK** 17 (recommend Eclipse Temurin)
- **Android SDK** with NDK installed
- Rust targets:
  ```bash
  rustup target add aarch64-linux-android armv7-linux-androideabi
  ```

## Dev

```bash
# install frontend deps
npm install

# start dev server (hot-reload frontend + Rust backend)
npm run tauri dev
```

## Build

### Desktop (current platform)

```bash
npm run tauri build
```

Output lands in `src-tauri/target/release/bundle/`.

### Desktop with CLI bundled

The desktop app bundles a CLI binary via `resources`. To include it:

```bash
# 1. build CLI
cd src-tauri
cargo build --release --features cli --bin rssh-cli

# 2. stage it
mkdir -p bin
cp target/release/rssh-cli bin/    # or rssh-cli.exe on Windows

# 3. build app
cd ..
npx tauri build
```

### Cross-compile for a specific target

```bash
# add the target
rustup target add x86_64-apple-darwin

# build
npx tauri build --target x86_64-apple-darwin
```

### Android

```bash
# init (first time only)
npx tauri android init

# dev
npx tauri android dev

# release APK
npx tauri android build --apk
```

The release APK requires a signing keystore. See `src-tauri/gen/android/key.properties`.

## Running outside Tauri (headless server + IDEA plugin)

The frontend can run **outside** the Tauri desktop shell — in a plain browser, or
inside IntelliJ's embedded Chromium (JCEF). This is **additive**; the desktop app
is unchanged.

**It is not an "IPC → WebSocket" rewrite.** The frontend always talks to the
backend through one seam: Tauri's injected `window.__TAURI_INTERNALS__`
(`invoke` / `listen`). There are two implementations of that one contract:

- **Desktop (unchanged):** Tauri injects the global; IPC is in-process via the WebView.
- **Outside Tauri:** `src/lib/ipc-shim.ts` detects the global is absent and installs a
  compatible one that routes `invoke`/`listen` over a localhost **WebSocket** to
  `rssh-server`. Inside the real app the shim is a no-op.

So the 90+ `invoke` / 25+ `listen` call sites are untouched and the desktop
transport stays byte-identical. On the Rust side the *same engine* is reused via a
`Host` enum (`src-tauri/src/emitter.rs`): Tauri commands pass `Host::Tauri`
(→ `app.emit` / `app.state`), the headless server passes `Host::Headless` (→ ws push).

### Pieces

```
src-tauri/src/server.rs        # headless adapter: HTTP (embedded UI) + ws (IPC) on one port
src-tauri/src/emitter.rs       # Host enum (Tauri | headless sink)
src-tauri/src/server_main.rs   # the `rssh-server` binary entry (--features server; kept out of src/bin/ so the Tauri bundler ignores it)
src/lib/ipc-shim.ts            # Tauri-IPC shim over ws (no-op inside Tauri)
idea-plugin/                   # IntelliJ plugin (JCEF tool window)
```

`rssh-server` is self-contained: `npm run build`'s output is embedded into the binary
(`include_dir!`), and a single loopback port serves both the UI (HTTP) and the IPC
(WebSocket, guarded by a per-launch token).

### Run in a browser

```bash
npm run build
node scripts/dev-browser.mjs   # builds + runs rssh-server, prints a localhost URL to open
```

### IDEA plugin

```bash
npm run build
cargo build --release --manifest-path src-tauri/Cargo.toml --features server --bin rssh-server
export RSSH_SERVER_BIN="$PWD/src-tauri/target/release/rssh-server"
# open idea-plugin/ in IDEA → run the `runIde` Gradle task → open the "RSSH" tool window
```

Package: `cd idea-plugin && ./gradlew buildPlugin` → a zip under `build/distributions/`.
Ship it on a GitHub release; install via Settings → Plugins → ⚙ → Install Plugin from
Disk. Details in `idea-plugin/README.md`.

## Project Structure

```
src/                          # frontend (Svelte 5)
  lib/
    stores/app.svelte.ts      # reactive state
    components/               # UI components
  styles/global.css           # theme tokens
src-tauri/                    # backend (Rust)
  src/
    main.rs                   # entry point
    lib.rs                    # Tauri app builder, IPC commands
    models.rs                 # domain types
    error.rs                  # error types
    state.rs                  # AppState (DB + sessions)
    db/                       # SQLite CRUD
    ssh/                      # russh client, config parser, forwarding, SFTP
    commands/                 # Tauri command handlers
    terminal/                 # PTY + asciicast recorder
    crypto.rs                 # config encryption
    sync/                     # GitHub sync
  src/bin/rssh.rs             # CLI binary (behind `cli` feature flag)
  gen/android/                # Android build files
```

## Release

Push a git tag to trigger CI:

```bash
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions builds all platforms and creates a draft release. Artifact naming:

```
rssh-{version}-{os}-{arch}.{ext}
```

Version is derived from the git tag. No need to manually update `tauri.conf.json` or `Cargo.toml` -- CI syncs them automatically.

## Code Style

- Rust: `cargo fmt` + `cargo clippy`
- Frontend: default Svelte/TypeScript conventions
- Commits: explain *why*, not *what*
