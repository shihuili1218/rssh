<script lang="ts">
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
</script>

<div class="page">
    <h3>{t("settings.appearance.sidebar_position")}</h3>
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
</div>

<style>
    .page {
        padding: 24px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }
    h3 {
        font-size: 15px;
        font-weight: 700;
        color: var(--text);
        margin: 0;
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
