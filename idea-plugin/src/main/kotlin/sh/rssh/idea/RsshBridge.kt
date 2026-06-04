package sh.rssh.idea

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.logger
import com.intellij.openapi.fileChooser.FileChooser
import com.intellij.openapi.fileChooser.FileChooserDescriptorFactory
import com.intellij.openapi.project.Project
import com.intellij.ui.jcef.JBCefBrowser
import com.intellij.ui.jcef.JBCefBrowserBase
import com.intellij.ui.jcef.JBCefJSQuery
import org.cef.browser.CefBrowser
import org.cef.browser.CefFrame
import org.cef.handler.CefLoadHandlerAdapter

/**
 * Bridges the web frontend's native file-dialog needs to IntelliJ's FileChooser.
 *
 * The IPC shim ([src/lib/ipc-shim.ts]) calls `window.__RSSH_PICK__(kind)` for
 * `sftp_pick_folder` / `sftp_pick_open_files`. A bare browser can't return a real
 * local path (sandbox), so those transfers reject there; inside the plugin we
 * answer with actual path(s) via IntelliJ's chooser, and the server-side SFTP
 * streaming (`sftp_download_to` / `sftp_upload_from`) does the rest. This is the
 * "richer than a browser, so don't disable it" path.
 *
 * Protocol (matches `hostPick` in the shim):
 *   __RSSH_PICK__("folder") -> Promise<string | null>          (cancel -> null)
 *   __RSSH_PICK__("files")  -> Promise<string[] | null>        (cancel -> null)
 *
 * NOTE: authored without an IntelliJ SDK to compile against. The JBCefJSQuery /
 * FileChooser APIs used here are stable across recent IDEs, but verify the exact
 * signatures when you first `runIde` and adjust if your platform version differs.
 */
object RsshBridge {

    private val LOG = logger<RsshBridge>()

    fun install(browser: JBCefBrowser, project: Project?) {
        val query = JBCefJSQuery.create(browser as JBCefBrowserBase)

        // Off-EDT JS request -> show the chooser on the EDT -> JSON reply.
        query.addHandler { kind ->
            val json = try {
                pickPathsOnEdt(kind, project)
            } catch (t: Throwable) {
                // Don't let a real bridge/runtime failure masquerade silently as a
                // user cancel — log it. (JS still sees "null"/cancel; surfacing a
                // distinct error to the SPA would need a shim-side protocol change.)
                LOG.warn("RSSH file picker failed for kind=$kind", t)
                "null"
            }
            JBCefJSQuery.Response(json)
        }

        // Define window.__RSSH_PICK__ on every load (the SPA only loads once, but
        // a reload must not lose the bridge). The injected snippet wraps the
        // native query in a Promise whose resolve/reject the query drives.
        val define = """
            window.__RSSH_PICK__ = function(kind) {
                return new Promise(function(resolve, reject) {
                    ${query.inject(
                        "kind",
                        "function(response){ try { resolve(JSON.parse(response)); } catch (e) { resolve(null); } }",
                        "function(error_code, error_message){ reject(error_message || 'pick_failed'); }"
                    )}
                });
            };
        """.trimIndent()

        browser.jbCefClient.addLoadHandler(
            object : CefLoadHandlerAdapter() {
                override fun onLoadEnd(b: CefBrowser?, frame: CefFrame?, httpStatusCode: Int) {
                    if (frame != null && frame.isMain) {
                        browser.cefBrowser.executeJavaScript(define, browser.cefBrowser.url, 0)
                    }
                }
            },
            browser.cefBrowser,
        )
    }

    /** Show the chooser on the EDT and return a JSON string: `"path"`, `["p1",..]`, or `null`. */
    private fun pickPathsOnEdt(kind: String, project: Project?): String {
        var json = "null"
        ApplicationManager.getApplication().invokeAndWait {
            when (kind) {
                "folder" -> {
                    val desc = FileChooserDescriptorFactory.createSingleFolderDescriptor()
                        .withTitle("Select destination folder")
                    val chosen = FileChooser.chooseFile(desc, project, null)
                    if (chosen != null) json = jsonString(chosen.path)
                }
                "files" -> {
                    val desc = FileChooserDescriptorFactory.createMultipleFilesNoJarsDescriptor()
                        .withTitle("Select files to upload")
                    val chosen = FileChooser.chooseFiles(desc, project, null)
                    if (chosen.isNotEmpty()) {
                        json = chosen.joinToString(prefix = "[", postfix = "]") { jsonString(it.path) }
                    }
                }
            }
        }
        return json
    }

    /** Minimal JSON string encoder (paths may contain backslashes / quotes on Windows). */
    private fun jsonString(s: String): String {
        val sb = StringBuilder("\"")
        for (c in s) {
            when (c) {
                '\\' -> sb.append("\\\\")
                '"' -> sb.append("\\\"")
                '\n' -> sb.append("\\n")
                '\r' -> sb.append("\\r")
                '\t' -> sb.append("\\t")
                else -> sb.append(c)
            }
        }
        return sb.append("\"").toString()
    }
}
