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
    import SnippetPicker from "./SnippetPicker.svelte";
    import TabContextMenu, {type CtxMenuItem} from "./TabContextMenu.svelte";
    import {attachShortcuts, attachKeyup, type Shortcut} from "../keyboard/registry.ts";
    import {t} from "../i18n/index.svelte.ts";

    let drawerOpen = $state(false);
    let focusIdx = $state(-1);
    let tabCycling = $state(false);
    let profiles = $state<Profile[]>([]);
    let groups = $state<Group[]>([]);
    let sidebarTimer = 0;
    let menuCtx = $state<{ x: number; y: number; tab: Tab } | null>(null);
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
                description: "关闭当前 Tab",
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
                description: "克隆当前 Tab",
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
                description: "在新窗口打开当前 Tab",
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
                description: "在 Tab 之间循环切换",
                match: e => e.ctrlKey && e.key === "Tab",
                handler: e => {
                    const dir = e.shiftKey ? -1 : 1;
                    if (!tabCycling) {
                        tabCycling = true;
                        drawerOpen = true;
                        const idx = navItems.findIndex(item =>
                            item.kind === "tab" ? item.id === app.activeTabId() && !app.settingsActive()
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
                description: "退出 Tab 切换模式",
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
        // open_tab_in_new_window): passing activeIds=[] would nuke every
        // session in the shared AppState, including other windows' tabs.
        if (!window.__rssh_clone) {
            invoke("reconcile_sessions", { activeIds: [] }).catch(() => {});
        }
        consumeCloneQuery();

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

    type NavItem = { kind: "pin"; profile: Profile } | { kind: "tab"; id: string } | { kind: "new-tab" } | { kind: "new-edit" } | { kind: "pin-window" } | { kind: "settings" };
    let navItems = $derived<NavItem[]>([
        ...app.tabs().filter(t => t.type === "home").map(t => ({kind: "tab" as const, id: t.id})),
        ...(app.isMobile ? [] : [{kind: "new-tab" as const}, {kind: "new-edit" as const}]),
        ...pinnedProfiles.map(p => ({kind: "pin" as const, profile: p})),
        ...app.tabs().filter(t => t.type !== "home").map(t => ({kind: "tab" as const, id: t.id})),
        ...(app.isMobile ? [] : [{kind: "pin-window" as const}]),
        {kind: "settings" as const},
    ]);

    function isFocused(kind: NavItem["kind"], id?: string): boolean {
        const f = navItems[focusIdx];
        if (!f || f.kind !== kind) return false;
        if (kind === "tab" && "id" in f) return f.id === id;
        if (kind === "pin" && "profile" in f) return f.profile.id === id;
        return true;
    }

    function activateNavItem(item: NavItem) {
        if (item.kind === "new-tab") addLocalTab();
        else if (item.kind === "new-edit") addEditTab();
        else if (item.kind === "pin") connectPinned(item.profile);
        else if (item.kind === "tab") selectTab(item.id);
        else if (item.kind === "pin-window") { togglePin(); closeDrawer(); }
        else selectSettings();
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

        // Multi-window requires Tauri WebviewWindowBuilder — desktop only.
        if (isTerminal && !app.isMobile) {
            sections.push([
                {label: t("tab.context.open_new_window"), shortcut: "⌘⇧N", onClick: () => openInNewWindow(tab)},
            ]);
        }

        return sections;
    }

    function tabIcon(tab: Tab): string {
        if (tab.type === "home") return "㋡";
        if (tab.type === "local") return "$";
        if (tab.type === "forward") return "F";
        if (tab.type === "edit") return "ᝰ";
        return tab.label.charAt(0).toUpperCase();
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
        if (!drawerOpen && touchStartX < 50 && dx > 60 && dy < dx) openDrawer();
        if (drawerOpen && dx < -60 && dy < Math.abs(dx)) closeDrawer();
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
<div class="shell" ontouchstart={handleTouchStart} ontouchend={handleTouchEnd}>

    {#if drawerOpen}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="backdrop" onclick={closeDrawer}></div>
    {/if}

    <!-- Sidebar: 40px collapsed ↔ 260px expanded -->
    <nav
        class="sidebar" class:open={drawerOpen}
        onmouseenter={enterSidebar} onmouseleave={leaveSidebar}
    >
        <div class="sidebar-inner">
            <!-- Home tab -->
            {#each app.tabs().filter(t => t.type === "home") as tab (tab.id)}
                <button
                    class="sb-item"
                    class:active={!app.settingsActive() && tab.id === app.activeTabId()}
                    class:focused={isFocused("tab", tab.id)}
                    onclick={() => selectTab(tab.id)}
                    title={tab.label}
                >
                    <span class="sb-icon">{tabIcon(tab)}</span>
                    <span class="sb-label">{tab.label}</span>
                </button>
            {/each}

            <!-- New Terminal (desktop only) -->
            {#if !app.isMobile}
            <button class="sb-item new-tab" class:focused={isFocused("new-tab")} onclick={addLocalTab} title={t("tab.new_terminal")}>
                <span class="sb-icon">+</span>
                <span class="sb-label">{t("tab.new_terminal")}</span>
            </button>
            <button class="sb-item new-tab" class:focused={isFocused("new-edit")} onclick={addEditTab} title={t("tab.new_edit")}>
                <span class="sb-icon">✎</span>
                <span class="sb-label">{t("tab.new_edit")}</span>
            </button>
            {/if}

            {#if pinnedProfiles.length > 0}
                <div class="sidebar-section">
                    {#each pinnedProfiles as p, i (p.id)}
                        <button
                            class="sb-item pinned"
                            class:focused={isFocused("pin", p.id)}
                            onclick={() => connectPinned(p)}
                            title={p.name}
                        >
                            <span class="sb-icon">{p.name.charAt(0).toUpperCase()}</span>
                            <span class="sb-label">{p.name}</span>
                        </button>
                    {/each}
                </div>
            {/if}

            <div class="sidebar-list">
                {#each app.tabs().filter(t => t.type !== "home") as tab (tab.id)}
                    {@const groupColor = tabGroupColor(tab)}
                    <button
                        class="sb-item"
                        class:active={!app.settingsActive() && tab.id === app.activeTabId()}
                        class:focused={isFocused("tab", tab.id)}
                        class:drag-over={dropTabId === tab.id && dragTabId !== tab.id}
                        draggable="true"
                        ondragstart={(e) => handleDragStart(e, tab.id)}
                        ondragover={(e) => handleDragOver(e, tab.id)}
                        ondrop={(e) => handleDrop(e, tab.id)}
                        ondragend={handleDragEnd}
                        onclick={() => selectTab(tab.id)}
                        title={tab.label}
                    >
                        <span class="sb-icon" style={groupColor ? `background: ${groupColor}; color: white` : ''}>{tabIcon(tab)}</span>
                        <span class="sb-label">{tab.label}</span>
                        <span
                            class="sb-close"
                            role="button"
                            tabindex="-1"
                            onclick={(e) => { e.stopPropagation(); app.closeTab(tab.id); }}
                        >&times;</span>
                    </button>
                {/each}
            </div>

            <div class="sidebar-footer">
                {#if !app.isMobile}
                    <button
                        class="sb-item"
                        class:pinned
                        class:focused={isFocused("pin-window")}
                        onclick={togglePin}
                        title={t("window.pin")}
                    >
                        <span class="sb-icon">📌</span>
                        <span class="sb-label">{t("window.pin")}</span>
                    </button>
                {/if}
                <button
                    class="sb-item"
                    class:active={app.settingsActive()}
                    class:focused={isFocused("settings")}
                    onclick={selectSettings}
                    title={t("tab.settings")}
                >
                    <span class="sb-icon">⚙</span>
                    <span class="sb-label">{t("tab.settings")}</span>
                </button>
            </div>
        </div>
    </nav>

    <div class="content">
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
</div>

<style>
    .shell {
        height: 100%;
        position: relative;
    }

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

    .sidebar-section {
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

    /* ── Sidebar item ── */
    .sb-item {
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        height: 30px;
        padding: 0 4px;
        border: none;
        border-radius: 6px;
        background: transparent;
        color: var(--text-sub);
        font-family: inherit;
        font-size: 13px;
        cursor: pointer;
        transition: all 0.15s;
        text-align: left;
        flex-shrink: 0;
    }

    .sb-item:hover, .sb-item.focused {
        background: var(--surface);
        color: var(--text);
    }

    .sb-item.active {
        background: var(--accent-soft);
        color: var(--accent);
        font-weight: 600;
    }

    .sb-item.focused {
        outline: 1px solid var(--accent);
        outline-offset: -1px;
    }

    .sb-item.drag-over {
        border-top: 2px solid var(--accent);
    }

    .sb-item.pinned {
        color: var(--warning);
    }

    .sb-icon {
        width: 22px;
        height: 22px;
        display: flex;
        align-items: center;
        justify-content: center;
        flex-shrink: 0;
        font-family: monospace;
        font-size: 12px;
        font-weight: 700;
        border-radius: 4px;
        background: var(--surface);
    }

    .sb-item.active .sb-icon {
        background: var(--accent);
        color: var(--bg);
    }

    .sb-label {
        flex: 1;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        min-width: 0;
    }

    .sb-close {
        font-size: 14px;
        line-height: 1;
        opacity: 0;
        transition: opacity 0.1s;
        flex-shrink: 0;
        padding: 0 2px;
    }

    .sb-item:hover .sb-close {
        opacity: 0.4;
    }

    .sb-close:hover {
        opacity: 1 !important;
        color: var(--error);
    }

    /* ── Backdrop ── */
    .backdrop {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.4);
        z-index: 100;
    }

    /* ── Content (offset by collapsed sidebar width) ── */
    .content {
        height: 100%;
        position: relative;
        margin-left: 40px;
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
