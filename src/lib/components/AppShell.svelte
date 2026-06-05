<script lang="ts">
    import {onMount} from "svelte";
    import {invoke} from "@tauri-apps/api/core";
    import {getCurrentWindow} from "@tauri-apps/api/window";
    import type {Profile, Tab, Group} from "../stores/app.svelte.ts";
    import * as app from "../stores/app.svelte.ts";
    import * as updates from "../stores/updates.svelte.ts";
    import HomeScreen from "./HomeScreen.svelte";
    import TerminalPane from "./TerminalPane.svelte";
    import ForwardPane from "./ForwardPane.svelte";
    import EditPane from "./EditPane.svelte";
    import SettingsLayout from "./SettingsLayout.svelte";
    import SftpBrowser from "./SftpBrowser.svelte";
    import DownloadsScreen from "./DownloadsScreen.svelte";
    import SnippetPicker from "./SnippetPicker.svelte";
    import * as transfers from "../stores/transfers.svelte.ts";
    import TabContextMenu, {type CtxMenuItem} from "./TabContextMenu.svelte";
    import MenuButton, {type NavItem, navItemKey} from "./MenuButton.svelte";
    import StripBar from "./StripBar.svelte";
    import ChatPanel from "../ai/ChatPanel.svelte";
    import * as ai from "../ai/store.svelte.ts";
    import {attachShortcuts, attachKeyup, type Shortcut} from "../keyboard/registry.ts";
    import {matchBinding, TAB_CYCLE} from "../keyboard/keymap.ts";
    import * as keymap from "../stores/keymap.svelte.ts";
    import {t, errMsg} from "../i18n/index.svelte.ts";
    import {toast} from "../stores/toast.svelte.ts";

    let drawerOpen = $state(false);
    let focusIdx = $state(-1);
    let tabCycling = $state(false);
    let profiles = $state<Profile[]>([]);
    let groups = $state<Group[]>([]);
    let sidebarTimer = 0;
    let menuCtx = $state<{ x: number; y: number; tab: Tab } | null>(null);
    let pinnedMenu = $state<{ x: number; y: number } | null>(null);
    let pinned = $state(false);

    function togglePin() {
        pinned = !pinned;
        getCurrentWindow().setAlwaysOnTop(pinned).catch(e => {
            console.error("setAlwaysOnTop failed:", e);
            pinned = !pinned;
        });
    }

    // Tab drag-and-drop
    let dragTabId = $state<string | null>(null);
    let dropTabId = $state<string | null>(null);

    /* ── 全局快捷键声明表 ── */
    function shortcutsTable(): Shortcut[] {
        return [
            {
                display: keymap.format("tab.close"),
                description: t("shortcut.tab.close"),
                skipInSettings: true,
                match: e => matchBinding(e, keymap.binding("tab.close")),
                handler: () => {
                    const id = app.activeTabId();
                    if (id === "home") return false;
                    app.closeTab(id);
                },
            },
            {
                display: keymap.format("tab.clone"),
                description: t("shortcut.tab.clone"),
                skipInSettings: true,
                match: e => matchBinding(e, keymap.binding("tab.clone")),
                handler: () => {
                    const tab = app.activeTab();
                    if (!tab || tab.type === "home") return false;
                    cloneTab(tab);
                },
            },
            {
                display: keymap.format("tab.openNewWindow"),
                description: t("shortcut.tab.open_new_window"),
                skipInSettings: true,
                match: e => matchBinding(e, keymap.binding("tab.openNewWindow")),
                handler: () => {
                    const tab = app.activeTab();
                    if (!tab || (tab.type !== "ssh" && tab.type !== "local") || app.isMobile) return false;
                    openInNewWindow(tab);
                },
            },
            {
                display: keymap.format("ai.toggle"),
                description: t("shortcut.ai.toggle"),
                skipInSettings: true,
                match: e => matchBinding(e, keymap.binding("ai.toggle")),
                handler: () => {
                    // Close always works; open only on a connected terminal tab
                    // (mirrors MobileKeybar's canOpenAi guard).
                    if (ai.isOpen()) { ai.closePanel(); return; }
                    const tab = app.activeTab();
                    const canOpen = !!tab && (tab.type === "ssh" || tab.type === "local") && !!app.sessionIdForTab(tab.id);
                    if (!canOpen) return false;
                    ai.openPanel();
                },
            },
            {
                display: "Ctrl+Tab / Ctrl+Shift+Tab",
                description: t("shortcut.tab.cycle"),
                // Exact match (excludes Ctrl+Alt/Meta+Tab) via the same data that
                // backs RESERVED, so the reserved set and this predicate can't drift.
                match: e => TAB_CYCLE.some(b => matchBinding(e, b)),
                handler: e => {
                    // Don't hijack keys while the user is recording a new binding.
                    if (keymap.recording()) return false;
                    const dir = e.shiftKey ? -1 : 1;
                    if (!tabCycling) {
                        tabCycling = true;
                        drawerOpen = true;
                        const idx = navItems.findIndex(item =>
                            item.kind === "tab" ? item.tab.id === app.activeTabId() && !app.settingsActive()
                            : item.kind === "settings" ? app.settingsActive()
                            : false
                        );
                        focusIdx = (idx + dir + navItems.length) % navItems.length;
                    } else {
                        focusIdx = (focusIdx + dir + navItems.length) % navItems.length;
                    }
                },
            },
            {
                display: "Esc",
                description: t("shortcut.tab.exit_cycle"),
                match: e => tabCycling && e.key === "Escape",
                handler: () => closeDrawer(),
            },
        ];
    }

    onMount(() => {
        keymap.init();
        app.loadProfiles().then(p => profiles = p);
        app.loadGroups().then(g => groups = g);
        // Crash recovery: reconcile with empty list tells the backend
        // "no sessions are alive" so it cleans up any orphaned resources
        // from a previous crash or hot-reload.
        //
        // Skip this in cloned windows (window.__rssh_clone is set by
        // open_tab_in_new_window) and AI handoff windows (window.__rssh_ai_handoff
        // is set by analyze_locally tool): passing activeIds=[] would nuke every
        // session in the shared AppState, including other windows' tabs.
        if (!window.__rssh_clone && !window.__rssh_ai_handoff) {
            invoke("reconcile_sessions", { activeIds: [] }).catch(() => {});
        }
        consumeCloneQuery();
        consumeAiHandoff();

        const detachKeydown = attachShortcuts(shortcutsTable());
        const detachKeyup = attachKeyup((e) => {
            if (tabCycling && e.key === "Control") {
                const item = navItems[focusIdx];
                tabCycling = false;
                if (item) activateNavItem(item);
                else closeDrawer();
            }
        });
        return () => { detachKeydown(); detachKeyup(); };
    });

    /* Consume window.__rssh_ai_handoff injected by analyze_locally tool.
       工作流：开本地 shell tab → 等 PTY 就绪 → 启动独立 AI 会话 → 把 task 作为首条消息发过去。
       PTY spawn 在 TerminalPane onMount 里走，前端轮询 sessionIdForTab 等就绪。 */
    async function consumeAiHandoff() {
        const data = window.__rssh_ai_handoff;
        if (!data) return;
        delete window.__rssh_ai_handoff;
        let payload: { local_path: string; task: string };
        try {
            payload = JSON.parse(data);
        } catch (e) {
            console.error("Failed to parse AI handoff:", e);
            return;
        }

        // 1. 开本地 shell tab
        const tabId = `local:${crypto.randomUUID()}`;
        app.addTab({type: "local", id: tabId, label: t("ai.handoff.tab_label"), meta: {}});
        ai.openPanel();

        // 2. Wait for the local PTY to register itself. The store fires
        //    this Promise the moment registerSession runs in TerminalPane —
        //    no polling. 30 s timeout still covers a stuck spawn.
        const sid = await app.waitForSession(tabId, 30000);
        if (!sid) {
            console.error("AI handoff: 本地 PTY 30s 内未就绪，放弃");
            return;
        }

        // 3. 启动独立 AI 会话 + 发首条消息
        try {
            const settings = await ai.loadSettings();
            if (!settings.has_api_key) {
                console.error("AI handoff: 缺 API key，无法自动启动会话");
                return;
            }
            const info = await ai.startSession({
                tabId,
                targetKind: "local",
                targetId: sid,
                skill: "general",
                provider: settings.provider,
                model: settings.model,
            });
            const initialMsg = t("ai.handoff.initial_msg", { path: payload.local_path, task: payload.task });
            await ai.sendMessage(info.tab_id, initialMsg);
        } catch (e) {
            console.error("AI handoff failed:", e);
        }
    }

    /* Consume window.__rssh_clone injected by open_tab_in_new_window */
    function consumeCloneQuery() {
        const data = window.__rssh_clone;
        if (!data) return;
        try {
            const payload = JSON.parse(data) as Tab;
            const newId = `${payload.type}:${crypto.randomUUID()}`;
            app.addTab({...payload, id: newId});
        } catch (e) {
            console.error("Failed to parse clone payload:", e);
        }
        // Clear so a manual reload doesn't re-clone
        delete window.__rssh_clone;
    }

    type SplitDir = "up" | "down" | "left" | "right";

    // split === undefined → plain new window (OS-positioned). A direction tiles
    // the current window into one half and opens the new one in the other.
    function openInNewWindow(tab: Tab, split?: SplitDir) {
        const payload = {type: tab.type, label: tab.label, meta: tab.meta};
        invoke("open_tab_in_new_window", {clone: JSON.stringify(payload), split: split ?? null})
            .catch(e => console.error("open_tab_in_new_window failed:", e));
    }

    $effect(() => {
        if (drawerOpen) {
            app.loadProfiles().then(p => profiles = p);
            app.loadGroups().then(g => groups = g);
        }
    });

    $effect(() => {
        const tab = app.activeTab();
        if (app.settingsActive()) {
            getCurrentWindow().setTitle("Settings");
        } else if (tab) {
            const termTitle = app.terminalTitle(tab.id);
            const title = termTitle ? `${tab.label} — ${termTitle}` : tab.label;
            getCurrentWindow().setTitle(title);
        } else {
            getCurrentWindow().setTitle("RSSH");
        }
        // The Transfers popover does not touch the window title — it is an
        // overlay, not a route.
    });

    let pinnedProfiles = $derived(
        profiles.filter(p => app.pinnedProfileIds().includes(p.id))
    );
    let sbPos = $derived(app.sidebarPosition());
    let isHorizontal = $derived(sbPos === "top" || sbPos === "bottom");

    // AI 面板：仅在终端 tab 已连接时可见；位置走 ai.position()
    let aiTabId = $derived(app.activeTabId());
    let aiActiveTab = $derived(app.activeTab());
    let aiSessionId = $derived(aiActiveTab ? app.sessionIdForTab(aiActiveTab.id) : undefined);
    let aiVisible = $derived(
        ai.isOpen()
        && !!aiActiveTab
        && (aiActiveTab.type === "ssh" || aiActiveTab.type === "local")
        && !!aiSessionId
        && !app.settingsActive()
        // The Transfers popover does not affect AI panel visibility — overlay.
    );
    let xferBadge = $derived.by(() => {
        const n = transfers.activeCount();
        return n > 0 ? String(n) : null;
    });
    let aiPos = $derived(ai.position());

    // SFTP per-tab：tabsWithSftp 是所有"开了 SFTP"的 tab（每个挂一个 SftpBrowser 实例保活）。
    // sftpVisible 只控制 aside 视觉是否展开 + 哪个 pane 显示 —— 切到无 SFTP 的 tab 时实例不 unmount。
    let sftpTabs = $derived(app.tabsWithSftp());
    let sftpVisible = $derived(
        !app.settingsActive() && app.sftpOpen()
        // The Transfers popover does not hide SFTP — overlay.
    );

    /* ── AI 面板宽度：用户拖拽 → localStorage，覆盖响应式默认值。
       未设置时回落到 CSS 中的 380px / 320px / mobile-takeover 媒体查询。 */
    const AI_PANEL_WIDTH_KEY = "ai-panel-width";
    const AI_PANEL_MIN_WIDTH = 280;
    const AI_PANEL_MIN_MAIN = 320; // 终端区至少留这么宽
    let aiPanelWidth = $state<number | null>(null);

    onMount(() => {
        const saved = localStorage.getItem(AI_PANEL_WIDTH_KEY);
        if (saved) {
            const n = parseInt(saved, 10);
            if (Number.isFinite(n) && n >= AI_PANEL_MIN_WIDTH) {
                // 大屏存的值切到小屏会溢出主区。restore 时 clamp 到当前视口可用宽度。
                const maxWidth = Math.max(AI_PANEL_MIN_WIDTH, window.innerWidth - AI_PANEL_MIN_MAIN);
                aiPanelWidth = Math.min(n, maxWidth);
            }
        }
    });

    let aiSideStyle = $derived(
        aiPanelWidth != null
            ? `flex: 0 0 ${aiPanelWidth}px; max-width: ${aiPanelWidth}px;`
            : ""
    );

    /** 另一侧 panel 的当前渲染宽度（aside 元素的 boundingClientRect）；hidden 状态返回 0。
     *  resize 时拿来从可用空间里减掉，避免两个 panel 都拖到极端导致主区被压成 0。 */
    function otherPanelWidth(selector: string): number {
        const el = document.querySelector(selector);
        return el ? (el as HTMLElement).getBoundingClientRect().width : 0;
    }

    function startAiResize(e: MouseEvent) {
        e.preventDefault();
        const startX = e.clientX;
        // 取实际渲染宽度作为起点，避免首次拖拽时的"跳变"
        const sideEl = (e.currentTarget as HTMLElement).parentElement as HTMLElement | null;
        const startWidth = aiPanelWidth ?? (sideEl?.getBoundingClientRect().width ?? 380);
        // AI 在右：handle 在左边缘，光标左移 → AI 变宽（dx 取反）
        // AI 在左：handle 在右边缘，光标右移 → AI 变宽（dx 直接用）
        const sign = aiPos === "left" ? 1 : -1;

        const onMove = (ev: MouseEvent) => {
            const dx = ev.clientX - startX;
            // 减去 SFTP 当前实际占的宽，让两个 panel 互相挤不死主区
            const maxWidth = Math.max(
                AI_PANEL_MIN_WIDTH,
                window.innerWidth - AI_PANEL_MIN_MAIN - otherPanelWidth('.sftp-side'),
            );
            const next = Math.max(AI_PANEL_MIN_WIDTH, Math.min(maxWidth, startWidth + sign * dx));
            aiPanelWidth = next;
        };
        const onUp = () => {
            document.removeEventListener("mousemove", onMove);
            document.removeEventListener("mouseup", onUp);
            if (aiPanelWidth != null) localStorage.setItem(AI_PANEL_WIDTH_KEY, String(aiPanelWidth));
        };
        document.addEventListener("mousemove", onMove);
        document.addEventListener("mouseup", onUp);
    }

    /** 双击 handle：清除手动宽度，回到响应式默认（媒体查询 + 380px）。 */
    function resetAiWidth() {
        aiPanelWidth = null;
        localStorage.removeItem(AI_PANEL_WIDTH_KEY);
    }

    /* ── SFTP 面板宽度：跟 AI 镜像一份，独立 localStorage key。
       SFTP 永远走 AI 的对侧（aiPos=right → SFTP 左；aiPos=left → SFTP 右），
       靠 .content.ai-left 的 row-reverse 自动翻边，不引入新位置 config。 */
    const SFTP_PANEL_WIDTH_KEY = "sftp-panel-width";
    const SFTP_PANEL_MIN_WIDTH = 280;
    const SFTP_PANEL_MIN_MAIN = 320;
    let sftpPanelWidth = $state<number | null>(null);

    onMount(() => {
        const saved = localStorage.getItem(SFTP_PANEL_WIDTH_KEY);
        if (saved) {
            const n = parseInt(saved, 10);
            if (Number.isFinite(n) && n >= SFTP_PANEL_MIN_WIDTH) {
                const maxWidth = Math.max(SFTP_PANEL_MIN_WIDTH, window.innerWidth - SFTP_PANEL_MIN_MAIN);
                sftpPanelWidth = Math.min(n, maxWidth);
            }
        }
    });

    let sftpSideStyle = $derived(
        sftpPanelWidth != null
            ? `flex: 0 0 ${sftpPanelWidth}px; max-width: ${sftpPanelWidth}px;`
            : ""
    );

    function startSftpResize(e: MouseEvent) {
        e.preventDefault();
        const startX = e.clientX;
        const sideEl = (e.currentTarget as HTMLElement).parentElement as HTMLElement | null;
        const startWidth = sftpPanelWidth ?? (sideEl?.getBoundingClientRect().width ?? 380);
        // SFTP 视觉在左（aiPos=right）：handle 在 SFTP 的右边缘，光标右移 → SFTP 变宽（dx 直接用）
        // SFTP 视觉在右（aiPos=left）：handle 在 SFTP 的左边缘，光标左移 → SFTP 变宽（dx 取反）
        const sign = aiPos === "left" ? -1 : 1;

        const onMove = (ev: MouseEvent) => {
            const dx = ev.clientX - startX;
            // 减去 AI 当前实际占的宽，让两个 panel 互相挤不死主区
            const maxWidth = Math.max(
                SFTP_PANEL_MIN_WIDTH,
                window.innerWidth - SFTP_PANEL_MIN_MAIN - otherPanelWidth('.ai-side'),
            );
            const next = Math.max(SFTP_PANEL_MIN_WIDTH, Math.min(maxWidth, startWidth + sign * dx));
            sftpPanelWidth = next;
        };
        const onUp = () => {
            document.removeEventListener("mousemove", onMove);
            document.removeEventListener("mouseup", onUp);
            if (sftpPanelWidth != null) localStorage.setItem(SFTP_PANEL_WIDTH_KEY, String(sftpPanelWidth));
        };
        document.addEventListener("mousemove", onMove);
        document.addEventListener("mouseup", onUp);
    }

    function resetSftpWidth() {
        sftpPanelWidth = null;
        localStorage.removeItem(SFTP_PANEL_WIDTH_KEY);
    }

    /* Menu data — sections describe layout (header / scrollable list / footer),
       flat navItems is what the keyboard shortcut cycles through. */
    let navSections = $derived<{ header: NavItem[]; middle: NavItem[]; footer: NavItem[] }>({
        header: [
            ...app.tabs().filter(t => t.type === "home").map(t => ({kind: "tab" as const, tab: t})),
            ...(app.isMobile ? [] : [{kind: "new-tab" as const}, {kind: "new-edit" as const}]),
            // Horizontal strip would burst sideways with N pinned profiles — collapse
            // them into one ★ button that pops a menu. Vertical sidebar keeps the list.
            ...(isHorizontal
                ? (pinnedProfiles.length > 0 ? [{kind: "pinned-menu" as const}] : [])
                : pinnedProfiles.map(p => ({kind: "pin" as const, profile: p}))),
        ],
        middle: app.tabs().filter(t => t.type !== "home").map(t => ({kind: "tab" as const, tab: t})),
        footer: [
            ...(app.isMobile ? [] : [{kind: "pin-window" as const}, {kind: "downloads" as const}]),
            {kind: "settings" as const},
        ],
    });
    let navItems = $derived<NavItem[]>([...navSections.header, ...navSections.middle, ...navSections.footer]);

    function isFocusedItem(item: NavItem): boolean {
        const f = navItems[focusIdx];
        if (!f || f.kind !== item.kind) return false;
        if (f.kind === "tab" && item.kind === "tab") return f.tab.id === item.tab.id;
        if (f.kind === "pin" && item.kind === "pin") return f.profile.id === item.profile.id;
        return true;
    }

    function isActiveItem(item: NavItem): boolean {
        if (item.kind === "tab") return !app.settingsActive() && item.tab.id === app.activeTabId();
        if (item.kind === "settings") return app.settingsActive();
        // Downloads is a popover, not a route. "active" only tracks real
        // routes (home / settings). The open/closed state surfaces through
        // the badge instead of taking sidebar active highlight.
        return false;
    }

    function activateNavItem(item: NavItem, e?: MouseEvent) {
        if (item.kind === "new-tab") addLocalTab();
        else if (item.kind === "new-edit") addEditTab();
        else if (item.kind === "pin") connectPinned(item.profile);
        else if (item.kind === "pinned-menu") openPinnedMenu(e);
        else if (item.kind === "tab") selectTab(item.tab.id);
        else if (item.kind === "pin-window") { togglePin(); closeDrawer(); }
        else if (item.kind === "downloads") selectDownloads();
        else selectSettings();
    }

    function openPinnedMenu(e?: MouseEvent) {
        const target = e?.currentTarget as HTMLElement | undefined;
        if (target) {
            const r = target.getBoundingClientRect();
            // Anchor to the bottom-left of the button when bar is on top, otherwise above it.
            const aboveBar = sbPos === "bottom";
            pinnedMenu = { x: r.left, y: aboveBar ? r.top : r.bottom + 4 };
        } else {
            // Keyboard cycle path — no anchor element. Drop near top-left of viewport.
            pinnedMenu = { x: 16, y: 60 };
        }
    }

    function closePinnedMenu() { pinnedMenu = null; }

    function buildPinnedMenu(): CtxMenuItem[][] {
        if (pinnedProfiles.length === 0) return [[]];
        return [pinnedProfiles.map(p => ({
            label: p.name,
            onClick: () => connectPinned(p),
        }))];
    }

    function connectPinned(p: Profile) {
        const tabId = `ssh:${crypto.randomUUID()}`;
        app.addTab({
            id: tabId, type: "ssh", label: p.name,
            meta: {profileId: p.id, host: p.host, port: String(p.port)},
        });
        closeDrawer();
    }

    let touchStartX = 0;
    let touchStartY = 0;

    function openDrawer() {
        drawerOpen = true;
    }

    function closeDrawer() {
        drawerOpen = false;
        focusIdx = -1;
        tabCycling = false;
    }

    function enterSidebar(e: MouseEvent) {
        if (e.buttons) return;
        clearTimeout(sidebarTimer);
        if (!drawerOpen) openDrawer();
    }

    function leaveSidebar() {
        sidebarTimer = window.setTimeout(closeDrawer, 200);
    }

    function selectTab(id: string) {
        app.setActiveTab(id);
        closeDrawer();
    }

    function selectSettings() {
        app.openSettings();
        closeDrawer();
    }

    function selectDownloads() {
        // Popover: every click on the sidebar entry toggles open/closed.
        app.toggleDownloads();
        closeDrawer();
    }

    function addLocalTab() {
        const id = `local:${crypto.randomUUID()}`;
        app.addTab({id, type: "local", label: "Local"});
        closeDrawer();
    }

    function addEditTab() {
        const id = `edit:${crypto.randomUUID()}`;
        app.addTab({ id, type: "edit", label: "Edit" });
        closeDrawer();
    }

    /* ── Tab context menu ── */
    function openCtxMenu(e: MouseEvent, tab: Tab) {
        e.preventDefault();
        menuCtx = {x: e.clientX, y: e.clientY, tab};
    }

    /** Detect 10-digit Unix seconds or 13-digit Unix ms timestamp. */
    function tryParseTimestamp(s: string): Date | null {
        const t = s.trim();
        if (/^\d{10}$/.test(t)) return new Date(parseInt(t, 10) * 1000);
        if (/^\d{13}$/.test(t)) return new Date(parseInt(t, 10));
        return null;
    }

    function formatUtc(d: Date): string {
        return d.toISOString().replace("T", " ").slice(0, 19) + "Z";
    }

    function closeCtxMenu() {
        menuCtx = null;
    }

    function cloneTab(tab: Tab) {
        const newId = `${tab.type}:${crypto.randomUUID()}`;
        app.addTab({
            id: newId,
            type: tab.type,
            label: tab.label,
            meta: tab.meta ? {...tab.meta} : undefined,
        });
    }

    function buildMenu(tab: Tab): CtxMenuItem[][] {
        const isTerminal = tab.type === "ssh" || tab.type === "local";
        const isSsh = tab.type === "ssh";
        const sections: CtxMenuItem[][] = [];

        // Copy / Paste (+ UTC if selection is a timestamp) / Add-to-Snippets.
        if (isTerminal) {
            const selection = app.terminalGetSelection(tab.id);
            const trimmed = selection?.trim() ?? "";
            // Parse the trimmed selection so timestamps with leading/trailing
            // whitespace still surface the UTC copy action.
            const ts = trimmed ? tryParseTimestamp(trimmed) : null;
            const copyPaste: CtxMenuItem[] = [
                {
                    label: t("tab.context.copy"),
                    disabled: !selection,
                    onClick: () => { if (selection) app.writeClipboard(selection); },
                },
                {
                    label: t("tab.context.paste"),
                    // Activate the target tab, then hand focus back to its
                    // terminal: the menu closing drops focus to <body>, and the
                    // activate-focus $effect in TerminalPane is a no-op when the
                    // tab was already active — so paste-into-current-tab would
                    // otherwise leave the user unable to type. terminalFocus runs
                    // after the async read, so it wins the focus back from <body>.
                    onClick: () => {
                        app.setActiveTab(tab.id);
                        app.readClipboard().then(text => {
                            if (text) app.terminalPaste(tab.id, text);
                            app.terminalFocus(tab.id);
                        });
                    },
                },
            ];
            if (ts) {
                const utc = formatUtc(ts);
                copyPaste.push({
                    label: `${t("tab.context.copy_utc")}: ${utc}`,
                    onClick: () => { app.writeClipboard(utc); },
                });
            }
            // Save the selected text as a command snippet: name = first 10
            // chars, command = the full selection. All-whitespace selections
            // are disabled — a 10-space-named snippet has no value.
            copyPaste.push({
                label: t("tab.context.add_to_snippets"),
                disabled: !trimmed,
                onClick: async () => {
                    if (!trimmed) return;
                    const name = trimmed.slice(0, 10);
                    try {
                        const all = await app.loadSnippets();
                        all.push({ name, command: trimmed });
                        await invoke("save_snippets", { snippets: all });
                        toast.success(`${t("tab.context.add_to_snippets")}: ${name}`);
                    } catch (e) {
                        toast.error(`${t("toast.error.save")}: ${errMsg(e)}`);
                    }
                },
            });
            sections.push(copyPaste);
        }

        if (isTerminal) {
            const items: CtxMenuItem[] = [
                {
                    label: t("tab.context.search"),
                    shortcut: keymap.format("term.search"),
                    onClick: () => { app.setActiveTab(tab.id); app.requestSearch(tab.id); },
                },
                {
                    label: t("tab.context.snippets"),
                    shortcut: keymap.format("term.snippet"),
                    onClick: () => { app.setActiveTab(tab.id); app.openSnippetPicker(); },
                },
            ];
            // SFTP requires native file dialogs — desktop only.
            if (!app.isMobile) {
                items.push({
                    label: t("tab.context.sftp"),
                    shortcut: keymap.format("term.sftp"),
                    disabled: !isSsh,
                    onClick: () => { app.setActiveTab(tab.id); app.openSftp(); },
                });
            }
            sections.push(items);
        }

        sections.push([
            {
                label: t("tab.context.clone"),
                shortcut: tab.type === "home" ? undefined : keymap.format("tab.clone"),
                disabled: tab.type === "home",
                onClick: () => cloneTab(tab),
            },
            {label: t("tab.context.close"), shortcut: keymap.format("tab.close"), onClick: () => app.closeTab(tab.id)},
        ]);

        // AI 排障入口（ssh/local tab 才有，且需要已经连上 = 有 sessionId）
        if (isTerminal) {
            const sid = app.sessionIdForTab(tab.id);
            sections.push([
                {
                    label: t("tab.context.ai"),
                    shortcut: keymap.format("ai.toggle"),
                    disabled: !sid,
                    onClick: () => { app.setActiveTab(tab.id); ai.openPanel(); },
                },
            ]);
        }

        // Multi-window requires Tauri WebviewWindowBuilder — desktop only.
        if (isTerminal && !app.isMobile) {
            sections.push([
                {
                    label: t("tab.context.open_new_window"),
                    shortcut: keymap.format("tab.openNewWindow"),
                    onClick: () => openInNewWindow(tab),
                    submenu: [
                        {label: t("tab.context.open_new_window.up"), onClick: () => openInNewWindow(tab, "up")},
                        {label: t("tab.context.open_new_window.down"), onClick: () => openInNewWindow(tab, "down")},
                        {label: t("tab.context.open_new_window.left"), onClick: () => openInNewWindow(tab, "left")},
                        {label: t("tab.context.open_new_window.right"), onClick: () => openInNewWindow(tab, "right")},
                    ],
                },
            ]);
        }

        return sections;
    }

    function tabGroupColor(tab: Tab): string | null {
        if (tab.type !== "ssh") return null;
        const profileId = tab.meta?.profileId;
        if (!profileId) return null;
        const profile = profiles.find(p => p.id === profileId);
        if (!profile?.group_id) return null;
        const group = groups.find(g => g.id === profile.group_id);
        return group?.color ?? null;
    }

    /* ── Tab drag-and-drop reorder ── */
    function handleDragStart(e: DragEvent, tabId: string) {
        dragTabId = tabId;
        if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
    }

    function handleDragOver(e: DragEvent, tabId: string) {
        e.preventDefault();
        if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
        dropTabId = tabId;
    }

    function handleDrop(e: DragEvent, tabId: string) {
        e.preventDefault();
        if (dragTabId && dragTabId !== tabId) {
            const allTabs = app.tabs();
            const fromIdx = allTabs.findIndex(t => t.id === dragTabId);
            const toIdx = allTabs.findIndex(t => t.id === tabId);
            if (fromIdx >= 0 && toIdx >= 0) app.moveTab(fromIdx, toIdx);
        }
        dragTabId = null;
        dropTabId = null;
    }

    function handleDragEnd(e: DragEvent) {
        // dropEffect === "none" means the drag was cancelled (Esc or invalid drop)
        const cancelled = e.dataTransfer?.dropEffect === "none";
        if (!cancelled && dragTabId && dropTabId && dragTabId !== dropTabId) {
            const allTabs = app.tabs();
            const fromIdx = allTabs.findIndex(t => t.id === dragTabId);
            const toIdx = allTabs.findIndex(t => t.id === dropTabId);
            if (fromIdx >= 0 && toIdx >= 0) app.moveTab(fromIdx, toIdx);
        }
        dragTabId = null;
        dropTabId = null;
    }

    function handleTouchStart(e: TouchEvent) {
        touchStartX = e.touches[0].clientX;
        touchStartY = e.touches[0].clientY;
    }

    function handleTouchEnd(e: TouchEvent) {
        const dx = e.changedTouches[0].clientX - touchStartX;
        const dy = Math.abs(e.changedTouches[0].clientY - touchStartY);
        const pos = app.sidebarPosition();
        if (pos !== "left" && pos !== "right") return;
        // Mirror edge-swipe direction based on which side the sidebar lives on.
        const sign = pos === "left" ? 1 : -1;
        const nearEdge = pos === "left" ? touchStartX < 50 : touchStartX > window.innerWidth - 50;
        if (!drawerOpen && nearEdge && sign * dx > 60 && dy < Math.abs(dx)) openDrawer();
        if (drawerOpen && sign * dx < -60 && dy < Math.abs(dx)) closeDrawer();
    }

    function handleKeydown(e: KeyboardEvent) {
        if (e.key === "Escape") {
            // The Transfers popover has its own Esc handler. When it's open,
            // a single Esc should only close the topmost overlay (the popover),
            // not also collapse SFTP/drawer underneath it.
            if (app.downloadsActive()) return;
            if (app.sftpOpen()) { app.closeSftp(); e.preventDefault(); }
            else if (drawerOpen) { closeDrawer(); e.preventDefault(); }
        }
    }
