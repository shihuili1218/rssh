<script lang="ts">
    import {onDestroy, onMount, untrack} from "svelte";
    import {Terminal} from "@xterm/xterm";
    import {FitAddon} from "@xterm/addon-fit";
    import {SearchAddon} from "@xterm/addon-search";
    import {Unicode11Addon} from "@xterm/addon-unicode11";
    import {invoke} from "@tauri-apps/api/core";
    import {listen, type UnlistenFn} from "@tauri-apps/api/event";
    import type {HighlightRule} from "../stores/app.svelte.ts";
    import * as app from "../stores/app.svelte.ts";
    import MobileKeybar from "./MobileKeybar.svelte";

    const ANSI: Record<string, string> = {
        red: "\x1b[31m", green: "\x1b[32m", yellow: "\x1b[33m",
        blue: "\x1b[34m", magenta: "\x1b[35m", cyan: "\x1b[36m", white: "\x1b[37m",
        brightRed: "\x1b[1;31m", brightGreen: "\x1b[1;32m", brightYellow: "\x1b[1;33m",
        brightBlue: "\x1b[1;34m", brightMagenta: "\x1b[1;35m", brightCyan: "\x1b[1;36m", brightWhite: "\x1b[1;37m",
    };
    const RST = "\x1b[0m";

    let hlRules = $state<HighlightRule[]>([]);
    let hlRegex: RegExp | null = null;

    function buildHighlightRegex(rules: HighlightRule[]) {
        const enabled = rules.filter(r => r.enabled && r.keyword);
        if (!enabled.length) {
            hlRegex = null;
            return;
        }
        const escaped = enabled.map(r => r.keyword.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
        hlRegex = new RegExp(escaped.join("|"), "gi");
    }

    function hlReplace(plain: string): string {
        if (!hlRegex) return plain;
        return plain.replace(hlRegex, (match) => {
            const rule = hlRules.find(r => r.enabled && r.keyword.toLowerCase() === match.toLowerCase());
            if (!rule) return match;
            const code = ANSI[rule.color] ?? "";
            return code + match + RST;
        });
    }

    function applyHighlights(text: string): string {
        if (!hlRegex || !hlRules.length) return text;
        // Skip escape sequences — only highlight plain text between them.
        // CSI: ESC [ params letter, OSC: ESC ] ... BEL/ST, simple: ESC + char
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
            out += rest.slice(esc); // incomplete escape sequence — pass through untouched
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

    async function oscOpenProfile(name: string) {
        const profiles = await invoke<any[]>("list_profiles");
        const p = profiles.find(x => x.name.toLowerCase() === name.toLowerCase());
        if (!p) { terminal?.write(`\r\n\x1b[31mProfile '${name}' not found\x1b[0m\r\n`); return; }
        let cred: any = null;
        if (p.credential_id) {
            try { cred = await invoke<any>("get_credential", {id: p.credential_id}); } catch {}
        }
        const tid = `ssh:${crypto.randomUUID()}`;
        app.addTab({
            id: tid, type: "ssh", label: p.name,
            meta: {
                profileId: p.id, host: p.host, port: String(p.port),
                username: cred?.username ?? "", authType: cred?.type ?? "password",
                secret: cred?.secret ?? "",
            },
        });
    }

    async function oscOpenForward(name: string) {
        const forwards = await invoke<any[]>("list_forwards");
        const f = forwards.find(x => x.name.toLowerCase() === name.toLowerCase());
        if (!f) { terminal?.write(`\r\n\x1b[31mForward '${name}' not found\x1b[0m\r\n`); return; }
        let profileName = "?";
        try {
            const p = await invoke<any>("get_profile", {id: f.profile_id});
            profileName = p.name;
        } catch {}
        const tid = `fwd:${f.id}:${Date.now()}`;
        app.addTab({
            id: tid, type: "forward", label: f.name,
            meta: {
                forwardId: f.id, name: f.name, forwardType: f.type,
                localPort: String(f.local_port), remoteHost: f.remote_host,
                remotePort: String(f.remote_port), profileName,
            },
        });
    }
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
    let unlisteners: UnlistenFn[] = [];
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

    /* Listen for external search requests (from tab context menu) */
    let _lastSearchN = 0;
    $effect(() => {
        const req = app.searchRequest();
        if (req && req.tabId === tabId && req.n !== _lastSearchN) {
            _lastSearchN = req.n;
            openSearch();
        }
    });

    function closeSearch() {
        showSearch = false;
        searchAddon?.clearDecorations();
        terminal?.focus();
    }

    function doSearch() {
        if (searchQuery) searchAddon?.findNext(searchQuery);
    }

    function searchNext() {
        searchAddon?.findNext(searchQuery);
    }

    function searchPrev() {
        searchAddon?.findPrevious(searchQuery);
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

        // Intercept Ctrl/Cmd+F for search, Ctrl/Cmd+O for SFTP
        terminal.attachCustomKeyEventHandler((e: KeyboardEvent) => {
            if (e.type !== "keydown") return true;
            const mod = e.metaKey || e.ctrlKey;
            if (mod && e.key === "f") {
                e.preventDefault();
                openSearch();
                return false;
            }
            if (mod && e.key === "o" && !isLocal && !app.isMobile) {
                e.preventDefault();
                app.navigate("sftp");
                return false;
            }
            if (mod && e.key === "s") {
                e.preventDefault();
                app.openSnippetPicker();
                return false;
            }
            return true;
        });

        // OSC 7337: rssh CLI → app integration (open profile / forward)
        terminal.parser.registerOscHandler(7337, (data: string) => {
            const sep = data.indexOf(":");
            if (sep < 0) return false;
            const kind = data.slice(0, sep);
            const name = data.slice(sep + 1);
            if (kind === "open") oscOpenProfile(name);
            else if (kind === "fwd") oscOpenForward(name);
            return true;
        });

        // Load highlight rules
        try {
            hlRules = await app.loadHighlights();
            buildHighlightRegex(hlRules);
        } catch { /* non-fatal */
        }

        const decoder = new TextDecoder("utf-8");

        // Helper: wire data + close events for a session
        async function wireSession(sid: string) {
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
            }));
        }

        // Connect
        if (isLocal) {
            try {
                sessionId = await invoke<string>("pty_spawn", {cols: terminal.cols, rows: terminal.rows});
            } catch (e: any) {
                terminal.write(`\x1b[31mLaunch failed: ${e}\x1b[0m\r\n`);
                return;
            }
            await wireSession(sessionId);
        } else {
            // SSH: listen on tabId FIRST for connection logs, then connect
            const logUn = await listen<number[]>(`ssh:data:${tabId}`, (ev) => {
                terminal.write(new Uint8Array(ev.payload));
            });

            // Keyboard-interactive auth prompts
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
                setupReconnect();
                return;
            }
            logUn(); authUn();
            await wireSession(sessionId);
        }

        const sid = sessionId!;

        // Wire input
        terminal.onData((data: string) => {
            if (!disconnected) {
                const d = processInput(data);
                invoke(writeCmd, {sessionId: sid, data: Array.from(new TextEncoder().encode(d))});
            }
        });
        terminal.onResize(({cols, rows}) => {
            if (!disconnected) invoke(resizeCmd, {sessionId: sid, cols, rows});
        });

        setupReconnect();

        terminal.onTitleChange((title) => {
            if (!title) return;
            if (isLocal) app.updateTabLabel(tabId, title);
            else app.setTerminalTitle(tabId, title);
        });

        resizeObs = new ResizeObserver(() => fitAddon?.fit());
        resizeObs.observe(containerEl);
        requestAnimationFrame(() => {
            fitAddon.fit();
            if (!disconnected) invoke(resizeCmd, {sessionId: sid, cols: terminal.cols, rows: terminal.rows});
        });
    });

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
        // On any keypress when disconnected → reconnect
        const handler = terminal.onData(() => {
            if (!disconnected) return;
            handler.dispose();
            reconnect();
        });
    }

    async function reconnect() {
        // Clean up old listeners
        unlisteners.forEach(u => u());
        unlisteners = [];
        disconnected = false;
        sessionId = null;

        terminal.write("\r\n\x1b[36mReconnecting ...\x1b[0m\r\n");

        if (isLocal) {
            try {
                sessionId = await invoke<string>("pty_spawn", {cols: terminal.cols, rows: terminal.rows});
                const sid = sessionId;
                const decoder = new TextDecoder();
                unlisteners.push(await listen<number[]>(`pty:data:${sid}`, (ev) => {
                    const raw = new Uint8Array(ev.payload);
                    terminal.write(hlRegex ? applyHighlights(decoder.decode(raw, { stream: true })) : raw);
                }));
                unlisteners.push(await listen(`pty:close:${sid}`, () => {
                    disconnected = true;
                    terminal.write("\r\n\x1b[31m--- Disconnected ---\x1b[0m\r\n");
                    terminal.write("\x1b[90mPress any key to reconnect.\x1b[0m\r\n");
                    setupReconnect();
                }));
                terminal.onData((data: string) => {
                    if (!disconnected) invoke("pty_write", {sessionId: sid, data: Array.from(new TextEncoder().encode(processInput(data)))});
                });
            } catch (e: any) {
                terminal.write(`\x1b[31mReconnect failed: ${e}\x1b[0m\r\n`);
                disconnected = true;
                setupReconnect();
            }
        } else {
            const logUn = await listen<number[]>(`ssh:data:${tabId}`, (ev) => {
                terminal.write(new Uint8Array(ev.payload));
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
                logUn();
                const sid = sessionId;
                const decoder = new TextDecoder();
                unlisteners.push(await listen<number[]>(`ssh:data:${sid}`, (ev) => {
                    const raw = new Uint8Array(ev.payload);
                    terminal.write(hlRegex ? applyHighlights(decoder.decode(raw, { stream: true })) : raw);
                }));
                unlisteners.push(await listen(`ssh:close:${sid}`, () => {
                    disconnected = true;
                    terminal.write("\r\n\x1b[31m--- Disconnected ---\x1b[0m\r\n");
                    terminal.write("\x1b[90mPress any key to reconnect.\x1b[0m\r\n");
                    setupReconnect();
                }));
                terminal.onData((data: string) => {
                    if (!disconnected) invoke("ssh_write", {sessionId: sid, data: Array.from(new TextEncoder().encode(processInput(data)))});
                });
                terminal.onResize(({cols, rows}) => {
                    if (!disconnected) invoke("ssh_resize", {sessionId: sid, cols, rows});
                });
            } catch (e: any) {
                logUn();
                terminal.write(`\x1b[31mReconnect failed: ${e}\x1b[0m\r\n`);
                terminal.write("\x1b[90mPress any key to reconnect.\x1b[0m\r\n");
                disconnected = true;
                setupReconnect();
            }
        }
    }

    // Register session in global registry for broadcast
    $effect(() => {
        if (sessionId && !disconnected) {
            untrack(() => app.registerSession({ tabId, sessionId, type: tabType }));
        } else {
            untrack(() => app.unregisterSession(tabId));
        }
    });

    // Focus terminal + register writer when this tab becomes active
    $effect(() => {
        if (app.activeTabId() === tabId && !app.settingsActive()) {
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
        resizeObs?.disconnect();
        app.unregisterTerminalWriter();
        app.unregisterSession(tabId);
        if (sessionId && !disconnected) {
            const cmd = isLocal ? "pty_close" : "ssh_disconnect";
            invoke(cmd, {sessionId}).catch(() => {
            });
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
    <div class="term-wrap" bind:this={containerEl}></div>
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
    }

    .term-wrap :global(.xterm) {
        height: 100%;
        padding: 4px;
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
