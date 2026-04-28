<script lang="ts">
    import {onMount} from "svelte";
    import {invoke} from "@tauri-apps/api/core";
    import {getCurrentWindow} from "@tauri-apps/api/window";
    import type {Profile, Tab, Group} from "../stores/app.svelte.ts";
    import * as app from "../stores/app.svelte.ts";
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
    import {t} from "../i18n/index.svelte.ts";

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
                display: "⌘W / Ctrl+W",
                description: t("shortcut.tab.close"),
                skipInSettings: true,
                match: e => (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key === "w",
                handler: () => {
                    const id = app.activeTabId();
                    if (id === "home") return false;
                    app.closeTab(id);
                },
            },
            {
                display: "⌘⇧D / Ctrl+Shift+D",
                description: t("shortcut.tab.clone"),
                skipInSettings: true,
                match: e => (e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && e.key.toLowerCase() === "d",
                handler: () => {
                    const tab = app.activeTab();
                    if (!tab || tab.type === "home") return false;
                    cloneTab(tab);
                },
            },
            {
                display: "⌘⇧N / Ctrl+Shift+N",
                description: t("shortcut.tab.open_new_window"),
                skipInSettings: true,
                match: e => (e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && e.key.toLowerCase() === "n",
                handler: () => {
                    const tab = app.activeTab();
                    if (!tab || (tab.type !== "ssh" && tab.type !== "local") || app.isMobile) return false;
                    openInNewWindow(tab);
                },
            },
            {
                display: "Ctrl+Tab / Ctrl+Shift+Tab",
                description: t("shortcut.tab.cycle"),
                match: e => e.ctrlKey && e.key === "Tab",
                handler: e => {
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

        // 2. 等本地 PTY 就绪（TerminalPane 异步 spawn + setSession）。300ms × 100 = 30s 上限
        const sid = await waitFor(() => app.sessionIdForTab(tabId), 300, 100);
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
                targetKind: "local",
                targetId: sid,
                skill: "general",
                provider: settings.provider,
                model: settings.model,
            });
            const initialMsg = t("ai.handoff.initial_msg", { path: payload.local_path, task: payload.task });
            await ai.sendMessage(info.session_id, initialMsg);
        } catch (e) {
            console.error("AI handoff failed:", e);
        }
    }

    async function waitFor<T>(probe: () => T | undefined, intervalMs: number, maxTries: number): Promise<T | undefined> {
        for (let i = 0; i < maxTries; i++) {
            const v = probe();
            if (v !== undefined) return v;
            await new Promise(r => setTimeout(r, intervalMs));
        }
        return undefined;
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

    function openInNewWindow(tab: Tab) {
        const payload = {type: tab.type, label: tab.label, meta: tab.meta};
        invoke("open_tab_in_new_window", {clone: JSON.stringify(payload)})
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
        } else if (app.downloadsActive()) {
            getCurrentWindow().setTitle(t("downloads.title"));
        } else if (tab) {
            const termTitle = app.terminalTitle(tab.id);
            const title = termTitle ? `${tab.label} — ${termTitle}` : tab.label;
            getCurrentWindow().setTitle(title);
        } else {
            getCurrentWindow().setTitle("RSSH");
        }
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
        && !app.downloadsActive()
    );
    let xferBadge = $derived.by(() => {
        const n = transfers.activeCount();
        return n > 0 ? String(n) : null;
    });
    let aiPos = $derived(ai.position());

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
        if (item.kind === "tab") return !app.settingsActive() && !app.downloadsActive() && item.tab.id === app.activeTabId();
        if (item.kind === "settings") return app.settingsActive();
        if (item.kind === "downloads") return app.downloadsActive();
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
        app.openDownloads();
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

        // Copy / Paste (+ UTC if selection is a timestamp).
        if (isTerminal) {
            const selection = app.terminalGetSelection(tab.id);
            const ts = selection ? tryParseTimestamp(selection) : null;
            const copyPaste: CtxMenuItem[] = [
                {
                    label: t("tab.context.copy"),
                    disabled: !selection,
                    onClick: () => { if (selection) navigator.clipboard.writeText(selection).catch(() => {}); },
                },
                {
                    label: t("tab.context.paste"),
                    onClick: () => { app.readClipboard().then(text => { if (text) app.terminalPaste(tab.id, text); }); },
                },
            ];
            if (ts) {
                const utc = formatUtc(ts);
                copyPaste.push({
                    label: `${t("tab.context.copy_utc")}: ${utc}`,
                    onClick: () => { navigator.clipboard.writeText(utc).catch(() => {}); },
                });
            }
            sections.push(copyPaste);
        }

        if (isTerminal) {
            const items: CtxMenuItem[] = [
                {
                    label: t("tab.context.search"),
                    shortcut: "⌘F",
                    onClick: () => { app.setActiveTab(tab.id); app.requestSearch(tab.id); },
                },
                {
                    label: t("tab.context.snippets"),
                    shortcut: "⌘S",
                    onClick: () => { app.setActiveTab(tab.id); app.openSnippetPicker(); },
                },
            ];
            // SFTP requires native file dialogs — desktop only.
            if (!app.isMobile) {
                items.push({
                    label: t("tab.context.sftp"),
                    shortcut: "⌘O",
                    disabled: !isSsh,
                    onClick: () => { app.setActiveTab(tab.id); app.openSftp(); },
                });
            }
            sections.push(items);
        }

        sections.push([
            {
                label: t("tab.context.clone"),
                shortcut: tab.type === "home" ? undefined : "⌘⇧D",
                disabled: tab.type === "home",
                onClick: () => cloneTab(tab),
            },
            {label: t("tab.context.close"), shortcut: "⌘W", onClick: () => app.closeTab(tab.id)},
        ]);

        // AI 排障入口（ssh/local tab 才有，且需要已经连上 = 有 sessionId）
        if (isTerminal) {
            const sid = app.sessionIdForTab(tab.id);
            sections.push([
                {
                    label: t("tab.context.ai"),
                    disabled: !sid,
                    onClick: () => { app.setActiveTab(tab.id); ai.openPanel(); },
                },
            ]);
        }

        // Multi-window requires Tauri WebviewWindowBuilder — desktop only.
        if (isTerminal && !app.isMobile) {
            sections.push([
                {label: t("tab.context.open_new_window"), shortcut: "⌘⇧N", onClick: () => openInNewWindow(tab)},
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

{#if app.sftpOpen()}
    <div class="sftp-overlay">
        <div class="sftp-bar">
            <button class="btn btn-sm" onclick={() => app.closeSftp()}>← Back</button>
            <span class="sftp-title">SFTP</span>
        </div>
        <div class="sftp-body">
            <SftpBrowser meta={{...app.activeTab()?.meta ?? {}, sessionId: app.sessionIdForTab(app.activeTabId()) ?? ''}}/>
        </div>
    </div>
{/if}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
    class="shell"
    class:sb-left={sbPos === "left"}
    class:sb-right={sbPos === "right"}
    class:sb-top={sbPos === "top"}
    class:sb-bottom={sbPos === "bottom"}
    ontouchstart={handleTouchStart}
    ontouchend={handleTouchEnd}
>

    {#if drawerOpen}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="backdrop" onclick={closeDrawer}></div>
    {/if}

    <!-- Sidebar: 40px collapsed ↔ 260px expanded. Position = left | right. -->
    {#if sbPos === "left" || sbPos === "right"}
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
                        onActivate={(e) => activateNavItem(item, e)}
                    />
                {/each}
            </div>
        </div>
    </nav>
    {:else}
        <StripBar
            sections={[navSections.header, navSections.middle, navSections.footer]}
            position={sbPos}
            pinned={pinned}
            dragTabId={dragTabId}
            dropTabId={dropTabId}
            xferBadge={xferBadge}
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
    >
        <div class="main-area">
            {#if app.settingsActive()}
                <div class="pane visible">
                    <SettingsLayout/>
                </div>
            {:else if app.downloadsActive()}
                <div class="pane visible">
                    <DownloadsScreen/>
                </div>
            {/if}

            {#each app.tabs() as tab (tab.id)}
                <div class="pane"
                     class:visible={!app.settingsActive() && !app.downloadsActive() && tab.id === app.activeTabId()}
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
            <aside class="ai-side">
                <ChatPanel
                    tabId={aiActiveTab.id}
                    targetKind={aiActiveTab.type as "ssh" | "local"}
                    targetId={aiSessionId}
                />
            </aside>
        {/if}
    </div>
</div>

<style>
    .shell {
        height: 100%;
        position: relative;
        /* Sidebar 在四个方向上的占用厚度——AI 面板与 .content 都从这里读，
           不再写 magic number 也不再为"sb 在右 + ai 在左"这种组合开特例。 */
        --sb-left: 0px;
        --sb-right: 0px;
        --sb-top: 0px;
        --sb-bottom: 0px;
    }
    .shell.sb-left   { --sb-left:   40px; }
    .shell.sb-right  { --sb-right:  40px; }
    .shell.sb-top    { --sb-top:    44px; }
    .shell.sb-bottom { --sb-bottom: 44px; }

    /* ── Sidebar: one component, two states ── */
    .sidebar {
        position: fixed;
        left: 0;
        top: env(safe-area-inset-top, 0px);
        width: 40px;
        height: calc(100% - env(safe-area-inset-top, 0px));
        background: var(--bg);
        border-right: 1px solid var(--divider);
        z-index: 200;
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
        background: rgba(0, 0, 0, 0.4);
        z-index: 100;
    }

    /* ── Content = 剩余空间（让位 sidebar 后），内部分成 main-area + ai-side flex 横排 ── */
    .content {
        position: relative;
        display: flex;
        flex-direction: row;
        margin-left: var(--sb-left);
        margin-right: var(--sb-right);
        margin-top: var(--sb-top);
        height: calc(100% - var(--sb-top) - var(--sb-bottom));
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
    }

    @media (max-width: 800px) { .ai-side { flex-basis: 320px; } }

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

    /* ── SFTP overlay ── */
    .sftp-overlay {
        position: fixed;
        inset: 0;
        z-index: 300;
        display: flex;
        flex-direction: column;
        background: var(--bg);
        padding-top: env(safe-area-inset-top, 0px);
    }

    .sftp-bar {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 8px 16px;
        border-bottom: 1px solid var(--divider);
        flex-shrink: 0;
    }

    .sftp-title {
        font-size: 14px;
        font-weight: 600;
        color: var(--text);
    }

    .sftp-body {
        flex: 1;
        overflow-y: auto;
    }
</style>
