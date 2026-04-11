<script lang="ts">
    import {onMount} from "svelte";
    import {getCurrentWindow} from "@tauri-apps/api/window";
    import type {Profile, Tab} from "../stores/app.svelte.ts";
    import * as app from "../stores/app.svelte.ts";
    import HomeScreen from "./HomeScreen.svelte";
    import TerminalPane from "./TerminalPane.svelte";
    import ForwardPane from "./ForwardPane.svelte";
    import SettingsLayout from "./SettingsLayout.svelte";
    import SftpBrowser from "./SftpBrowser.svelte";
    import SnippetPicker from "./SnippetPicker.svelte";

    let drawerOpen = $state(false);
    let focusIdx = $state(-1);
    let sidebarEl: HTMLElement;
    let profiles = $state<Profile[]>([]);
    let sidebarTimer = 0;

    onMount(async () => {
        profiles = await app.loadProfiles();
    });

    $effect(() => {
        if (drawerOpen) app.loadProfiles().then(p => profiles = p);
    });

    $effect(() => {
        const tab = app.activeTab();
        const title = app.settingsActive() ? "Settings" : tab?.label ?? "RSSH";
        getCurrentWindow().setTitle(title);
    });

    let pinnedProfiles = $derived(
        profiles.filter(p => app.pinnedProfileIds().includes(p.id))
    );

    type NavItem = { kind: "pin"; profile: Profile } | { kind: "tab"; id: string } | { kind: "new-tab" } | { kind: "settings" };
    let navItems = $derived<NavItem[]>([
        ...app.tabs().filter(t => t.type === "home").map(t => ({kind: "tab" as const, id: t.id})),
        ...(app.isMobile ? [] : [{kind: "new-tab" as const}]),
        ...pinnedProfiles.map(p => ({kind: "pin" as const, profile: p})),
        ...app.tabs().filter(t => t.type !== "home").map(t => ({kind: "tab" as const, id: t.id})),
        {kind: "settings" as const},
    ]);

    function activateNavItem(item: NavItem) {
        if (item.kind === "new-tab") addLocalTab();
        else if (item.kind === "pin") connectPinned(item.profile);
        else if (item.kind === "tab") selectTab(item.id);
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
        requestAnimationFrame(() => sidebarEl?.focus());
    }

    function closeDrawer() {
        drawerOpen = false;
        focusIdx = -1;
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

    function tabIcon(tab: Tab): string {
        if (tab.type === "home") return "H";
        if (tab.type === "local") return "$";
        if (tab.type === "forward") return "F";
        return tab.label.charAt(0).toUpperCase();
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
        if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            if (drawerOpen) { closeDrawer(); return; }
            openDrawer();
            const activeIdx = navItems.findIndex(
                item => (item.kind === "tab" && item.id === app.activeTabId()) ||
                    (item.kind === "settings" && app.settingsActive())
            );
            focusIdx = activeIdx >= 0 ? activeIdx : 0;
            return;
        }

        if (e.key === "Escape") {
            if (app.sftpOpen()) { app.closeSftp(); e.preventDefault(); }
            else if (drawerOpen) { closeDrawer(); e.preventDefault(); }
            return;
        }

        if (drawerOpen) {
            if (e.key === "ArrowDown" || e.key === "ArrowRight") {
                e.preventDefault();
                focusIdx = (focusIdx + 1) % navItems.length;
            } else if (e.key === "ArrowUp" || e.key === "ArrowLeft") {
                e.preventDefault();
                focusIdx = (focusIdx - 1 + navItems.length) % navItems.length;
            } else if (e.key === "Enter" && focusIdx >= 0 && focusIdx < navItems.length) {
                e.preventDefault();
                activateNavItem(navItems[focusIdx]);
            }
        }
    }
</script>

<svelte:window onkeydown={handleKeydown}/>

{#if app.snippetPickerOpen()}
    <SnippetPicker />
{/if}

{#if app.sftpOpen()}
    <div class="sftp-overlay">
        <div class="sftp-bar">
            <button class="btn btn-sm" onclick={() => app.closeSftp()}>← Back</button>
            <span class="sftp-title">SFTP</span>
        </div>
        <div class="sftp-body">
            <SftpBrowser meta={app.activeTab()?.meta ?? {}}/>
        </div>
    </div>
{/if}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="shell" ontouchstart={handleTouchStart} ontouchend={handleTouchEnd}>

    {#if drawerOpen}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="backdrop" onclick={closeDrawer}></div>
    {/if}

    <!-- Sidebar: single component, 40px collapsed ↔ 260px expanded -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <nav
        class="sidebar" class:open={drawerOpen}
        bind:this={sidebarEl} tabindex="0"
        onmouseenter={enterSidebar} onmouseleave={leaveSidebar}
    >
        <div class="sidebar-inner">
            <!-- Home tab -->
            {#each app.tabs().filter(t => t.type === "home") as tab (tab.id)}
                <button
                    class="sb-item"
                    class:active={!app.settingsActive() && tab.id === app.activeTabId()}
                    class:focused={focusIdx === 0}
                    onclick={() => selectTab(tab.id)}
                    title={tab.label}
                >
                    <span class="sb-icon">{tabIcon(tab)}</span>
                    <span class="sb-label">{tab.label}</span>
                </button>
            {/each}

            <!-- New Terminal (desktop only) -->
            {#if !app.isMobile}
            <button class="sb-item new-tab" class:focused={focusIdx === 1} onclick={addLocalTab} title="New terminal">
                <span class="sb-icon">+</span>
                <span class="sb-label">New Terminal</span>
            </button>
            {/if}

            {#if pinnedProfiles.length > 0}
                <div class="sidebar-section">
                    {#each pinnedProfiles as p, i (p.id)}
                        <button
                            class="sb-item pinned"
                            class:focused={focusIdx === 2 + i}
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
                {#each app.tabs().filter(t => t.type !== "home") as tab, i (tab.id)}
                    {@const idx = 2 + pinnedProfiles.length + i}
                    <button
                        class="sb-item"
                        class:active={!app.settingsActive() && tab.id === app.activeTabId()}
                        class:focused={focusIdx === idx}
                        onclick={() => selectTab(tab.id)}
                        title={tab.label}
                    >
                        <span class="sb-icon">{tabIcon(tab)}</span>
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
                <button
                    class="sb-item"
                    class:active={app.settingsActive()}
                    class:focused={focusIdx === navItems.length - 1}
                    onclick={selectSettings}
                    title="Settings"
                >
                    <span class="sb-icon">S</span>
                    <span class="sb-label">Settings</span>
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
            <div class="pane" class:visible={!app.settingsActive() && tab.id === app.activeTabId()}>
                {#if tab.type === "home"}
                    <HomeScreen/>
                {:else if tab.type === "ssh" || tab.type === "local"}
                    <TerminalPane tabId={tab.id} tabType={tab.type} meta={tab.meta ?? {}}/>
                {:else if tab.type === "forward"}
                    <ForwardPane tabId={tab.id} meta={tab.meta ?? {}}/>
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

    .sidebar:focus {
        outline: none;
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
