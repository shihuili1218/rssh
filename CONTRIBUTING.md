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
