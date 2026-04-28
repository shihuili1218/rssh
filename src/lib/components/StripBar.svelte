<script lang="ts">
    import MenuButton, {type NavItem, navItemKey} from "./MenuButton.svelte";
    import type { Tab } from "../stores/app.svelte.ts";

    interface Props {
        sections: NavItem[][];
        position: "top" | "bottom";
        pinned: boolean;
        dragTabId: string | null;
        dropTabId: string | null;
        xferBadge?: string | null;
        isActiveItem: (item: NavItem) => boolean;
        isFocusedItem: (item: NavItem) => boolean;
        groupColorOf: (tab: Tab) => string | null;
        onActivate: (item: NavItem) => void;
        onClose: (tabId: string) => void;
        onDragStart: (e: DragEvent, tabId: string) => void;
        onDragOver: (e: DragEvent, tabId: string) => void;
        onDrop: (e: DragEvent, tabId: string) => void;
        onDragEnd: (e: DragEvent) => void;
    }

    let {
        sections, position, pinned,
        dragTabId, dropTabId, xferBadge = null,
        isActiveItem, isFocusedItem, groupColorOf,
        onActivate, onClose,
        onDragStart, onDragOver, onDrop, onDragEnd,
    }: Props = $props();

    // Skip empty sections so no orphan separators render.
    let visibleSections = $derived(sections.filter(s => s.length > 0));

    let el: HTMLElement;

    // Auto-scroll when dragging near the left/right edge — keyed on cursor
    // position, not item hover, so long lists can be reordered end-to-end.
    const EDGE = 60;
    const MAX_SPEED = 14;
    let rafId = 0;
    let dxPerFrame = 0;

    function pump() {
        if (!dxPerFrame) { rafId = 0; return; }
        el.scrollLeft += dxPerFrame;
        rafId = requestAnimationFrame(pump);
    }

    function updateAutoScroll(clientX: number) {
        if (!el) return;
        const rect = el.getBoundingClientRect();
        const leftZone = Math.max(0, EDGE - (clientX - rect.left));
        const rightZone = Math.max(0, EDGE - (rect.right - clientX));
        dxPerFrame = rightZone > 0 ? Math.min(MAX_SPEED, rightZone / 4)
                   : leftZone > 0 ? -Math.min(MAX_SPEED, leftZone / 4)
                   : 0;
        if (dxPerFrame && !rafId) rafId = requestAnimationFrame(pump);
    }

    function stopAutoScroll() {
        dxPerFrame = 0;
        if (rafId) { cancelAnimationFrame(rafId); rafId = 0; }
    }

    // Non-tab / home-tab items are not draggable — match Sidebar behaviour.
    function draggableTab(item: NavItem): Tab | null {
        return item.kind === "tab" && item.tab.type !== "home" ? item.tab : null;
    }
</script>

<nav
    class="stripbar"
    class:top={position === "top"}
    class:bottom={position === "bottom"}
    bind:this={el}
    ondragleave={stopAutoScroll}
>
    {#each visibleSections as section, i}
        {#if i > 0}
            <!-- Separator before the last (footer) section also pushes
                 it to the right end via margin-left: auto. -->
            <div class="sep" class:push-right={i === visibleSections.length - 1}></div>
        {/if}
        {#each section as item (navItemKey(item))}
            {@const t = draggableTab(item)}
            {@const itemTab = item.kind === "tab" ? item.tab : null}
            <MenuButton
                horizontal
                {item}
                active={isActiveItem(item)}
                focused={isFocusedItem(item)}
                pinnedState={pinned}
                badge={item.kind === "downloads" ? xferBadge : null}
                dragOver={t !== null && dropTabId === t.id && dragTabId !== t.id}
                groupColor={itemTab ? groupColorOf(itemTab) : null}
                showClose={t !== null}
                onActivate={() => onActivate(item)}
                onClose={t ? () => onClose(t.id) : undefined}
                onDragStart={t ? (e) => onDragStart(e, t.id) : undefined}
                onDragOver={t ? (e) => { onDragOver(e, t.id); updateAutoScroll(e.clientX); } : undefined}
                onDrop={t ? (e) => { stopAutoScroll(); onDrop(e, t.id); } : undefined}
                onDragEnd={t ? (e) => { stopAutoScroll(); onDragEnd(e); } : undefined}
            />
        {/each}
    {/each}
</nav>

<style>
    .stripbar {
        position: fixed;
        left: 0;
        right: 0;
        z-index: 200;
        background: var(--bg);
        display: flex;
        flex-direction: row;
        align-items: center;
        gap: 4px;
        padding: 6px 8px;
        overflow-x: auto;
        overflow-y: hidden;
        -webkit-overflow-scrolling: touch;
        scrollbar-width: none;
        -ms-overflow-style: none;
    }
    .stripbar::-webkit-scrollbar { display: none; }

    .sep {
        flex-shrink: 0;
        width: 1px;
        align-self: stretch;
        margin: 4px 4px;
        background: var(--divider);
    }
    .sep.push-right {
        margin-left: auto;
    }
    .stripbar.top {
        top: env(safe-area-inset-top, 0px);
        border-bottom: 1px solid var(--divider);
    }
    .stripbar.bottom {
        bottom: 0;
        border-top: 1px solid var(--divider);
    }
</style>
