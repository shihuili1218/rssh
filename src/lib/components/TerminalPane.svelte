<script lang="ts">
    import {onDestroy, onMount, untrack} from "svelte";
    import {Terminal, type IDisposable} from "@xterm/xterm";
    import {FitAddon} from "@xterm/addon-fit";
    import {SearchAddon} from "@xterm/addon-search";
    import {Unicode11Addon} from "@xterm/addon-unicode11";
    import {invoke} from "@tauri-apps/api/core";
    import {listen, type UnlistenFn} from "@tauri-apps/api/event";
    import type {HighlightRule} from "../stores/app.svelte.ts";
    import * as app from "../stores/app.svelte.ts";
    import MobileKeybar from "./MobileKeybar.svelte";
    import {registerRsshOscHandlers} from "../osc/handler.ts";
    import {createCommandBlockTracker, type CommandBlockTracker} from "../terminal/command-blocks.ts";

    const RST = "\x1b[0m";

    /** Hex color → ANSI 24-bit true color escape. */
    function ansiColor(hex: string): string {
        const h = hex.replace("#", "");
        if (h.length !== 6) return "";
        const r = parseInt(h.slice(0, 2), 16);
        const g = parseInt(h.slice(2, 4), 16);
        const b = parseInt(h.slice(4, 6), 16);
        return `\x1b[38;2;${r};${g};${b}m`;
    }

    let hlRules = $state<HighlightRule[]>([]);
    let hlRegex: RegExp | null = null;

    function buildHighlightRegex(rules: HighlightRule[]) {
        const enabled = rules.filter(r => r.enabled && r.keyword);
        if (!enabled.length) { hlRegex = null; return; }
        const escaped = enabled.map(r => r.keyword.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
        hlRegex = new RegExp(escaped.join("|"), "gi");
    }

    function hlReplace(plain: string): string {
        if (!hlRegex) return plain;
        return plain.replace(hlRegex, (match) => {
            const rule = hlRules.find(r => r.enabled && r.keyword.toLowerCase() === match.toLowerCase());
            if (!rule) return match;
            const code = ansiColor(rule.color);
            return code + match + RST;
        });
    }

    function applyHighlights(text: string): string {
        if (!hlRegex || !hlRules.length) return text;
        const escRe = /\x1b(?:\[[0-9;?]*[A-Za-z@`]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[^\[\]])/g;
        let out = '', pos = 0, m;
        while ((m = escRe.exec(text)) !== null) {
            if (m.index > pos) out += hlReplace(text.slice(pos, m.index));
            out += m[0];
            pos = escRe.lastIndex;
        }
        const rest = text.slice(pos);
        const esc = rest.indexOf('\x1b');
        if (esc < 0) {
            out += hlReplace(rest);
        } else {
            if (esc > 0) out += hlReplace(rest.slice(0, esc));
            out += rest.slice(esc);
        }
        return out;
    }

    let {tabId, tabType, meta = {}}: {
        tabId: string;
        tabType: "ssh" | "local";
        meta: Record<string, string>;
    } = $props();

    let containerEl: HTMLDivElement;
    let searchInputEl: HTMLInputElement;

    type AuthPromptData = { name: string; instructions: string; prompts: { prompt: string; echo: boolean }[] };
    let authPrompt = $state<AuthPromptData | null>(null);
    let authValues = $state<string[]>([]);

    function submitAuth() {
        if (!authPrompt) return;
        invoke("ssh_auth_respond", { tabId, responses: authValues });
        authPrompt = null;
        authValues = [];
    }

    let terminal: Terminal;
    let fitAddon: FitAddon;
    let searchAddon: SearchAddon;
    let sessionId = $state<string | null>(null);
    let disconnected = $state(false);
    let showSearch = $state(false);
    let searchQuery = $state("");

    // Command block overlay state. `paintTick` is a dumb counter we bump
    // whenever something that affects the overlay changes — scroll, render,
    // block list change. The $derived below recomputes svg rects from it.
    let blockTracker: CommandBlockTracker | undefined;
    let paintTick = $state(0);
    let isAltBuffer = $state(false);

    type BlockRect = { id: number; y: number; h: number; color: string };

    const blockRects = $derived.by((): BlockRect[] => {
        paintTick; // dependency
        if (!app.commandBlockBar()) return [];
        if (!terminal || !blockTracker || !containerEl || isAltBuffer) return [];
        const firstRow = containerEl.querySelector(".xterm-rows")?.firstElementChild as HTMLElement | null;
        const rowHeight = firstRow?.offsetHeight ?? 0;
        if (!rowHeight) return [];
        const buf = terminal.buffer.active;
        const viewportY = buf.viewportY;
        const rows = terminal.rows;
        // For an unfinished block, its tail is wherever the cursor currently
        // sits (absolute row = baseY + cursorY). This grows naturally as the
        // shell writes output and stops at the real last line — not at the
        // bottom of the viewport.
        const cursorAbs = buf.baseY + buf.cursorY;
        const out: BlockRect[] = [];
        for (const b of blockTracker.blocks) {
            if (b.start.isDisposed) continue;
            const startLine = b.start.line;
            const endLine = b.end && !b.end.isDisposed ? b.end.line : cursorAbs;
            const top = Math.max(startLine, viewportY);
            const bot = Math.min(endLine, viewportY + rows - 1);
            if (top > bot) continue;
            out.push({
                id: b.id,
                y: (top - viewportY) * rowHeight,
                h: (bot - top + 1) * rowHeight,
                color: b.color,
            });
        }
        return out;
    });

    // Listener tracking — disposed on cleanup/reconnect
    let unlisteners: UnlistenFn[] = [];
    let dataDisposable: IDisposable | undefined;
    let resizeDisposable: IDisposable | undefined;
    let reconnectDisposable: IDisposable | undefined;
    let resizeObs: ResizeObserver;

    const isLocal = $derived(tabType === "local");
    const writeCmd = $derived(isLocal ? "pty_write" : "ssh_write");
    const resizeCmd = $derived(isLocal ? "pty_resize" : "ssh_resize");
    const dataEvent = $derived(isLocal ? "pty:data" : "ssh:data");
    const closeEvent = $derived(isLocal ? "pty:close" : "ssh:close");

    function openSearch() {
        showSearch = true;
        requestAnimationFrame(() => searchInputEl?.focus());
    }

    let _lastSearchN = 0;
    $effect(() => {
        const req = app.searchRequest();
        if (req && req.tabId === tabId && req.n !== _lastSearchN) {
            _lastSearchN = req.n;
            openSearch();
        }
    });

    function closeSearch() { showSearch = false; searchAddon?.clearDecorations(); terminal?.focus(); }
    function doSearch() { if (searchQuery) searchAddon?.findNext(searchQuery); }
    function searchNext() { searchAddon?.findNext(searchQuery); }
    function searchPrev() { searchAddon?.findPrevious(searchQuery); }

    // ─── Shared connect/wire helpers ───

    const decoder = new TextDecoder("utf-8");

    /** Wire Tauri event listeners for session data + close. */
    async function wireSessionEvents(sid: string) {
        unlisteners.push(await listen<number[]>(`${dataEvent}:${sid}`, (ev) => {
            const raw = new Uint8Array(ev.payload);
            if (hlRegex) {
                terminal.write(applyHighlights(decoder.decode(raw, { stream: true })));
            } else {
                terminal.write(raw);
            }
        }));
        unlisteners.push(await listen(`${closeEvent}:${sid}`, () => {
            disconnected = true;
            terminal.write("\r\n\x1b[31m--- Disconnected ---\x1b[0m\r\n");
            terminal.write("\x1b[90mPress any key to reconnect.\x1b[0m\r\n");
            setupReconnect();
        }));
    }

    /** Register terminal input + resize handlers (disposes old ones first). */
    function wireSessionInput(sid: string) {
        dataDisposable?.dispose();
        resizeDisposable?.dispose();

        dataDisposable = terminal.onData((data: string) => {
            if (!disconnected) {
                invoke(writeCmd, { sessionId: sid, data: Array.from(new TextEncoder().encode(processInput(data))) });
            }
        });
        resizeDisposable = terminal.onResize(({ cols, rows }) => {
            if (!disconnected) invoke(resizeCmd, { sessionId: sid, cols, rows });
        });
    }

    /** Full connect cycle: spawn session, wire events + input. */
    async function connectAndWire(): Promise<boolean> {
        // Cleanup previous
        unlisteners.forEach(u => u());
        unlisteners = [];
        disconnected = false;
        sessionId = null;

        if (isLocal) {
            try {
                sessionId = await invoke<string>("pty_spawn", { cols: terminal.cols, rows: terminal.rows });
            } catch (e: any) {
                terminal.write(`\x1b[31mLaunch failed: ${e}\x1b[0m\r\n`);
                return false;
            }
            await wireSessionEvents(sessionId);
        } else {
            // SSH: listen on tabId FIRST for connection logs
            const logUn = await listen<number[]>(`ssh:data:${tabId}`, (ev) => {
                terminal.write(new Uint8Array(ev.payload));
            });
            const authUn = await listen<AuthPromptData>(`ssh:auth_prompt:${tabId}`, (ev) => {
                authPrompt = ev.payload;
                authValues = ev.payload.prompts.map(() => "");
            });

            try {
                sessionId = await invoke<string>("ssh_connect", {
                    profileId: meta.profileId || null,
                    host: meta.profileId ? null : meta.host,
                    port: meta.profileId ? null : (Number(meta.port) || 22),
                    username: meta.profileId ? null : meta.username,
                    authType: meta.profileId ? null : meta.authType,
                    secret: meta.profileId ? null : (meta.secret || null),
                    logSessionId: tabId,
                    cols: terminal.cols, rows: terminal.rows,
                });
            } catch (e: any) {
                logUn(); authUn();
                terminal.write(`\x1b[31mConnection failed: ${e}\x1b[0m\r\n`);
                terminal.write("\x1b[90mPress any key to reconnect.\x1b[0m\r\n");
                disconnected = true;
                return false;
            }
            logUn(); authUn();
            await wireSessionEvents(sessionId);
        }

        wireSessionInput(sessionId!);

        // Sync initial size
        requestAnimationFrame(() => {
            fitAddon.fit();
            if (sessionId && !disconnected) {
                invoke(resizeCmd, { sessionId, cols: terminal.cols, rows: terminal.rows });
            }
        });

        return true;
    }

    function processInput(data: string): string {
        const ctrl = app.ctrlActive();
        const alt = app.altActive();
        if (!ctrl && !alt) return data;
        if (ctrl && data.length === 1) {
            const code = data.toUpperCase().charCodeAt(0);
            if (code >= 65 && code <= 90) data = String.fromCharCode(code - 64);
        }
        if (alt) data = '\x1b' + data;
        app.clearModifiers();
        return data;
    }

    function setupReconnect() {
        reconnectDisposable?.dispose();
        reconnectDisposable = terminal.onData(() => {
            if (!disconnected) return;
            reconnectDisposable?.dispose();
            reconnectDisposable = undefined;
            reconnect();
        });
    }

    async function reconnect() {
        terminal.write("\r\n\x1b[36mReconnecting ...\x1b[0m\r\n");
        const ok = await connectAndWire();
        setupReconnect();
        if (!ok) {
            disconnected = true;
        }
    }

    onMount(async () => {
        terminal = new Terminal({
            cursorBlink: true,
            fontSize: 13,
            fontFamily: "'JetBrainsMono Nerd Font', 'FiraCode Nerd Font', 'Hack Nerd Font', 'MesloLGS NF', 'Symbols Nerd Font Mono', Menlo, Monaco, 'Apple Color Emoji', 'Apple Symbols', 'PingFang SC', 'Courier New', monospace",
            allowProposedApi: true,
            theme: {
                background: "#2B2D3A", foreground: "#E0E5EC", cursor: "#4A6CF7",
                selectionBackground: "rgba(74,108,247,0.3)",
                black: "#1E2028", white: "#E0E5EC",
                red: "#E05555", green: "#4CB88A", yellow: "#DDAA33",
                blue: "#4A6CF7", magenta: "#9B72E4", cyan: "#2898AC",
                brightBlack: "#6B7A99", brightWhite: "#FFFFFF",
                brightRed: "#FF6B6B", brightGreen: "#6EDAA0", brightYellow: "#FFD060",
                brightBlue: "#6B8FF8", brightMagenta: "#B894F6", brightCyan: "#40C8E0",
            },
        });
        fitAddon = new FitAddon();
        searchAddon = new SearchAddon();
        terminal.loadAddon(fitAddon);
        terminal.loadAddon(searchAddon);
        terminal.loadAddon(new Unicode11Addon());
        terminal.open(containerEl);
        terminal.unicode.activeVersion = "11";
        fitAddon.fit();

        // Intercept Ctrl/Cmd+F for search, Ctrl/Cmd+O for SFTP, Ctrl/Cmd+S for snippets
        terminal.attachCustomKeyEventHandler((e: KeyboardEvent) => {
            if (e.type !== "keydown") return true;
            const mod = e.metaKey || e.ctrlKey;
            if (mod && e.key === "f") { e.preventDefault(); openSearch(); return false; }
            if (mod && e.key === "o" && !isLocal && !app.isMobile) { e.preventDefault(); app.navigate("sftp"); return false; }
            if (mod && e.key === "s") { e.preventDefault(); app.openSnippetPicker(); return false; }
            return true;
        });

        // OSC 7337: rssh CLI → app integration（处理逻辑见 lib/osc/handler.ts）
        registerRsshOscHandlers(terminal.parser, {
            error: (msg) => terminal?.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`),
        });

        // Command block tracker — marks Enter keypresses in normal buffer.
        blockTracker = createCommandBlockTracker(terminal);
        blockTracker.onChange(() => paintTick++);
        terminal.onScroll(() => paintTick++);
        terminal.onRender(() => paintTick++);
        terminal.buffer.onBufferChange((buf) => {
            isAltBuffer = buf.type === "alternate";
            paintTick++;
        });

        // Load highlight rules + the command-block-bar toggle. Awaiting
        // the toggle before `connectAndWire` runs avoids a first-frame
        // flash of the bar when the user has it disabled.
        try { hlRules = await app.loadHighlights(); buildHighlightRegex(hlRules); } catch {}
        await app.loadCommandBlockBar();

        // Connect
        await connectAndWire();
        setupReconnect();

        terminal.onTitleChange((title) => {
            if (!title) return;
            if (isLocal) app.updateTabLabel(tabId, title);
            else app.setTerminalTitle(tabId, title);
        });

        resizeObs = new ResizeObserver((entries) => {
            // Skip fitting when the container is hidden (display:none
            // collapses dimensions to zero) — fitting at 0×0 corrupts
            // xterm's column count and causes the narrow-tab bug.
            const { width, height } = entries[0].contentRect;
            if (width > 0 && height > 0) fitAddon?.fit();
        });
        resizeObs.observe(containerEl);
    });

    // Register session in global registry for broadcast
    $effect(() => {
        if (sessionId && !disconnected) {
            const sid = sessionId;
            untrack(() => app.registerSession({ tabId, sessionId: sid, type: tabType }));
        } else {
            untrack(() => app.unregisterSession(tabId));
        }
    });

    // When the block-bar toggle flips, xterm's left padding changes — it
    // needs to refit so columns recompute. The refit triggers xterm's own
    // render, which fires onRender → paintTick++, so the overlay resyncs
    // without us writing paintTick here (writing it here would make this
    // effect self-dependent via `++`, causing an update loop).
    $effect(() => {
        app.commandBlockBar(); // subscribe
        fitAddon?.fit();
    });

    // Focus terminal + register writer when this tab becomes active.
    // Double-rAF: the first frame lets the browser apply layout after
    // the pane switches from display:none → flex; the second frame
    // ensures the computed dimensions are stable before we fit.
    $effect(() => {
        if (app.activeTabId() === tabId && !app.settingsActive()) {
            requestAnimationFrame(() => requestAnimationFrame(() => fitAddon?.fit()));
            terminal?.focus();
            app.registerTerminalWriter((text: string) => {
                if (sessionId && !disconnected) {
                    const cmd = isLocal ? "pty_write" : "ssh_write";
                    invoke(cmd, {sessionId, data: Array.from(new TextEncoder().encode(text))});
                }
            });
        }
    });

    onDestroy(() => {
        unlisteners.forEach(u => u());
        dataDisposable?.dispose();
        resizeDisposable?.dispose();
        reconnectDisposable?.dispose();
        resizeObs?.disconnect();
        blockTracker?.dispose();
        app.unregisterTerminalWriter();
        app.unregisterSession(tabId);
        if (sessionId && !disconnected) {
            const cmd = isLocal ? "pty_close" : "ssh_disconnect";
            invoke(cmd, {sessionId}).catch(() => {});
        }
        terminal?.dispose();
    });
</script>

<div class="term-outer">
    {#if showSearch}
        <div class="search-bar">
            <input
                    bind:this={searchInputEl}
                    type="text"
                    bind:value={searchQuery}
                    placeholder="Search..."
                    oninput={doSearch}
                    onkeydown={(e) => {
          if (e.key === "Enter") { e.shiftKey ? searchPrev() : searchNext(); }
          if (e.key === "Escape") closeSearch();
        }}
            />
            <button class="search-btn" onclick={searchPrev} title="Previous">&#x25B2;</button>
            <button class="search-btn" onclick={searchNext} title="Next">&#x25BC;</button>
            <button class="search-btn" onclick={closeSearch} title="Close">&times;</button>
        </div>
    {/if}
    {#if authPrompt}
        <div class="auth-overlay">
            <div class="auth-dialog">
                {#if authPrompt.name}<div class="auth-title">{authPrompt.name}</div>{/if}
                {#if authPrompt.instructions}<div class="auth-instructions">{authPrompt.instructions}</div>{/if}
                {#each authPrompt.prompts as p, i}
                    <label class="auth-label">
                        <span>{p.prompt}</span>
                        <input
                            type={p.echo ? "text" : "password"}
                            bind:value={authValues[i]}
                            onkeydown={(e) => { if (e.key === "Enter") submitAuth(); }}
                        />
                    </label>
                {/each}
                <button class="auth-submit" onclick={submitAuth}>Submit</button>
            </div>
        </div>
    {/if}
    <div class="term-wrap" class:no-block-bar={!app.commandBlockBar()}>
        <div class="xterm-host" bind:this={containerEl}></div>
        {#if app.commandBlockBar()}
            <svg class="block-bar" aria-hidden="true">
                {#if isAltBuffer}
                    <rect x="0" y="0" width="3" height="100%" rx="1.5" fill="#6B7A99" opacity="0.5" />
                {:else}
                    {#each blockRects as r (r.id)}
                        <rect x="0" y={r.y} width="3" height={r.h} rx="1.5" fill={r.color} />
                    {/each}
                {/if}
            </svg>
        {/if}
    </div>
    {#if app.isMobile}
        <MobileKeybar />
    {/if}
</div>

<style>
    .term-outer {
        display: flex;
        flex-direction: column;
        width: 100%;
        height: 100%;
    }

    .term-wrap {
        flex: 1;
        min-height: 0;
        position: relative;
    }

    .xterm-host {
        width: 100%;
        height: 100%;
    }

    /* Widen left padding 4px → 12px to make room for the block bar.
       When the feature is off, restore the original symmetric 4px padding. */
    .term-wrap :global(.xterm) {
        height: 100%;
        padding: 4px 4px 4px 12px;
    }
    .term-wrap.no-block-bar :global(.xterm) {
        padding: 4px;
    }

    /* Overlay painted inside the enlarged left padding. Sits above xterm's
       canvas/DOM but ignores pointer events so text selection still works. */
    .block-bar {
        position: absolute;
        left: 5px;
        top: 4px;
        width: 4px;
        height: calc(100% - 8px);
        pointer-events: none;
        overflow: visible;
    }

    .search-bar {
        display: flex;
        align-items: center;
        gap: 4px;
        padding: 4px 8px;
        background: var(--surface);
        border-bottom: 1px solid var(--divider);
        flex-shrink: 0;
    }

    .search-bar input {
        flex: 1;
        padding: 4px 8px;
        font-size: 12px;
        border-radius: 4px;
        min-width: 0;
    }

    .search-btn {
        background: none;
        border: none;
        color: var(--text-sub);
        font-size: 12px;
        cursor: pointer;
        padding: 2px 6px;
        border-radius: 4px;
    }

    .search-btn:hover {
        background: var(--divider);
        color: var(--text);
    }

    .auth-overlay {
        position: absolute;
        inset: 0;
        z-index: 10;
        display: flex;
        align-items: center;
        justify-content: center;
        background: rgba(0,0,0,0.5);
    }

    .auth-dialog {
        background: var(--bg);
        border: 1px solid var(--divider);
        border-radius: 8px;
        padding: 20px;
        min-width: 300px;
        max-width: 400px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }

    .auth-title {
        font-size: 14px;
        font-weight: 600;
        color: var(--text);
    }

    .auth-instructions {
        font-size: 12px;
        color: var(--text-sub);
    }

    .auth-label {
        display: flex;
        flex-direction: column;
        gap: 4px;
        font-size: 12px;
        color: var(--text-sub);
    }

    .auth-label input {
        padding: 6px 8px;
        border-radius: 4px;
        font-size: 13px;
    }

    .auth-submit {
        align-self: flex-end;
        padding: 6px 16px;
        border-radius: 4px;
        border: none;
        background: var(--accent);
        color: var(--bg);
        font-size: 13px;
        cursor: pointer;
    }
</style>
