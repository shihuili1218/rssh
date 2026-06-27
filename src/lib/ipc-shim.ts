/**
 * Tauri IPC shim — lets the rssh frontend run OUTSIDE the Tauri webview
 * (a plain browser, or IntelliJ's embedded JCEF) by emulating the single
 * global that the whole `@tauri-apps/api` surface funnels through:
 * `window.__TAURI_INTERNALS__`.
 *
 * When that global is absent we install one that routes `invoke` and the
 * event protocol (`plugin:event|listen` / `unlisten`, plus backend event
 * push) over a WebSocket to a headless rssh server. The 34 files that import
 * `@tauri-apps/api` need ZERO changes — there is one seam, not 34 (INV-1).
 *
 * In the real Tauri app this is a no-op: the webview already injects the
 * global, so `installTauriShim()` returns immediately (INV-2: desktop
 * behaviour unchanged).
 *
 * Wire protocol (JSON over a single WS):
 *   →  { type:"invoke",   id, cmd, args }
 *   ←  { type:"response",  id, ok:true,  result }
 *   ←  { type:"response",  id, ok:false, error }
 *   ←  { type:"event",     event, payload }
 */

type Pending = { resolve: (v: unknown) => void; reject: (e: unknown) => void };
type CallbackEntry = { cb: (payload: unknown) => void; once: boolean };

/**
 * Where to reach the headless server. The host (IDEA plugin) injects
 * `window.__RSSH_SERVER__`; in a plain browser we fall back to URL query
 * params, e.g. `?rsshPort=54321&rsshToken=abc`.
 */
function serverUrl(): string | null {
    const g = ((window as any).__RSSH_SERVER__ ?? {}) as { port?: number; token?: string };
    const qs = new URLSearchParams(location.search);
    const port = Number(g.port ?? qs.get("rsshPort") ?? 0);
    const token = String(g.token ?? qs.get("rsshToken") ?? "");
    if (!port || !token) return null;
    return `ws://127.0.0.1:${port}/?token=${encodeURIComponent(token)}`;
}

/**
 * When this window was opened by `open_tab_in_new_window`'s browser fallback,
 * the opener stashed the clone payload in localStorage under a nonce carried in
 * the URL hash. Promote it to `window.__rssh_clone` (what AppShell reads) before
 * the app mounts, then clean up so a manual reload doesn't re-clone.
 */
function consumeCloneHandoff(): void {
    const m = location.hash.match(/rsshClone=([A-Za-z0-9_]+)/);
    if (!m) return;
    const key = "__rssh_clone:" + m[1];
    try {
        const clone = localStorage.getItem(key);
        if (clone) (window as any).__rssh_clone = clone;
        localStorage.removeItem(key);
    } catch {
        /* localStorage may be unavailable; ignore */
    }
    history.replaceState(null, "", location.pathname + location.search);
}

