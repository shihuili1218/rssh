<script module lang="ts">
    import type { Profile, Tab } from "../stores/app.svelte.ts";

    export type NavItem =
        | { kind: "tab"; tab: Tab }
        | { kind: "new-tab" }
        | { kind: "new-edit" }
        | { kind: "pin"; profile: Profile }
        | { kind: "pin-window" }
        | { kind: "downloads" }
        | { kind: "settings" };

    export function navItemKey(item: NavItem): string {
        if (item.kind === "tab") return `tab:${item.tab.id}`;
        if (item.kind === "pin") return `pin:${item.profile.id}`;
        return item.kind;
    }
</script>

<script lang="ts">
    import { t } from "../i18n/index.svelte.ts";

    interface Props {
        item: NavItem;
        active?: boolean;
        focused?: boolean;
        dragOver?: boolean;
        pinnedState?: boolean;
        groupColor?: string | null;
        showClose?: boolean;
        horizontal?: boolean;
        badge?: string | null;
        onActivate: () => void;
        onClose?: () => void;
        onDragStart?: (e: DragEvent) => void;
        onDragOver?: (e: DragEvent) => void;
        onDrop?: (e: DragEvent) => void;
        onDragEnd?: (e: DragEvent) => void;
    }

    let {
        item,
        active = false,
        focused = false,
        dragOver = false,
        pinnedState = false,
        groupColor = null,
        showClose = false,
        horizontal = false,
        badge = null,
        onActivate,
        onClose,
        onDragStart,
        onDragOver,
        onDrop,
        onDragEnd,
    }: Props = $props();

    function iconOf(i: NavItem): string {
        if (i.kind === "new-tab") return "+";
        if (i.kind === "new-edit") return "✎";
        if (i.kind === "pin") return i.profile.name.charAt(0).toUpperCase();
        if (i.kind === "pin-window") return "📌";
        if (i.kind === "downloads") return "⇅";
        if (i.kind === "settings") return "⚙";
        // tab
        const tab = i.tab;
        if (tab.type === "home") return "㋡";
        if (tab.type === "local") return "$";
        if (tab.type === "forward") return "F";
        if (tab.type === "edit") return "ᝰ";
        return tab.label.charAt(0).toUpperCase();
    }

    function labelOf(i: NavItem): string {
        if (i.kind === "tab") return i.tab.label;
        if (i.kind === "pin") return i.profile.name;
        if (i.kind === "new-tab") return t("tab.new_terminal");
        if (i.kind === "new-edit") return t("tab.new_edit");
        if (i.kind === "pin-window") return t("window.pin");
        if (i.kind === "downloads") return t("tab.downloads");
        return t("tab.settings");
    }

    let icon = $derived(iconOf(item));
    let label = $derived(labelOf(item));
    // Pin-on-top, when ON, uses the selected/active style (it *is* a toggle);
    // pinned profiles keep the warning tint because they're shortcuts, not state.
    let tinted = $derived(item.kind === "pin");
    let showActive = $derived(active || (item.kind === "pin-window" && pinnedState));
    let draggable = $derived(!!onDragStart);
    // In horizontal mode, static function entries show icon; content items
    // (tabs, pinned profiles) show the user-supplied label.
    let iconOnly = $derived(horizontal && item.kind !== "tab" && item.kind !== "pin");
</script>

<button
    class="sb-item"
    class:active={showActive}
    class:focused
    class:drag-over={dragOver}
    class:pinned={tinted}
    class:horizontal
    class:icon-only={iconOnly}
    draggable={draggable ? "true" : undefined}
    onclick={onActivate}
    ondragstart={onDragStart}
    ondragover={onDragOver}
    ondrop={onDrop}
    ondragend={onDragEnd}
    title={label}
    style={horizontal && showActive && groupColor ? `--accent: ${groupColor}; --accent-soft: color-mix(in srgb, ${groupColor} 15%, transparent)` : null}
>
    <span class="sb-icon-wrap">
        <span class="sb-icon" style={groupColor ? `background: ${groupColor}; color: white` : ''}>{icon}</span>
        {#if badge}
            <span class="sb-badge">{badge}</span>
        {/if}
    </span>
    <span class="sb-label">{label}</span>
    {#if showClose && onClose}
        <span
            class="sb-close"
            role="button"
            tabindex="-1"
            onclick={(e) => { e.stopPropagation(); onClose?.(); }}
        >&times;</span>
    {/if}
</button>

<style>
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

    .sb-icon-wrap {
        position: relative;
        display: flex;
        flex-shrink: 0;
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

    .sb-badge {
        position: absolute;
        top: -4px;
        right: -6px;
        min-width: 14px;
        height: 14px;
        padding: 0 3px;
        border-radius: 7px;
        background: var(--error, #d64444);
        color: white;
        font-size: 9px;
        font-weight: 700;
        line-height: 14px;
        text-align: center;
        box-shadow: 0 0 0 2px var(--bg);
        pointer-events: none;
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

    /* ── Horizontal mode (StripBar): label-only + max-width + ellipsis ── */
    .sb-item.horizontal {
        width: auto;
        height: 32px;
        padding: 0 10px;
        background: var(--surface);
    }
    .sb-item.horizontal:hover,
    .sb-item.horizontal.focused {
        background: var(--divider);
    }
    .sb-item.horizontal .sb-icon { display: none; }
    .sb-item.horizontal .sb-label {
        flex: none;
        max-width: 140px;
    }
    /* Function entries: icon only, compact square-ish button */
    .sb-item.horizontal.icon-only {
        padding: 0 5px;
        gap: 0;
    }
    .sb-item.horizontal.icon-only .sb-icon { display: flex; }
    .sb-item.horizontal.icon-only .sb-label { display: none; }
    .sb-item.horizontal.drag-over {
        border-top: none;
        border-left: 2px solid var(--accent);
    }
</style>
