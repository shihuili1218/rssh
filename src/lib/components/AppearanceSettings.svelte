<script lang="ts">
    import { onMount } from "svelte";
    import * as app from "../stores/app.svelte.ts";
    import { t } from "../i18n/index.svelte.ts";

    const positions: { value: app.SidebarPosition; labelKey: string }[] = [
        { value: "left",   labelKey: "settings.appearance.pos.left" },
        { value: "right",  labelKey: "settings.appearance.pos.right" },
        { value: "top",    labelKey: "settings.appearance.pos.top" },
        { value: "bottom", labelKey: "settings.appearance.pos.bottom" },
    ];

    // "bottom" collides with MobileKeybar on mobile — block the choice there,
    // don't leave the user to discover the clash after picking.
    function disabled(value: app.SidebarPosition): boolean {
        return app.isMobile && value === "bottom";
    }

    function pick(value: app.SidebarPosition) {
        if (disabled(value)) return;
        app.setSidebarPosition(value);
    }

    let current = $derived(app.sidebarPosition());

    let commandBlockBar = $state(true);
    onMount(async () => { commandBlockBar = await app.loadCommandBlockBar(); });
    async function saveCommandBlockBar() { await app.setCommandBlockBar(commandBlockBar); }
</script>

<div class="page">
    <div class="section-label">{t("settings.appearance.sidebar_position")}</div>
    <div class="segmented">
        {#each positions as p}
            <button
                class="seg-btn"
                class:active={current === p.value}
                disabled={disabled(p.value)}
                onclick={() => pick(p.value)}
            >
                {t(p.labelKey)}
            </button>
        {/each}
    </div>

    <div class="section-label">TERMINAL DISPLAY</div>
    <div class="switch-card">
        <div class="switch-card-body">
            <div class="switch-card-title" class:on={commandBlockBar} class:off={!commandBlockBar}>COMMAND BLOCK BAR</div>
            <div class="switch-card-desc">Show a colored side bar next to each command to visually group its input and output. A gray bar marks full-screen programs (vim, top, less).</div>
        </div>
        <label class="switch">
            <input type="checkbox" bind:checked={commandBlockBar} onchange={saveCommandBlockBar} />
            <span class="slider"></span>
        </label>
    </div>
</div>

<style>
    .page {
        padding: 24px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }
    .segmented {
        display: inline-flex;
        border: 1px solid var(--divider);
        border-radius: var(--radius-sm);
        overflow: hidden;
        width: fit-content;
    }
    .seg-btn {
        padding: 8px 18px;
        border: none;
        background: var(--bg);
        color: var(--text-sub);
        font-family: inherit;
        font-size: 13px;
        cursor: pointer;
        border-right: 1px solid var(--divider);
    }
    .seg-btn:last-child { border-right: none; }
    .seg-btn:hover:not(:disabled):not(.active) {
        background: var(--surface);
        color: var(--text);
    }
    .seg-btn.active {
        background: var(--accent);
        color: #fff;
        font-weight: 600;
    }
    .seg-btn:disabled {
        opacity: 0.4;
        cursor: not-allowed;
    }
</style>