</script>

<svelte:window onkeydown={handleKeydown}/>

{#if app.snippetPickerOpen()}
    <SnippetPicker />
{/if}

{#if menuCtx}
    <TabContextMenu
        x={menuCtx.x}
        y={menuCtx.y}
        sections={buildMenu(menuCtx.tab)}
        onClose={closeCtxMenu}
    />
{/if}

{#if pinnedMenu}
    <TabContextMenu
        x={pinnedMenu.x}
        y={pinnedMenu.y}
        sections={buildPinnedMenu()}
        onClose={closePinnedMenu}
    />
{/if}

<div
    class="shell"
    class:sb-left={sbPos === "left"}
    class:sb-right={sbPos === "right"}
    class:sb-top={sbPos === "top"}
    class:sb-bottom={sbPos === "bottom"}
    ontouchstart={handleTouchStart}
    ontouchend={handleTouchEnd}
    role="presentation"
>

    {#if drawerOpen}
        <div class="backdrop" onclick={closeDrawer} role="presentation"></div>
    {/if}

    <!-- Sidebar: 40px collapsed ↔ 260px expanded. Position = left | right. -->
    {#if sbPos === "left" || sbPos === "right"}
    <div class="sidebar-rail">
    <nav
        class="sidebar" class:open={drawerOpen} class:right={sbPos === "right"}
        onmouseenter={enterSidebar} onmouseleave={leaveSidebar}
    >
        <div class="sidebar-inner">
            {#each navSections.header as item (navItemKey(item))}
                <MenuButton
                    {item}
                    active={isActiveItem(item)}
                    focused={isFocusedItem(item)}
                    pinnedState={pinned}
                    onActivate={(e) => activateNavItem(item, e)}
                />
            {/each}

            <div class="sidebar-list">
                {#each navSections.middle as item (navItemKey(item))}
                    {@const tab = item.kind === "tab" ? item.tab : null}
                    <MenuButton
                        {item}
                        active={isActiveItem(item)}
                        focused={isFocusedItem(item)}
                        dragOver={tab !== null && dropTabId === tab.id && dragTabId !== tab.id}
                        groupColor={tab ? tabGroupColor(tab) : null}
                        showClose={tab !== null}
                        onActivate={(e) => activateNavItem(item, e)}
                        onClose={tab ? () => app.closeTab(tab.id) : undefined}
                        onDragStart={tab ? (e) => handleDragStart(e, tab.id) : undefined}
                        onDragOver={tab ? (e) => handleDragOver(e, tab.id) : undefined}
                        onDrop={tab ? (e) => handleDrop(e, tab.id) : undefined}
                        onDragEnd={tab ? handleDragEnd : undefined}
                    />
                {/each}
            </div>

            <div class="sidebar-footer">
                {#each navSections.footer as item (navItemKey(item))}
                    <MenuButton
                        {item}
                        active={isActiveItem(item)}
                        focused={isFocusedItem(item)}
                        pinnedState={pinned}
                        badge={item.kind === "downloads" ? xferBadge : null}
                        redDot={item.kind === "settings" && updates.hasUpdate()}
                        onActivate={(e) => activateNavItem(item, e)}
                    />
                {/each}
            </div>
        </div>
    </nav>
    </div>
    {:else}
        <StripBar
            sections={[navSections.header, navSections.middle, navSections.footer]}
            position={sbPos}
            pinned={pinned}
            dragTabId={dragTabId}
            dropTabId={dropTabId}
            xferBadge={xferBadge}
            settingsRedDot={updates.hasUpdate()}
            isActiveItem={isActiveItem}
            isFocusedItem={isFocusedItem}
            groupColorOf={tabGroupColor}
            onActivate={activateNavItem}
            onClose={(id) => app.closeTab(id)}
            onDragStart={handleDragStart}
            onDragOver={handleDragOver}
            onDrop={handleDrop}
            onDragEnd={handleDragEnd}
        />
    {/if}

    <div
        class="content"
        class:ai-on={aiVisible}
        class:ai-left={aiVisible && aiPos === "left"}
        class:sftp-on={sftpVisible}
    >
        <!-- 任何 tab 开了 SFTP 就把 aside 挂上（保留所有 tab 的 SftpBrowser 实例 → 切回时 cwd 不丢）。
             active tab 没开 / 进入 settings / downloads 时整块 aside 走 .hidden 收掉视觉宽度，但 DOM 留着。
             SFTP 走 AI 对侧：aiPos=right(default) → SFTP 视觉左、handle 右边缘；
             aiPos=left 下 .content.ai-left 翻 row → SFTP 视觉右、handle 左边缘。 -->
        {#if sftpTabs.length > 0}
            <aside class="sftp-side" class:hidden={!sftpVisible} style={sftpSideStyle}>
                <div class="sftp-resize-handle"
                     class:on-left={aiPos === "left"}
                     onmousedown={startSftpResize}
                     ondblclick={resetSftpWidth}
                     role="separator"
                     aria-orientation="vertical"
                     title={t("common.resize_hint")}></div>
                {#each sftpTabs as tab (tab.id)}
                    <div class="sftp-pane" class:visible={tab.id === app.activeTabId() && sftpVisible}>
                        <SftpBrowser meta={{...tab.meta ?? {}, sessionId: app.sessionIdForTab(tab.id) ?? ''}}/>
                    </div>
                {/each}
            </aside>
        {/if}
        <div class="main-area">
            {#if app.settingsActive()}
                <div class="pane visible">
                    <SettingsLayout/>
                </div>
            {/if}

            {#each app.tabs() as tab (tab.id)}
                <div class="pane"
                     class:visible={!app.settingsActive() && tab.id === app.activeTabId()}
                     oncontextmenu={app.isMobile ? undefined : (e) => openCtxMenu(e, tab)}>
                    {#if tab.type === "home"}
                        <HomeScreen/>
                    {:else if tab.type === "ssh" || tab.type === "local"}
                        <TerminalPane tabId={tab.id} tabType={tab.type} meta={tab.meta ?? {}}/>
                    {:else if tab.type === "forward"}
                        <ForwardPane tabId={tab.id} meta={tab.meta ?? {}}/>
                    {:else if tab.type === "edit"}
                        <EditPane tabId={tab.id} />
                    {/if}
                </div>
            {/each}
        </div>

        {#if aiVisible && aiActiveTab && aiSessionId}
            <aside class="ai-side" style={aiSideStyle}>
                <div class="ai-resize-handle"
                     class:on-right={aiPos === "left"}
                     onmousedown={startAiResize}
                     ondblclick={resetAiWidth}
                     role="separator"
                     aria-orientation="vertical"
                     title={t("common.resize_hint")}></div>
                <ChatPanel
                    tabId={aiActiveTab.id}
                    targetKind={aiActiveTab.type as "ssh" | "local"}
                    targetId={aiSessionId}
                />
            </aside>
        {/if}
    </div>

    <!-- Popover lives inside .shell so it inherits the --sb-* layout vars that
         drive its edge offsets. position:fixed still anchors to the viewport
         because .shell does not create a fixed-positioning containing block. -->
    {#if app.downloadsActive()}
        <DownloadsScreen/>
    {/if}
</div>

<style>
    /* Flow layout: the bar (sidebar rail / stripbar) and .content are flex
       items, so the bar occupies real space and content takes the rest. No
       fixed positioning, no margin reservation — content can never slide
       under the bar (the old position:fixed + margin-top hack let a stray
       document scroll tuck the terminal top behind a viewport-pinned bar). */
    .shell {
        height: 100%;
        position: relative;
        display: flex;
        /* Bar thickness per edge — only the DownloadsScreen popover reads
           these now, to offset itself off the bar. Layout uses flow. */
        --sb-left: 0px;
        --sb-right: 0px;
        --sb-top: 0px;
        --sb-bottom: 0px;
    }
    .shell.sb-left   { flex-direction: row;            --sb-left:   40px; }
    .shell.sb-right  { flex-direction: row-reverse;    --sb-right:  40px; }
    .shell.sb-top    { flex-direction: column;         --sb-top:    44px; }
    .shell.sb-bottom { flex-direction: column-reverse; --sb-bottom: 44px; }

    /* ── Sidebar: rail (flow footprint) + overlay (hover-expanding panel) ── */
    /* The rail is a 40px flex item that reserves the collapsed sidebar's space
       in normal flow, so content sits beside it and can never overlap it. */
    .sidebar-rail {
        flex: 0 0 40px;
        position: relative;
        z-index: 200;
    }

    /* The panel itself is absolute within the rail: 40px collapsed, 260px on
       hover. Absolute so the expansion floats over content instead of pushing
       it — preserving the original drawer feel without the viewport-fixed hack. */
    .sidebar {
        position: absolute;
        left: 0;
        top: 0;
        width: 40px;
        height: 100%;
        background: var(--bg);
        border-right: 1px solid var(--divider);
        overflow: hidden;
        transition: width 0.15s ease;
    }

    .sidebar.right {
        left: auto;
        right: 0;
        border-right: none;
        border-left: 1px solid var(--divider);
    }

    .sidebar.open {
        width: 260px;
        box-shadow: var(--raised);
    }


    /* Inner container always 260px — sidebar clips it */
    .sidebar-inner {
        width: 260px;
        min-width: 260px;
        height: 100%;
        display: flex;
        flex-direction: column;
        padding: 6px;
        gap: 2px;
    }

    .sidebar-list {
        padding-top: 2px;
        border-top: 1px solid var(--divider);
        flex: 1;
        overflow-y: auto;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .sidebar-footer {
        border-top: 1px solid var(--divider);
        padding-top: 6px;
        margin-top: 2px;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    /* ── Backdrop ── */
    .backdrop {
        position: fixed;
        inset: 0;
        background: var(--overlay-soft);
        z-index: 100;
    }

    /* ── Content = 剩余空间（让位 sidebar 后），内部分成 main-area + ai-side flex 横排 ── */
    .content {
        position: relative;
        display: flex;
        flex-direction: row;
        flex: 1;
        min-width: 0;
        min-height: 0;
    }
    /* AI 在左：flex row 翻转，模板顺序不变，无须状态机 */
    .content.ai-left { flex-direction: row-reverse; }

    /* 终端区——所有 .pane 挂在这里，绝对定位由父级 main-area 提供 position: relative。
       min-width: 0 让 flex 能把它压到 0（窄屏 AI 接管时） */
    .main-area {
        flex: 1;
        position: relative;
        min-width: 0;
    }

    /* 边框在 ChatPanel 自身 CSS 里（左右都有），aside 不重复加 */
    .ai-side {
        flex: 0 0 380px;
        background: var(--bg);
        position: relative;
    }

    @media (max-width: 800px) { .ai-side { flex-basis: 320px; } }

    /* 拖拽宽度的把手：贴在 ai-side 的内边缘（默认右布局 → 左边；左布局 → 右边）。
       6px 命中区域，悬停/拖拽时露一根细线。 */
    .ai-resize-handle {
        position: absolute;
        top: 0;
        bottom: 0;
        left: -3px;
        width: 6px;
        cursor: col-resize;
        z-index: 10;
        background: transparent;
        transition: background 0.12s ease;
    }
    .ai-resize-handle.on-right {
        left: auto;
        right: -3px;
    }
    .ai-resize-handle:hover,
    .ai-resize-handle:active {
        background: var(--accent);
        opacity: 0.45;
    }

    /* 竖屏手机：AI 接管整块内容区，main-area 挤到 0（终端实例保留，关 AI 后恢复） */
    @media (max-width: 480px) {
        .ai-side { flex: 1; }
        .content.ai-on .main-area { flex: 0; }
    }

    .pane {
        position: absolute;
        inset: 0;
        display: none;
    }

    .pane.visible {
        display: flex;
        flex-direction: column;
    }

    /* ── SFTP side panel —— 跟 .ai-side 镜像；位置由 .content 的 row / row-reverse 决定 ── */
    .sftp-side {
        flex: 0 0 380px;
        background: var(--bg);
        position: relative;
        border-right: 1px solid var(--divider);
    }
    /* aside 里挂多个 SftpBrowser 实例（每 tab 一个），靠 .visible 决定显示哪个。
       绝对定位让所有非活跃实例不占布局空间，但 DOM 留着 → cwd / 网络连接保活。 */
    .sftp-pane {
        position: absolute;
        inset: 0;
        display: none;
    }
    .sftp-pane.visible {
        display: flex;
        flex-direction: column;
    }
    /* active tab 没开 SFTP / settings / downloads 状态下整块 aside 折叠为 0，
       内部 SftpBrowser 实例保持 mount —— 切回有 SFTP 的 tab 时立刻恢复。 */
    .sftp-side.hidden {
        flex-basis: 0 !important;
        max-width: 0 !important;
        overflow: hidden;
        border: none;
    }
    /* aiPos=left 时 .content 翻 row-reverse → SFTP 视觉在右，分隔线得贴左边缘 */
    .content.ai-left .sftp-side {
        border-right: none;
        border-left: 1px solid var(--divider);
    }
    @media (max-width: 800px) { .sftp-side { flex-basis: 320px; } }

    .sftp-resize-handle {
        position: absolute;
        top: 0;
        bottom: 0;
        right: -3px;        /* SFTP 视觉在左 → handle 在右边缘 */
        width: 6px;
        cursor: col-resize;
        z-index: 10;
        background: transparent;
        transition: background 0.12s ease;
    }
    .sftp-resize-handle.on-left {  /* SFTP 视觉在右（aiPos=left）→ handle 翻到左边缘 */
        right: auto;
        left: -3px;
    }
    .sftp-resize-handle:hover,
    .sftp-resize-handle:active {
        background: var(--accent);
        opacity: 0.45;
    }

</style>
