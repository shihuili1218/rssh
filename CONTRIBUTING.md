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

### macOS desktop

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

### iOS

iOS release packaging runs on GitHub Actions; no signing certificate is required
on a contributor's machine. Before enabling it, register the identifier from
`src-tauri/tauri.conf.json` (`com.rssh.app` by default) as an explicit App ID.
The registered Bundle ID and the Tauri `identifier` must match exactly.

Configure these GitHub Actions secrets with the manual signing material:

- `APPLE_DEVELOPMENT_TEAM`: the 10-character Apple Developer Team ID
- `IOS_CERTIFICATE`: base64-encoded Apple Distribution `.p12`
- `IOS_CERTIFICATE_PASSWORD`: password used when exporting that `.p12`
- `IOS_MOBILE_PROVISION`: base64-encoded App Store Connect provisioning profile

Run **iOS Release** manually from the Actions tab to test the setup. Its optional
`build_number` input defaults to the monotonically increasing GitHub Actions run
number. To package iOS automatically for `v*` tags, set the repository variable
`IOS_BUILD_ENABLED` to `true`; without that opt-in, tag-triggered iOS jobs are
skipped and the existing desktop/Android release workflow is unchanged.

The workflow builds with `--export-method app-store-connect`, uploads the IPA as
a workflow artifact, and, for tags, appends it to the draft GitHub release. The
IPA is not uploaded to App Store Connect automatically. The CI helper copies the
required privacy manifest into the generated Xcode project before packaging.

iOS may suspend network sessions when RSSH enters the background; this project
does not claim a background execution mode it cannot guarantee.

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
    sync/                     # Remote sync (GitHub + WebDAV)
  src/bin/rssh.rs             # CLI binary (behind `cli` feature flag)
  gen/android/                # Android build files
  gen/apple/                  # iOS Xcode project and privacy manifest
```

## Release

Push a git tag to trigger CI:

```bash
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions builds desktop and Android artifacts and creates a draft release.
The separate **iOS Release** workflow adds a signed IPA only when explicitly run
or when `IOS_BUILD_ENABLED=true` enables tag builds.
Desktop/Android artifact naming:

```
rssh-{version}-{os}-{arch}.{ext}
```

The iOS Actions artifact is named `rssh-ios-{run_id}-build-{build_number}`;
the signed file inside it, and on a tagged draft release, is `RSSH.ipa`.

Version is derived from the git tag. No need to manually update `tauri.conf.json` or `Cargo.toml` -- CI syncs them automatically.

## Code Style

- Rust: `cargo fmt` + `cargo clippy`
- Frontend: default Svelte/TypeScript conventions
- Commits: explain *why*, not *what*
