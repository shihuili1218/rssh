<script module lang="ts">
    import type { Profile, Tab } from "../stores/app.svelte.ts";

    export type NavItem =
        | { kind: "tab"; tab: Tab }
        | { kind: "new-tab" }
        | { kind: "new-edit" }
        | { kind: "pin"; profile: Profile }
        | { kind: "pinned-menu" }
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
        /** Vertical sidebar only: inline width override during Ctrl+Tab ripple
         *  (null → CSS :hover / .fill governs). Ignored in horizontal mode. */
        width?: number | null;
        /** Vertical sidebar only: touch drawer open → row fills full width. */
        fill?: boolean;
        badge?: string | null;
        redDot?: boolean;
        onActivate: (e?: MouseEvent) => void;
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
        width = null,
        fill = false,
        badge = null,
        redDot = false,
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
        if (i.kind === "pinned-menu") return "★";
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
        if (i.kind === "pinned-menu") return t("tab.pinned_menu");
        if (i.kind === "pin-window") return t("window.pin");
        if (i.kind === "downloads") return t("tab.downloads");
        return t("tab.settings");
    }

    let icon = $derived(iconOf(item));
    let label = $derived(labelOf(item));
    // Pin-on-top, when ON, uses the selected/active style (it *is* a toggle);
    // pinned profiles keep the warning tint because they're shortcuts, not state.
    let tinted = $derived(item.kind === "pin" || item.kind === "pinned-menu");
    let showActive = $derived(active || (item.kind === "pin-window" && pinnedState));
    let draggable = $derived(!!onDragStart);
    // In horizontal mode, static function entries show icon; content items
    // (tabs, pinned profiles) show the user-supplied label.
    let iconOnly = $derived(horizontal && item.kind !== "tab" && item.kind !== "pin");

    // Merge the two inline-style overrides into one string: the vertical ripple
    // width and the horizontal group-color accent never co-occur, but both ride
    // the same `style` attribute.
    let widthStyle = $derived(!horizontal && width != null ? `width: ${width}px;` : "");
    let accentStyle = $derived(
        horizontal && showActive && groupColor
            ? `--accent: ${groupColor}; --accent-soft: color-mix(in srgb, ${groupColor} 15%, transparent);`
            : ""
    );
    let styleAttr = $derived(`${widthStyle}${accentStyle}` || null);

    // During Ctrl+Tab cycling the row carries an inline ripple width. Use that
    // same signal to lift the row off the terminal (distinct surface + shadow)
    // so non-focused rows don't blend into the same-colored background.
    let rippling = $derived(!horizontal && width != null);
</script>

<button
    class="sb-item"
    class:active={showActive}
    class:focused
    class:drag-over={dragOver}
    class:pinned={tinted}
    class:horizontal
    class:icon-only={iconOnly}
    class:fill
    class:rippling
    draggable={draggable ? "true" : undefined}
    onclick={(e) => onActivate(e)}
    ondragstart={onDragStart}
    ondragover={onDragOver}
    ondrop={onDrop}
    ondragend={onDragEnd}
    title={label}
    data-transfers-trigger={item.kind === "downloads" ? "true" : undefined}
    style={styleAttr}
>
    <span class="sb-icon-wrap">
        <span class="sb-icon" style={groupColor ? `background: ${groupColor}; color: white` : ''}>{icon}</span>
        {#if badge}
            <span class="sb-badge">{badge}</span>
        {:else if redDot}
            <span class="sb-dot" aria-hidden="true"></span>
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

    /* ── Vertical sidebar (AppShell): 40px rail ↔ per-row expansion ──
       Each row owns its width. Hover = cliff (only this row grows). Ctrl+Tab
       sets an inline width (ripple) that overrides these. Touch drawer adds
       .fill. overflow:hidden clips the label + close button beyond the
       collapsed 40px so they neither show nor catch clicks until the row
       expands. Placed BEFORE the generic :hover/.active rules so those win the
       equal-specificity background/color ties, while the width rules here keep
       their :not edge. */
    .sb-item:not(.horizontal) {
        width: 40px;
        padding: 0 9px;            /* centers the 22px icon in the 40px rail */
        overflow: hidden;
        pointer-events: auto;      /* opt back in; the overlay around us is none */
        background: var(--bg);     /* solid so cycle rows read over the terminal */
        box-shadow: 0 1px 2px rgba(0, 0, 0, 0.16);
        transition: width 400ms cubic-bezier(0.34, 1.56, 0.64, 1),
                    background 0.15s, color 0.15s;
    }
    .sb-item:not(.horizontal):hover { width: 240px; }   /* cliff: only the hovered row */
    .sb-item:not(.horizontal).fill  { width: 100%; }     /* touch drawer: every row full */
    /* Ctrl+Tab ripple: lift rows off the same-colored terminal so the full
       expanded silhouette (including non-focused rows) is legible. */
    .sb-item:not(.horizontal).rippling {
        background: var(--surface);
        /*box-shadow: var(--raised);*/
    }
    /* Keep the active tab's accent tint visible during the ripple. */
    .sb-item:not(.horizontal).rippling.active { background: var(--accent-soft); }

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
        background: var(--error);
        color: var(--white);
        font-size: 9px;
        font-weight: 700;
        line-height: 14px;
        text-align: center;
        box-shadow: 0 0 0 2px var(--bg);
        pointer-events: none;
    }

    .sb-dot {
        position: absolute;
        top: -2px;
        right: -2px;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--error);
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
