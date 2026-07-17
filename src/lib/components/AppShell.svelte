<script lang="ts">
    import {onMount} from "svelte";
    import {invoke} from "@tauri-apps/api/core";
    import {getCurrentWindow} from "@tauri-apps/api/window";
    import type {Profile, Tab, Group} from "../stores/app.svelte.ts";
    import * as app from "../stores/app.svelte.ts";
    import * as updates from "../stores/updates.svelte.ts";
    import * as syncStatus from "../stores/sync.svelte.ts";
    import HomeScreen from "./HomeScreen.svelte";
    import TerminalPane from "./TerminalPane.svelte";
    import ForwardPane from "./ForwardPane.svelte";
    import EditPane from "./EditPane.svelte";
    import SettingsLayout from "./SettingsLayout.svelte";
    import SftpBrowser from "./SftpBrowser.svelte";
    import DownloadsScreen from "./DownloadsScreen.svelte";
    import SnippetPicker from "./SnippetPicker.svelte";
    import Modal from "./Modal.svelte";
    import * as transfers from "../stores/transfers.svelte.ts";
    import TabContextMenu, {type CtxMenuItem} from "./TabContextMenu.svelte";
    import MenuButton, {type NavItem, navItemKey} from "./MenuButton.svelte";
    import StripBar from "./StripBar.svelte";
    import { rippleWidth } from "./sidebar-ripple.ts";
    import ChatPanel from "../ai/ChatPanel.svelte";
    import * as ai from "../ai/store.svelte.ts";
    import type { AiTargetKind } from "../ai/types.ts";
    import {attachShortcuts, attachKeyup, digitTabIndex, type Shortcut} from "../keyboard/registry.ts";
    import {matchBinding, TAB_CYCLE} from "../keyboard/keymap.ts";
    import * as keymap from "../stores/keymap.svelte.ts";
    import {t, errMsg} from "../i18n/index.svelte.ts";
    import {toast} from "../stores/toast.svelte.ts";
    import {initializePrimarySessionWindow} from "./primary-session-window.ts";
    import {defaultPanelWidth, fitPanelWidths, resizePanelWidth} from "./panel-widths.ts";

    let drawerOpen = $state(false);
    let focusIdx = $state(-1);
    let tabCycling = $state(false);
    let profiles = $state<Profile[]>([]);
    let groups = $state<Group[]>([]);
    let menuCtx = $state<{ x: number; y: number; tab: Tab } | null>(null);
    let pinnedMenu = $state<{ x: number; y: number } | null>(null);
    let pinned = $state(false);
    const bypassStartupReconcile = !!window.__rssh_clone || !!window.__rssh_ai_handoff;
    let resourcePanesAllowed = $state(bypassStartupReconcile);
    let navigationLoad = 0;

    async function refreshNavigationData() {
        const current = ++navigationLoad;
        try {
            const [nextProfiles, nextGroups] = await Promise.all([
                app.loadProfiles(),
                app.loadGroups(),
            ]);
            if (current !== navigationLoad) return;
            profiles = nextProfiles;
            groups = nextGroups;
        } catch (error) {
            console.warn("[sync] navigation data refresh failed:", error);
        }
    }

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

    /* 关闭 tab 的唯一入口：所有关闭路径（快捷键 / 右键菜单 / tab 上的 × 按钮）都走这里。
       开了二次确认就先弹窗存住待关的 tab，否则直接关。home tab 不可关，跟 closeTab 一致。 */
    let closingTab = $state<Tab | null>(null);
    function requestCloseTab(id: string) {
        const tab = app.tabs().find(t => t.id === id);
        if (!tab || tab.type === "home") return;
        if (app.confirmCloseTab()) { closingTab = tab; return; }
        app.closeTab(id);
    }
    function confirmCloseTab() {
        if (closingTab) app.closeTab(closingTab.id);
        closingTab = null;
    }

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
                    requestCloseTab(id);
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
                    // serial excluded: the port is exclusive, a second window would fail.
                    if (!tab || !canOpenTabInNewWindow(tab) || app.isMobile) return false;
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
                    const tab = app.activeTab();
                    if (tab && ai.isOpen(tab.id)) {
                        void ai.closePanel(tab.id).catch((e) => {
                            console.warn("[ai] close panel shortcut:", e);
                        });
                        return;
                    }
                    const canOpen = !!tab && app.isAiCapableTabType(tab.type) && !!app.sessionIdForTab(tab.id);
                    if (!canOpen) return false;
                    ai.openPanel(tab.id);
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
                display: keymap.isMac ? "⌘1 … ⌘9" : "Alt+1 … Alt+9",
                description: t("shortcut.tab.goto"),
                // Direct jump to the Nth tab (1-based, by strip order). A fixed
                // combo like Ctrl+Tab — kept out of the customizable ACTIONS
                // editor since it is 9 combos, not one bindable action. Home
                // (index 0) is skipped, so the combo lands on the first session
                // tab. Cmd+1..9 on macOS, Alt+1..9 elsewhere — see digitTabIndex.
                match: e => digitTabIndex(e, keymap.isMac) !== null,
                handler: e => {
                    // Don't hijack keys while the user is recording a new binding.
                    if (keymap.recording()) return false;
                    const idx = digitTabIndex(e, keymap.isMac);
                    if (idx === null) return false;
                    const tab = app.tabs()[idx];
                    if (!tab) return false; // out of range: don't swallow the key
                    app.setActiveTab(tab.id);
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
        const startup = new AbortController();
        keymap.init();
        // Crash recovery must settle before any pane can create a replacement
        // backend resource. Otherwise reconcile([]) can race that new session.
        //
        // Skip this in cloned windows (window.__rssh_clone is set by
        // open_tab_in_new_window) and AI handoff windows (window.__rssh_ai_handoff
        // is set by analyze_locally tool): passing activeIds=[] would nuke every
        // session in the shared AppState, including other windows' tabs.
        if (!bypassStartupReconcile) {
            void initializePrimarySessionWindow({
                signal: startup.signal,
                reconcile: () => invoke("reconcile_sessions", { activeIds: [] }),
                allowResourcePanes: () => { resourcePanesAllowed = true; },
                loadAutoOpenLocal: async () =>
                    await invoke<string | null>("get_setting", { key: "open_local_on_startup" }) === "true",
                openLocal: addLocalTab,
            });
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
        return () => {
            startup.abort();
            detachKeydown();
            detachKeyup();
        };
    });

    $effect(() => {
        syncStatus.configurationRevision();
        void refreshNavigationData();
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
        ai.openPanel(tabId);
        const lease = ai.captureSessionLease(tabId);

        // 2. Wait for the local PTY to register itself. The store fires
        //    this Promise the moment registerSession runs in TerminalPane —
        //    no polling. 30 s timeout still covers a stuck spawn.
        const sid = await app.waitForSession(tabId, 30000);
        if (!sid) {
            console.error("AI handoff: 本地 PTY 30s 内未就绪，放弃");
            return;
        }
        // 用户等待 PTY 时可能已经手动关掉 AI；关闭现在代表放弃这轮会话，
        // 不能在后台偷偷把面板对应的 actor 又启动起来。
        if (!ai.isOpen(tabId)) return;

        // 3. 启动独立 AI 会话 + 发首条消息
        try {
            const settings = await ai.loadSettings();
            if (!ai.isOpen(tabId)) return;
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
                lease,
            });
            const initialMsg = t("ai.handoff.initial_msg", { path: payload.local_path, task: payload.task });
            await ai.sendMessage(info.tab_id, initialMsg, lease);
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
        // Desktop no longer opens a drawer; refresh pinned profiles when the
        // touch drawer opens OR when Ctrl+Tab cycling begins.
        if (drawerOpen || tabCycling) {
            void refreshNavigationData();
        }
    });

    // Keep the focused row in view while cycling a long, scrollable tab list.
    $effect(() => {
        const i = focusIdx;        // track focusIdx
        if (!tabCycling) return;   // track tabCycling
        void i;
        requestAnimationFrame(() => {
            document.querySelector(".sidebar .sb-item.focused")
                ?.scrollIntoView({ block: "nearest" });
        });
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
        ai.isOpen(aiTabId)
        && !!aiActiveTab
        && app.isAiCapableTabType(aiActiveTab.type)
        && !!aiSessionId
        && !app.settingsActive()
        // The Transfers popover does not affect AI panel visibility — overlay.
    );
    // 跟 SFTP 一样：每个已打开 AI 的 tab 保留一个 keyed ChatPanel 实例。
    // 切 tab 只改可见性，草稿、审计页、滚动位置和命令卡局部状态不会串台或丢失。
    let aiTabs = $derived(
        app.tabs().filter((tab) =>
            ai.isOpen(tab.id)
            && app.isAiCapableTabType(tab.type)
        )
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

    /* ── 两侧面板宽度：preferred value 按 tab 保存，rendered value 按当前容器
       动态收敛；开第二块面板、切回宽 tab 或缩窗都不能把主区挤穿。 */
    const PANEL_MIN_WIDTH = 280;
    const PANEL_MIN_MAIN = 320;
    let aiPanelWidth = $derived(ai.panelWidth(aiTabId));
    let sftpPanelWidth = $derived(app.sftpPanelWidthForTab(app.activeTabId()));
    let contentEl = $state<HTMLDivElement | null>(null);
    let contentWidth = $state(window.innerWidth);
    let viewportWidth = $state(window.innerWidth);

    $effect(() => {
        const el = contentEl;
        if (!el) return;
        const sync = () => {
            contentWidth = el.getBoundingClientRect().width;
            viewportWidth = window.innerWidth;
        };
        sync();
        const observer = new ResizeObserver(sync);
        observer.observe(el);
        return () => observer.disconnect();
    });

    let fittedPanelWidths = $derived(fitPanelWidths({
        containerWidth: contentWidth,
        mainMinWidth: PANEL_MIN_MAIN,
        panelMinWidth: PANEL_MIN_WIDTH,
        defaultWidth: defaultPanelWidth(viewportWidth),
        aiVisible,
        sftpVisible,
        aiWidth: aiPanelWidth,
        sftpWidth: sftpPanelWidth,
    }));

    let aiSideStyle = $derived(
        `flex: 0 0 ${fittedPanelWidths.ai}px; max-width: ${fittedPanelWidths.ai}px;`
    );
    let sftpSideStyle = $derived(
        `flex: 0 0 ${fittedPanelWidths.sftp}px; max-width: ${fittedPanelWidths.sftp}px;`
    );

    let activePanelResizeStop: (() => void) | null = null;
    $effect(() => {
        // 订阅所有会改变 drag owner/visibility 的坐标；一旦变化，旧手势立即失效。
        aiTabId;
        aiVisible;
        sftpVisible;
        app.settingsActive();
        activePanelResizeStop?.();
    });

    /** 另一侧 panel 的当前渲染宽度（aside 元素的 boundingClientRect）；hidden 状态返回 0。
     *  resize 时拿来从可用空间里减掉，避免两个 panel 都拖到极端导致主区被压成 0。 */
    function otherPanelWidth(selector: string): number {
        const el = document.querySelector(selector);
        return el ? (el as HTMLElement).getBoundingClientRect().width : 0;
    }

    function startPanelResize(e: MouseEvent, options: {
        tabId: string;
        currentWidth: number | null;
        sign: number;
        minWidth: number;
        minMain: number;
        otherSelector: string;
        stillActive: () => boolean;
        setWidth: (tabId: string, width: number) => void;
        setOtherWidth: (tabId: string, width: number) => void;
    }) {
        e.preventDefault();
        activePanelResizeStop?.();
        const startX = e.clientX;
        const sideEl = (e.currentTarget as HTMLElement).parentElement as HTMLElement | null;
        // 取实际渲染宽度作为起点，避免首次拖拽时的"跳变"。
        const measuredWidth = sideEl?.getBoundingClientRect().width ?? 0;
        const startWidth = measuredWidth > 0 ? measuredWidth : (options.currentWidth ?? 380);
        const otherWidthAtStart = otherPanelWidth(options.otherSelector);
        let allocationsSettled = false;
        let stopped = false;

        function stop() {
            if (stopped) return;
            stopped = true;
            document.removeEventListener("mousemove", onMove);
            document.removeEventListener("mouseup", stop);
            window.removeEventListener("blur", stop);
            if (activePanelResizeStop === stop) activePanelResizeStop = null;
        }

        function onMove(ev: MouseEvent) {
            // 拖动期间切 tab、关 panel 或关 tab：结束旧手势，绝不能写到新 tab。
            if (!options.stillActive()) { stop(); return; }
            const dx = ev.clientX - startX;
            if (!allocationsSettled) {
                if (dx === 0) return;
                // fit 可能正把两个 oversized preference 临时压窄。第一次真实移动
                // 先把对侧当前 allocation 落成 preference；整个手势固定以它为界，
                // 当前 panel 缩小后再反向拖动不会被对侧自动扩张锁死。
                allocationsSettled = true;
                if (otherWidthAtStart > 0) {
                    options.setOtherWidth(options.tabId, otherWidthAtStart);
                }
            }
            const next = resizePanelWidth({
                startWidth,
                deltaX: dx,
                sign: options.sign,
                minWidth: options.minWidth,
                containerWidth: contentWidth,
                mainMinWidth: options.minMain,
                otherWidthAtStart,
            });
            options.setWidth(options.tabId, next);
        }
        document.addEventListener("mousemove", onMove);
        document.addEventListener("mouseup", stop);
        window.addEventListener("blur", stop);
        activePanelResizeStop = stop;
    }

    function startAiResize(e: MouseEvent) {
        const tabId = aiTabId;
        startPanelResize(e, {
            tabId,
            currentWidth: ai.panelWidth(tabId),
            // AI 在右：左移变宽；AI 在左：右移变宽。
            sign: aiPos === "left" ? 1 : -1,
            minWidth: PANEL_MIN_WIDTH,
            minMain: PANEL_MIN_MAIN,
            otherSelector: ".sftp-side",
            stillActive: () => aiVisible && app.activeTabId() === tabId,
            setWidth: ai.setPanelWidth,
            setOtherWidth: app.setSftpPanelWidth,
        });
    }

    /** 双击 handle：清除手动宽度，回到响应式默认（媒体查询 + 380px）。 */
    function resetAiWidth() {
        ai.setPanelWidth(aiTabId, null);
    }

    /* ── SFTP 面板宽度：跟 AI 镜像一份，同样按 tab 管理。
       SFTP 永远走 AI 的对侧（aiPos=right → SFTP 左；aiPos=left → SFTP 右），
       靠 .content.ai-left 的 row-reverse 自动翻边，不引入新位置 config。 */
    function startSftpResize(e: MouseEvent) {
        const tabId = app.activeTabId();
        startPanelResize(e, {
            tabId,
            currentWidth: app.sftpPanelWidthForTab(tabId),
            // SFTP 在左：右移变宽；SFTP 在右：左移变宽。
            sign: aiPos === "left" ? -1 : 1,
            minWidth: PANEL_MIN_WIDTH,
            minMain: PANEL_MIN_MAIN,
            otherSelector: ".ai-side",
            stillActive: () => sftpVisible && app.activeTabId() === tabId,
            setWidth: app.setSftpPanelWidth,
            setOtherWidth: ai.setPanelWidth,
        });
    }

    function resetSftpWidth() {
        app.setSftpPanelWidth(app.activeTabId(), null);
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
            // Downloads (transfer queue) is now reachable on mobile too — SFTP
            // single-file transfer runs through it. pin-window stays desktop-only.
            ...(app.isMobile ? [{kind: "downloads" as const}] : [{kind: "pin-window" as const}, {kind: "downloads" as const}]),
            {kind: "settings" as const},
        ],
    });
    let navItems = $derived<NavItem[]>([...navSections.header, ...navSections.middle, ...navSections.footer]);

    // Ctrl+Tab ripple: a row's width falls off with its distance from the
    // focused row. null when not cycling → MenuButton's CSS (:hover / .fill)
    // governs the width instead. See sidebar-ripple.ts.
    function rowWidth(item: NavItem): number | null {
        // Drawer wins over ripple: a touch device with a keyboard could have the
        // swipe drawer open AND hit Ctrl+Tab. Returning null lets MenuButton's
        // .fill keep rows full-width; the focus ring still tracks the cycle.
        if (!tabCycling || drawerOpen) return null;
        const key = navItemKey(item);
        const idx = navItems.findIndex(n => navItemKey(n) === key);
        return idx < 0 ? null : rippleWidth(Math.abs(idx - focusIdx));
    }

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

    function canOpenTabInNewWindow(tab: Tab): boolean {
        return app.isTerminalTabType(tab.type) && tab.type !== "serial";
    }

    function buildMenu(tab: Tab): CtxMenuItem[][] {
        // Serial and telnet are also text terminals — they get copy/paste/search/
        // snippets AND AI (the agent runs commands via manual-submit, no shell
        // sentinel). Serial does NOT get open-in-new-window: a serial port is
        // exclusive — a second window opening the same device would fail. Telnet
        // has no such exclusivity (each window is its own TCP connection).
        const isTextTerminal = app.isTerminalTabType(tab.type);
        const isSsh = tab.type === "ssh";
        const sections: CtxMenuItem[][] = [];

        // Copy / Paste (+ UTC if selection is a timestamp) / Add-to-Snippets.
        if (isTextTerminal) {
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

        if (isTextTerminal) {
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
            // Tab context menu is a desktop right-click affordance; on mobile
            // SFTP opens from the keybar instead.
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

        // Serial control lines: DTR/RTS assert/deassert + break. Runtime ops on
        // the open port (MCU reset, bootloader entry, break-to-debugger). Greyed
        // out until the session exists (briefly during connect / after unplug).
        if (tab.type === "serial") {
            const sid = app.sessionIdForTab(tab.id);
            const ctl = (cmd: string, extra: Record<string, unknown> = {}) => () =>
                void invoke(cmd, {sessionId: sid, ...extra}).catch((e) => toast.error(errMsg(e)));
            sections.push([
                {
                    label: t("serial.ctl"),
                    disabled: !sid,
                    onClick: () => {},
                    submenu: [
                        {label: t("serial.ctl.dtr_assert"), disabled: !sid, onClick: ctl("serial_set_dtr", {level: true})},
                        {label: t("serial.ctl.dtr_deassert"), disabled: !sid, onClick: ctl("serial_set_dtr", {level: false})},
                        {label: t("serial.ctl.rts_assert"), disabled: !sid, onClick: ctl("serial_set_rts", {level: true})},
                        {label: t("serial.ctl.rts_deassert"), disabled: !sid, onClick: ctl("serial_set_rts", {level: false})},
                        {label: t("serial.ctl.break"), disabled: !sid, onClick: ctl("serial_send_break")},
                    ],
                },
            ]);
        }

        sections.push([
            {
                label: t("tab.context.clone"),
                shortcut: tab.type === "home" ? undefined : keymap.format("tab.clone"),
                disabled: tab.type === "home",
                onClick: () => cloneTab(tab),
            },
            {label: t("tab.context.close"), shortcut: keymap.format("tab.close"), onClick: () => requestCloseTab(tab.id)},
        ]);

        // AI 排障入口（ssh/local/serial/telnet tab 才有，且需要已经连上 = 有 sessionId）
        if (app.isAiCapableTabType(tab.type)) {
            const sid = app.sessionIdForTab(tab.id);
            sections.push([
                {
                    label: t("tab.context.ai"),
                    shortcut: keymap.format("ai.toggle"),
                    disabled: !sid,
                    onClick: () => { app.setActiveTab(tab.id); ai.openPanel(tab.id); },
                },
            ]);
        }

        // Multi-window requires Tauri WebviewWindowBuilder — desktop only.
        if (canOpenTabInNewWindow(tab) && !app.isMobile) {
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

    <!-- Sidebar: 40px rail; each row expands on hover (cliff) or Ctrl+Tab
         (ripple). Touch swipe opens the full drawer. Position = left | right. -->
    {#if sbPos === "left" || sbPos === "right"}
    <div class="sidebar-rail">
    <nav
        class="sidebar" class:open={drawerOpen} class:right={sbPos === "right"}
    >
        <div class="sidebar-inner">
            {#each navSections.header as item (navItemKey(item))}
                <MenuButton
                    {item}
                    width={rowWidth(item)}
                    fill={drawerOpen}
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
                        width={rowWidth(item)}
                        fill={drawerOpen}
                        active={isActiveItem(item)}
                        focused={isFocusedItem(item)}
                        dragOver={tab !== null && dropTabId === tab.id && dragTabId !== tab.id}
                        groupColor={tab ? tabGroupColor(tab) : null}
                        showClose={tab !== null}
                        onActivate={(e) => activateNavItem(item, e)}
                        onClose={tab ? () => requestCloseTab(tab.id) : undefined}
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
                        width={rowWidth(item)}
                        fill={drawerOpen}
                        active={isActiveItem(item)}
                        focused={isFocusedItem(item)}
                        pinnedState={pinned}
                        badge={item.kind === "downloads" ? xferBadge : null}
                        redDot={item.kind === "settings" && (updates.hasUpdate() || syncStatus.anyVersionDifference())}
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
            settingsRedDot={updates.hasUpdate() || syncStatus.anyVersionDifference()}
            isActiveItem={isActiveItem}
            isFocusedItem={isFocusedItem}
            groupColorOf={tabGroupColor}
            onActivate={activateNavItem}
            onClose={(id) => requestCloseTab(id)}
            onDragStart={handleDragStart}
            onDragOver={handleDragOver}
            onDrop={handleDrop}
            onDragEnd={handleDragEnd}
        />
    {/if}

    <div
        class="content"
        bind:this={contentEl}
        class:ai-on={aiVisible}
        class:ai-left={aiVisible && aiPos === "left"}
        class:sftp-on={sftpVisible}
    >
        <!-- 任何 tab 开了 SFTP 就把 aside 挂上（保留所有 tab 的 SftpBrowser 实例 → 切回时 cwd 不丢）。
             active tab 没开 / 进入 settings / downloads 时整块 aside 走 .hidden 收掉视觉宽度，但 DOM 留着。
             SFTP 走 AI 对侧：aiPos=right(default) → SFTP 视觉左、handle 右边缘；
             aiPos=left 下 .content.ai-left 翻 row → SFTP 视觉右、handle 左边缘。 -->
        {#if resourcePanesAllowed && sftpTabs.length > 0}
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
                    {:else if app.isTerminalTabType(tab.type) && resourcePanesAllowed}
                        <TerminalPane tabId={tab.id} tabType={tab.type} meta={tab.meta ?? {}}/>
                    {:else if tab.type === "forward" && resourcePanesAllowed}
                        <ForwardPane tabId={tab.id} meta={tab.meta ?? {}}/>
                    {:else if tab.type === "edit"}
                        <EditPane tabId={tab.id} />
                    {/if}
                </div>
            {/each}
        </div>

        <!-- 任何 tab 开了 AI 就保留对应 ChatPanel；只有当前 tab 的 pane 可见。
             不能用 {#if aiVisible} 包住 aside，否则切到没开 AI 的 tab 会销毁旧实例。 -->
        {#if resourcePanesAllowed && aiTabs.length > 0}
            <aside class="ai-side" class:hidden={!aiVisible} style={aiSideStyle}>
                <div class="ai-resize-handle"
                     class:on-right={aiPos === "left"}
                     onmousedown={startAiResize}
                     ondblclick={resetAiWidth}
                     role="separator"
                     aria-orientation="vertical"
                     title={t("common.resize_hint")}></div>
                {#each aiTabs as tab (tab.id)}
                    {@const targetId = app.sessionIdForTab(tab.id) ?? null}
                    {@const active = aiVisible && tab.id === aiTabId}
                    <div class="ai-pane" class:visible={active}>
                        <ChatPanel
                            tabId={tab.id}
                            targetKind={tab.type as AiTargetKind}
                            {targetId}
                            {active}
                        />
                    </div>
                {/each}
            </aside>
        {/if}
    </div>

    <!-- Popover lives inside .shell so it inherits the --sb-* layout vars that
         drive its edge offsets. position:fixed still anchors to the viewport
         because .shell does not create a fixed-positioning containing block. -->
    {#if app.downloadsActive()}
        <DownloadsScreen/>
    {/if}

    <!-- 关闭 tab 二次确认。Tauri webview 不支持原生 confirm()，沿用 ChatPanel/AiSettings
         同款自定义 modal。只有开启「关闭标签页前确认」后 requestCloseTab 才会挂起 closingTab。 -->
    {#if closingTab}
        <Modal onClose={() => (closingTab = null)} class="stack"
               aria-labelledby="close-tab-title" aria-describedby="close-tab-body">
            <h3 id="close-tab-title" class="dialog-title">{t("tab.close_confirm_title")}</h3>
            <div id="close-tab-body" class="dialog-body">{t("tab.close_confirm_body", { label: closingTab.label })}</div>
            <div class="modal-actions">
                <button class="btn btn-sm" onclick={() => (closingTab = null)}>{t("common.cancel")}</button>
                <button class="btn btn-sm btn-primary" onclick={confirmCloseTab}>{t("tab.context.close")}</button>
            </div>
        </Modal>
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

    /* ── Sidebar: rail (40px flow footprint) + transparent overlay ── */
    /* The rail is a 40px flex item that reserves the sidebar's space in normal
       flow, so content sits beside it and can never overlap it. */
    .sidebar-rail {
        flex: 0 0 40px;
        position: relative;
        z-index: 200;
        background: var(--bg);     /* the persistent 40px rail strip */
    }

    /* Overlay spanning the full expanded width (260px), transparent and
       click-through (pointer-events:none); only the rows opt back in, so the
       gap beside a collapsed row passes clicks through to the content below.
       The 40px rail look comes from .sidebar-rail's background + .content's
       divider border. No whole-panel hover-expansion on desktop anymore —
       each row expands on its own (MenuButton). */
    .sidebar {
        position: absolute;
        left: 0;
        top: 0;
        width: 260px;
        height: 100%;
        overflow: hidden;
        pointer-events: none;
    }

    .sidebar.right { left: auto; right: 0; }

    /* Right sidebar: rows hug the right edge and grow leftward. */
    .sidebar.right .sidebar-inner,
    .sidebar.right .sidebar-list,
    .sidebar.right .sidebar-footer { align-items: flex-end; }

    /* Touch drawer (mobile / touch swipe sets drawerOpen): the overlay turns
       into a solid, interactive panel; rows fill it via MenuButton's .fill. */
    .sidebar.open {
        background: var(--bg);
        box-shadow: var(--raised);
        pointer-events: auto;
    }


    /* Inner container always 260px — sidebar clips it */
    .sidebar-inner {
        width: 260px;
        min-width: 260px;
        height: 100%;
        display: flex;
        flex-direction: column;
        padding: 6px 0;            /* no horizontal pad: rows sit flush to the rail */
        gap: 2px;
    }

    /* Section separators: a 40px-wide line painted via background (not a
       full-width border) so it stays inside the rail and never spills across
       the content under the transparent 260px overlay. */
    .sidebar-list {
        padding-top: 2px;
        flex: 1;
        overflow-y: auto;
        /* Scrolls via wheel/drag, but the bar is hidden — the overlay is 260px
           wide and transparent, so a visible bar would float over the terminal.
           Same treatment as StripBar's horizontal overflow. */
        scrollbar-width: none;
        -ms-overflow-style: none;
        display: flex;
        flex-direction: column;
        gap: 2px;
        background: linear-gradient(var(--divider), var(--divider)) no-repeat top left / 40px 1px;
    }
    .sidebar-list::-webkit-scrollbar { display: none; }

    .sidebar-footer {
        padding-top: 6px;
        margin-top: 2px;
        display: flex;
        flex-direction: column;
        gap: 2px;
        background: linear-gradient(var(--divider), var(--divider)) no-repeat top left / 40px 1px;
    }

    .sidebar.right .sidebar-list,
    .sidebar.right .sidebar-footer { background-position: top right; }

    /* Touch drawer: rows fill the panel, so separators span it full-width too. */
    .sidebar.open .sidebar-list,
    .sidebar.open .sidebar-footer { background-size: 100% 1px; }

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
    /* Rail/content divider. Lives on .content (not the overlay) so a collapsed
       row never paints over it; an expanded row floats above it, as intended. */
    .shell.sb-left  .content { border-left: 1px solid var(--divider); }
    .shell.sb-right .content { border-right: 1px solid var(--divider); }
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
    .ai-pane {
        position: absolute;
        inset: 0;
        display: none;
    }
    .ai-pane.visible {
        display: flex;
        flex-direction: column;
    }
    .ai-side.hidden {
        flex: 0 0 0 !important;
        max-width: 0 !important;
        overflow: hidden;
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
        /* inline preferred width 不能压过 mobile takeover。 */
        .ai-side:not(.hidden) {
            flex: 1 1 auto !important;
            max-width: none !important;
        }
        .ai-resize-handle { display: none; }
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

    /* 关闭 tab 二次确认弹窗 —— 外壳（scrim + 卡片）由 Modal.svelte 统一提供，
       标题/正文排版用全局 .dialog-title/.dialog-body。 */
</style>