export function installTauriShim(): void {
    // Real Tauri webview already provides this — never shim over it.
    if ((window as any).__TAURI_INTERNALS__) return;

    // Browser fallback for "open in new window": pick up a clone handoff if the
    // opener left one. Runs only off-Tauri (desktop uses a native init script).
    consumeCloneHandoff();

    const url = serverUrl();
    if (!url) {
        console.warn("[rssh-shim] no server coords (rsshPort/rsshToken); invoke/listen will fail");
        return;
    }

    let nextReqId = 1;
    let nextCbId = 1;
    let nextEventId = 1;
    const pending = new Map<number, Pending>();
    const callbacks = new Map<number, CallbackEntry>();
    const listeners = new Map<string, Set<number>>();                    // event name → callback ids
    const eventReg = new Map<number, { event: string; cbId: number }>(); // eventId → registration

    let socket: WebSocket | null = null;
    let closed = false; // set once the socket closes; no reconnect, so it's terminal
    const outbox: string[] = [];
    const send = (msg: object) => {
        const s = JSON.stringify(msg);
        if (socket && socket.readyState === WebSocket.OPEN) socket.send(s);
        else outbox.push(s);
    };

    function dispatchEvent(event: string, payload: unknown) {
        const ids = listeners.get(event);
        if (!ids) return;
        // Tauri delivers { event, id, payload } to each registered callback.
        for (const cbId of [...ids]) {
            const entry = callbacks.get(cbId);
            if (!entry) { ids.delete(cbId); continue; }
            entry.cb({ event, id: cbId, payload });
            if (entry.once) { callbacks.delete(cbId); ids.delete(cbId); }
        }
    }

    function connect() {
        socket = new WebSocket(url!);
        socket.onopen = () => { for (const m of outbox.splice(0)) socket!.send(m); };
        socket.onmessage = (ev) => {
            let msg: any;
            try { msg = JSON.parse(ev.data as string); } catch { return; }
            if (msg.type === "response") {
                const p = pending.get(msg.id);
                if (!p) return;
                pending.delete(msg.id);
                if (msg.ok) p.resolve(msg.result);
                else p.reject(msg.error);
            } else if (msg.type === "event") {
                dispatchEvent(msg.event, msg.payload);
            }
        };
        socket.onclose = () => {
            // Terminal: no reconnect. Fail every in-flight invoke; the app's own
            // error paths take over.
            closed = true;
            for (const [, p] of pending) p.reject(new Error("rssh server connection closed"));
            pending.clear();
        };
    }

    function wsInvoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
        // After close there is no reconnect, so a new invoke would otherwise sit in
        // `outbox` and its promise would never settle. Reject it instead of hanging.
        if (closed) return Promise.reject(new Error("rssh server connection closed"));
        const id = nextReqId++;
        return new Promise((resolve, reject) => {
            pending.set(id, { resolve, reject });
            send({ type: "invoke", id, cmd, args: args ?? {} });
        });
    }

    // Native file/folder pick: only the host (IDEA plugin) can return real local
    // paths for server-side streaming transfers. It injects `__RSSH_PICK__`; a
    // bare browser has no equivalent, so we reject with a clear, non-fatal error.
    function hostPick(kind: "folder" | "files"): Promise<unknown> {
        const bridge = (window as any).__RSSH_PICK__;
        if (typeof bridge === "function") return Promise.resolve(bridge(kind));
        return Promise.reject(
            "file_dialog_unavailable: disk transfers need the RSSH desktop app or the IDEA plugin file chooser",
        );
    }

    // Commands whose "backend" off-Tauri is the BROWSER itself, not the rssh
    // engine: clipboard, opening URLs / windows, native file dialogs. Served by
    // web APIs (or the host bridge) so they never hit the ws. Keeps INV-1 — the
    // frontend call sites are unchanged; this one seam absorbs the difference.
    const LOCAL: Record<string, (a: any) => Promise<unknown>> = {
        clipboard_read: () => navigator.clipboard.readText(),
        clipboard_write: (a) => navigator.clipboard.writeText(String(a.text ?? "")),
        open_external_url: async (a) => {
            const u = String(a.url ?? "");
            // Match the desktop AppError wire shape (external.rs) so errMsg() can
            // localize this with the {url} param, instead of the UI showing the
            // raw code. Bare-string throws below stay as-is — they're either UI
            // messages already (PLUGIN_UNSUPPORTED) or codeless bridge errors.
            if (!u.startsWith("http://") && !u.startsWith("https://"))
                throw `__rssh_err__|${JSON.stringify({ code: "window_non_https_url", params: { url: u } })}`;
            // `noreferrer` (not just `noopener`): the headless UI carries the
            // per-launch `?rsshToken=` in its URL, so without it the browser would
            // leak the token to the external site via the `Referer` header.
            window.open(u, "_blank", "noopener,noreferrer");
        },
        // No native multi-window off-Tauri: open a new browser window of the same
        // app (shared server ⇒ shared sessions, like the desktop's shared state),
        // handing the cloned tab over via localStorage. `split` tiling is desktop-only.
        open_tab_in_new_window: async (a) => {
            const nonce = (crypto.randomUUID?.() ?? String(nextReqId++)).replace(/-/g, "");
            try {
                localStorage.setItem("__rssh_clone:" + nonce, String(a.clone ?? ""));
            } catch {
                /* localStorage unavailable; the new window just opens empty */
            }
            const w = window.open(location.pathname + location.search + "#rsshClone=" + nonce, "_blank");
            if (!w) {
                try {
                    localStorage.removeItem("__rssh_clone:" + nonce);
                } catch {
                    /* ignore */
                }
                throw "popup_blocked";
            }
        },
        sftp_pick_folder: () => hostPick("folder"),
        sftp_pick_open_files: () => hostPick("files"),
        // Window-plugin commands: off-Tauri the app lives in an IDE tool window
        // (or a browser tab), with no native window to drive. getCurrentWindow()
        // works (we supply `metadata` below); these calls just succeed with no
        // effect so the title / pin / decoration code paths don't reject. Title
        // and chrome belong to the host (IDE tab / browser tab).
        "plugin:window|set_title": async () => {},
        "plugin:window|set_always_on_top": async () => {},
        "plugin:window|set_decorations": async () => {},
        "plugin:window|is_decorated": async () => true,
    };

    function invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
        // The event-plugin pseudo-commands are bookkeeping — they never hit the
        // wire as real commands; the WS event push is what drives delivery.
        if (cmd === "plugin:event|listen") {
            const { event, handler } = (args ?? {}) as { event: string; handler: number };
            let set = listeners.get(event);
            if (!set) { set = new Set(); listeners.set(event, set); }
            set.add(handler);
            const eventId = nextEventId++;
            eventReg.set(eventId, { event, cbId: handler });
            return Promise.resolve(eventId);
        }
        if (cmd === "plugin:event|unlisten") {
            const { eventId } = (args ?? {}) as { eventId: number };
            const reg = eventReg.get(eventId);
            if (reg) { listeners.get(reg.event)?.delete(reg.cbId); eventReg.delete(eventId); }
            return Promise.resolve();
        }
        if (cmd === "plugin:event|emit" || cmd === "plugin:event|emit_to") {
            // Frontend→backend emit: not needed by the tracer; no-op.
            return Promise.resolve();
        }
        // Browser-environment commands (clipboard / open / file dialogs).
        const local = LOCAL[cmd];
        if (local) return local(args ?? {});
        // Everything else is a real engine command → route over the ws.
        return wsInvoke(cmd, args);
    }

    function transformCallback(cb: (payload: unknown) => void, once = false): number {
        const id = nextCbId++;
        callbacks.set(id, { cb, once });
        return id;
    }

    (window as any).__TAURI_INTERNALS__ = {
        invoke,
        transformCallback,
        unregisterCallback: (id: number) => { callbacks.delete(id); },
        // Asset URL rewriting is a Tauri-only concern; off-Tauri, pass through.
        convertFileSrc: (p: string) => p,
        // getCurrentWindow() / getCurrentWebview() read these labels SYNCHRONOUSLY
        // (`metadata.currentWindow.label`). Without them the getters throw at mount
        // (title effect, pin, WelcomeScreen). One "main" window mirrors the single
        // embedded webview; the window-plugin calls it makes are no-ops (LOCAL).
        metadata: {
            currentWindow: { label: "main" },
            currentWebview: { label: "main" },
        },
    };
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: (_event: string, eventId: number) => {
            const reg = eventReg.get(eventId);
            // Delete by the registration's authoritative event name, not the
            // caller's arg — a mismatched/empty `event` would otherwise fail to
            // remove the entry and leak the callback (matches the path above).
            if (reg) { listeners.get(reg.event)?.delete(reg.cbId); eventReg.delete(eventId); }
        },
    };

    connect();
}
