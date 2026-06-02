package sh.rssh.idea

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.logger
import com.intellij.openapi.project.DumbAware
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.Disposer
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
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
 */
class RsshToolWindowFactory : ToolWindowFactory, DumbAware {

    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val contentManager = toolWindow.contentManager
        val factory = ContentFactory.getInstance()

        if (!JBCefApp.isSupported()) {
            contentManager.addContent(
                factory.createContent(centered("JCEF is not available in this IDE build."), "", false)
            )
            return
        }

        // Placeholder shown while the server starts; swapped for the browser once ready.
        val content = factory.createContent(centered("Starting rssh…"), "", false)
        contentManager.addContent(content)

        // Server startup blocks on a stdout read — must run off the EDT.
        ApplicationManager.getApplication().executeOnPooledThread {
            try {
                val (_, ep) = RsshServerProcess.start(toolWindow.disposable)
                val url = "http://127.0.0.1:${ep.port}/?rsshPort=${ep.port}&rsshToken=${ep.token}"
                ApplicationManager.getApplication().invokeLater {
                    // Build the browser WITHOUT a URL, install the file-chooser
                    // bridge's load handler, THEN navigate. Constructing with the
                    // URL starts loading immediately, and a fast localhost load can
                    // fire onLoadEnd before the handler is registered — leaving
                    // window.__RSSH_PICK__ undefined and breaking SFTP disk
                    // transfers (the SPA loads once, so a missed onLoadEnd is lost).
                    val browser = JBCefBrowser()
                    Disposer.register(toolWindow.disposable, browser)
                    RsshBridge.install(browser, project)
                    browser.loadURL(url)
                    content.component = browser.component
                }
            } catch (t: Throwable) {
                LOG.warn("failed to start rssh-server", t)
                ApplicationManager.getApplication().invokeLater {
                    content.component = centered("Failed to start rssh-server: ${t.message}")
                }
            }
        }
    }

    private fun centered(text: String): JLabel =
        JLabel(text, SwingConstants.CENTER)

    companion object {
        private val LOG = logger<RsshToolWindowFactory>()
    }
}
