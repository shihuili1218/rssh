# RSSH IntelliJ plugin

Runs the full RSSH web UI (terminal + signature color bars + AI panel + SFTP) in
a JCEF tool window, backed by the headless `rssh-server`. Shares your existing
rssh hosts / keys / settings (same data directory).

## Status

**Builds** — `./gradlew buildPlugin` produces `build/distributions/rssh-idea-0.1.0.zip`
(~8 MB, with the `rssh-server` binary bundled inside). Compiled against a local
IntelliJ IDEA CE **2026.1 / build 261** (so all JCEF / `JBCefJSQuery` APIs are
real, not guessed). **Runtime not yet exercised** here (no headless GUI) — the
tool window rendering, `RsshBridge` JS injection, and SFTP picks need a `runIde`
or an install to confirm.

Build specifics that made it work against a current IDE (already in
`build.gradle.kts`):
- builds against the **locally-installed IDE** via `local(...)` (no SDK download);
  override with `-PrsshIde=/path/to/Your IDE.app`.
- `jvmToolchain(21)` (261 runs JBR 21).
- `-Xskip-metadata-version-check` — the 261 platform classes are compiled with a
  newer Kotlin than the kotlin-gradle-plugin here; without this the compiler
  crashes on them.
- `buildSearchableOptions = false` — no Settings UI to index.
- Gradle pinned to **8.10.2** via the wrapper (IntelliJ plugin 2.1.0 predates Gradle 9).

## How it works

`rssh-server` is self-contained: the built frontend is embedded into the binary
(`include_dir!`), and one local port serves **both** the UI over HTTP and the IPC
over WebSocket. The plugin spawns the binary, reads its `{port,token}` line, and
points `JBCefBrowser` at `http://127.0.0.1:<port>/?rsshPort=<port>&rsshToken=<token>`.
The frontend's IPC shim (`src/lib/ipc-shim.ts`) reads those query params and routes
`invoke`/`listen` over the ws — no changes to the 90+ frontend call sites.

## Build order (the frontend is embedded at compile time)

```sh
npm run build                                  # → dist/
cargo build --release --manifest-path src-tauri/Cargo.toml \
      --features server --bin rssh-server      # embeds dist/ into the binary
```

## Dev run

```sh
export RSSH_SERVER_BIN="$PWD/src-tauri/target/release/rssh-server"   # or .../debug/
# open idea-plugin/ in IDEA (auto-imports Gradle), then run the `runIde` task
# — or via the checked-in wrapper (pinned to Gradle 8.10.2):
#   (cd idea-plugin && ./gradlew runIde)
```

A sandbox IDE launches; open the **RSSH** tool window (docked bottom).

## Package for a GitHub release (no Marketplace)

```sh
# processResources auto-copies src-tauri/target/release/rssh-server → resources/bin
(cd idea-plugin && ./gradlew buildPlugin)      # → build/distributions/rssh-idea-*.zip
```

Attach the zip to a GitHub release. Users install via **Settings → Plugins → ⚙ →
Install Plugin from Disk → pick the zip → restart**.

Cross-platform: build `rssh-server` on each OS; either ship one zip per OS, or put
the binaries under `resources/bin/<os-arch>/` and extend `resolveBinary()`.

## File dialogs & multi-window

- **SFTP disk transfers** (`sftp_pick_folder` / `sftp_pick_open_files`): wired via
  `RsshBridge.kt` — it injects `window.__RSSH_PICK__` (a `JBCefJSQuery` backed by
  IntelliJ's `FileChooser`) so the picks return real local paths and the server's
  streaming `sftp_download_to` / `sftp_upload_from` do the transfer. **Unverified**
  (no IntelliJ SDK here): check the `JBCefJSQuery` / `FileChooser` call shapes on
  your first `runIde`.
- **Config export / import, AI audit save**: handled by the IPC shim with web APIs
  (Blob download / `<input type=file>`). Works in a browser; in JCEF a built-in or
  plugin-registered download/dialog handler may be needed.
- **"Open in new window"**: the shim degrades it to a new browser/JCEF window of the
  same app (shared server ⇒ shared sessions), handing the cloned tab over via
  localStorage. `split` tiling stays desktop-only.
