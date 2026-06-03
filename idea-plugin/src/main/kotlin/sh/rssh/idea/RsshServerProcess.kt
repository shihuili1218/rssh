package sh.rssh.idea

import com.intellij.openapi.Disposable
import com.intellij.openapi.diagnostic.logger
import com.intellij.openapi.util.Disposer
import java.io.BufferedReader
import java.io.File
import java.io.InputStreamReader
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.concurrent.TimeUnit

/** Where the running rssh-server can be reached. */
data class ServerEndpoint(val port: Int, val token: String)

/**
 * Spawns and owns the headless `rssh-server` child process. The server prints a
 * single `{"port":..,"token":..}` JSON line on stdout at startup; we read it to
 * learn where to point JCEF. Killed when the registered parent is disposed
 * (tool window / project close, IDE exit).
 */
class RsshServerProcess private constructor(private val process: Process) : Disposable {

    override fun dispose() {
        process.destroy()
        if (!process.waitFor(2, TimeUnit.SECONDS)) {
            process.destroyForcibly()
        }
    }

    companion object {
        private val LOG = logger<RsshServerProcess>()

        /** Spawn the server and block (call OFF the EDT) until it reports its endpoint. */
        fun start(parent: Disposable): Pair<RsshServerProcess, ServerEndpoint> {
            val bin = resolveBinary()
            LOG.info("starting rssh-server: ${bin.absolutePath}")
            val proc = ProcessBuilder(bin.absolutePath)
                // Drain the child's stderr into ours. rssh-server logs to stderr at
                // info level (env_logger); an unread pipe buffer (~64KB) would fill
                // and block the server, freezing tool-window startup. stdout stays a
                // pipe — we read the one endpoint line below and it's quiet after.
                .redirectError(ProcessBuilder.Redirect.INHERIT)
                .start()
            val handle = RsshServerProcess(proc)
            Disposer.register(parent, handle)

            val line = BufferedReader(InputStreamReader(proc.inputStream)).readLine()
                ?: error("rssh-server exited before printing its endpoint")
            return handle to parseEndpoint(line)
        }

        /**
         * Dev: set RSSH_SERVER_BIN to a locally-built binary
         * (`src-tauri/target/{debug,release}/rssh-server`).
         * Release: extract the bundled binary to a STABLE path under the shared
         * data dir (`~/.rssh/bin/rssh-server`).
         *
         * Why a fixed path, not a temp file: macOS keychain "Always Allow" only
         * persists for a stable code identity at a stable location. The binary is
         * ad-hoc signed with a constant cdhash, but extracting it to a random
         * `/tmp` path on every launch made the grant never stick — so the
         * master-key prompt fired again and again. A fixed path + constant cdhash
         * lets the grant hold (one "Always Allow", then quiet).
         */
        private fun resolveBinary(): File {
            System.getenv("RSSH_SERVER_BIN")?.let { p ->
                val f = File(p)
                if (f.exists()) return f
            }
            val exe = if (System.getProperty("os.name").startsWith("Windows", ignoreCase = true)) {
                "rssh-server.exe"
            } else {
                "rssh-server"
            }
            val url = RsshServerProcess::class.java.getResource("/bin/$exe")
                ?: error(
                    "rssh-server binary not found. Dev: set RSSH_SERVER_BIN. " +
                        "Release: bundle it under resources/bin/$exe."
                )

            // ~/.rssh is the shared data dir; bin/ is plugin-owned (the desktop app
            // and CLI live elsewhere), so there is no collision.
            val dir = File(System.getProperty("user.home"), ".rssh/bin").apply { mkdirs() }
            val dest = File(dir, exe)

            // Reuse key = this plugin's version. A release bundles exactly one
            // binary, so the version identifies its content — keyed on version, NOT
            // file size (equal size doesn't prove equal bytes). On a match we reuse
            // the extracted file untouched, so we never rewrite or clobber a copy
            // that may be executing right now.
            val version = runCatching {
                com.intellij.ide.plugins.PluginManagerCore
                    .getPlugin(com.intellij.openapi.extensions.PluginId.getId("sh.rssh.idea"))
                    ?.version
            }.getOrNull() ?: "dev"
            val stamp = File(dir, ".$exe.version")
            val current = dest.exists() &&
                runCatching { stamp.readText().trim() }.getOrNull() == version

            if (!current) {
                // Changed or missing: write a temp sibling, then atomically rename it
                // into place. The atomic replace is safe even if an old process is
                // still running — it keeps its own inode; new launches get the new
                // one. (A rare cross-version race — two IDE windows on DIFFERENT
                // plugin versions extracting at once — could have one launch exec the
                // other's binary; both are valid rssh-servers, so harm is negligible.)
                val tmp = File.createTempFile("rssh-server", ".tmp", dir)
                try {
                    url.openStream().use { input -> tmp.outputStream().use { input.copyTo(it) } }
                    tmp.setExecutable(true, false)
                    Files.move(
                        tmp.toPath(),
                        dest.toPath(),
                        StandardCopyOption.REPLACE_EXISTING,
                        StandardCopyOption.ATOMIC_MOVE,
                    )
                    stamp.writeText(version)
                } catch (t: Throwable) {
                    tmp.delete()
                    throw t
                }
            }

            // Guarantee the result is actually runnable — a silently non-executable
            // file would fail opaquely at spawn time.
            dest.setExecutable(true, false)
            if (!dest.canExecute()) {
                error("rssh-server is present but not executable at ${dest.absolutePath}")
            }
            return dest
        }

        private fun parseEndpoint(json: String): ServerEndpoint {
            val port = Regex("\"port\"\\s*:\\s*(\\d+)").find(json)?.groupValues?.get(1)?.toInt()
                ?: error("rssh-server endpoint line missing port: $json")
            val token = Regex("\"token\"\\s*:\\s*\"([^\"]+)\"").find(json)?.groupValues?.get(1)
                ?: error("rssh-server endpoint line missing token: $json")
            return ServerEndpoint(port, token)
        }
    }
}
