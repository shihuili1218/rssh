package sh.rssh.idea

import com.intellij.icons.AllIcons
import com.intellij.openapi.Disposable
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.logger
import com.intellij.openapi.project.DumbAware
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.Disposer
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.openapi.wm.ex.ToolWindowEx
import com.intellij.openapi.wm.ex.ToolWindowManagerListener
import com.intellij.ui.content.Content
import com.intellij.ui.content.ContentFactory
import com.intellij.ui.jcef.JBCefApp
import com.intellij.ui.jcef.JBCefBrowser
import javax.swing.JLabel
import javax.swing.SwingConstants

/**
 * The "RSSH" tool window: spawn the headless server, then render the rssh web UI
 * in a JCEF browser pointed at `http://127.0.0.1:<port>/`. The server serves the
 * frontend AND the IPC websocket on that one port; the IPC shim reads the port +
 * token from the query string.
 *
 * The session lifecycle (server + browser) is owned by [RsshToolWindowController]
 * so the title-bar Close button can tear it down on demand.
 *
 * NOTE: authored without an IntelliJ SDK to compile against. `ToolWindowEx
 * .setTitleActions(List<AnAction>)` and `ToolWindowManagerListener.toolWindowShown
 * (ToolWindow)` are stable across recent IDEs (242+), but verify the exact
 * signatures when you first `runIde` and adjust if your platform version differs.
 */
class RsshToolWindowFactory : ToolWindowFactory, DumbAware {

    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        if (!JBCefApp.isSupported()) {
            val factory = ContentFactory.getInstance()
            toolWindow.contentManager.addContent(
                factory.createContent(centered("JCEF is not available in this IDE build."), "", false)
            )
            return
        }
        RsshToolWindowController(project, toolWindow).install()
    }
}

/**
 * Owns the tool window's rssh session. The server + JCEF browser live under a
 * per-session [Disposable] (a child of `toolWindow.disposable`, NOT registered on
 * it directly) so the Close button can dispose exactly that session. Project
 * close / IDE exit still cascades through the parent chain.
 *
 * Close (✕) kills the session and hides the tool window; re-showing it from the
 * sidebar spawns a fresh session. `createToolWindowContent` runs only once, so
 * (re)launch is driven off `toolWindowShown` and guarded by [session] state —
 * first open and every reopen take the same [launch] path, no special case.
 *
 * All [session] reads/writes happen on the EDT (createToolWindowContent, the
 * action, and the listener all fire there), so the field needs no locking.
 */
private class RsshToolWindowController(
    private val project: Project,
    private val toolWindow: ToolWindow,
) {
    private val content: Content =
        ContentFactory.getInstance().createContent(centered("Starting rssh…"), "", false)

    /** Non-null while a session is running or starting; null once stopped. */
    private var session: Disposable? = null

    fun install() {
        toolWindow.contentManager.addContent(content)

        // Close (✕), sits next to the gear / hide buttons in the title bar.
        val closeAction = object : AnAction("Close RSSH", "Stop rssh-server and hide the tool window", AllIcons.Actions.Cancel) {
            override fun actionPerformed(e: AnActionEvent) = close()
        }
        (toolWindow as? ToolWindowEx)?.setTitleActions(listOf(closeAction))

        // A reopen after Close must respawn. Tie the subscription to the tool
        // window so it dies with the project.
        project.messageBus.connect(toolWindow.disposable).subscribe(
            ToolWindowManagerListener.TOPIC,
            object : ToolWindowManagerListener {
                override fun toolWindowShown(shown: ToolWindow) {
                    if (shown.id == toolWindow.id && session == null) launch()
                }
            },
        )

        launch() // first open
    }

    /** Spawn the server + browser under a fresh session disposable. No-op if one is already live. */
    private fun launch() {
        if (session != null) return
        val live = Disposer.newDisposable("rssh-session")
        Disposer.register(toolWindow.disposable, live)
        session = live
        content.component = centered("Starting rssh…")

        // Server startup blocks on a stdout read — must run off the EDT.
        ApplicationManager.getApplication().executeOnPooledThread {
            try {
                val (_, ep) = RsshServerProcess.start(live)
                val url = "http://127.0.0.1:${ep.port}/?rsshPort=${ep.port}&rsshToken=${ep.token}"
                ApplicationManager.getApplication().invokeLater {
                    if (session !== live) return@invokeLater // closed while starting
                    // Build the browser WITHOUT a URL, install the file-chooser
                    // bridge's load handler, THEN navigate. Constructing with the
                    // URL starts loading immediately, and a fast localhost load can
                    // fire onLoadEnd before the handler is registered — leaving
                    // window.__RSSH_PICK__ undefined and breaking SFTP disk
                    // transfers (the SPA loads once, so a missed onLoadEnd is lost).
                    val browser = JBCefBrowser()
                    Disposer.register(live, browser)
                    RsshBridge.install(browser, project)
                    browser.loadURL(url)
                    content.component = browser.component
                }
            } catch (t: Throwable) {
                LOG.warn("failed to start rssh-server", t)
                ApplicationManager.getApplication().invokeLater {
                    if (session === live) content.component = centered("Failed to start rssh-server: ${t.message}")
                }
            }
        }
    }

    /** Kill the running session (server + browser) and hide the tool window. */
    private fun close() {
        val live = session ?: return
        session = null
        Disposer.dispose(live) // destroy()s the server, disposes the browser + bridge
        content.component = centered("RSSH stopped")
        toolWindow.hide(null)
    }

    companion object {
        private val LOG = logger<RsshToolWindowController>()
    }
}

private fun centered(text: String): JLabel = JLabel(text, SwingConstants.CENTER)
