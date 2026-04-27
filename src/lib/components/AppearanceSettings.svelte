<script lang="ts">
    import { onMount } from "svelte";
    import * as app from "../stores/app.svelte.ts";
    import * as ai from "../ai/store.svelte.ts";
    import { t } from "../i18n/index.svelte.ts";

    const positions = [
        { value: "left",   labelKey: "settings.appearance.pos.left" },
        { value: "right",  labelKey: "settings.appearance.pos.right" },
        { value: "top",    labelKey: "settings.appearance.pos.top" },
        { value: "bottom", labelKey: "settings.appearance.pos.bottom" },
    ] as const;

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

    // ─── AI panel position (migrated from AI settings) ────────────────
    let aiPos = $state<"left" | "right">(ai.position());
    function pickAiPos(p: "left" | "right") {
        aiPos = p;
        ai.setPosition(p);
    }
</script>

<div class="page">
    <div class="section-label">{t("settings.appearance.sidebar_position")}</div>
    <div class="layout-grid">
        {#each positions as p}
            <button
                class="layout-card"
                class:active={current === p.value}
                class:disabled={disabled(p.value)}
                disabled={disabled(p.value)}
                onclick={() => pick(p.value)}
            >
                <div class="mini-window">
                    <div class="mini-titlebar">
                        <span class="mini-dot red"></span>
                        <span class="mini-dot yellow"></span>
                        <span class="mini-dot green"></span>
                    </div>
                    <div class="mini-body dir-{p.value}">
                        <div class="mini-sidebar">
                            <div class="mini-sidebar-logo"></div>
                            <div class="mini-sidebar-item active"></div>
                            <div class="mini-sidebar-item"></div>
                            <div class="mini-sidebar-item"></div>
                            <div class="mini-sidebar-item"></div>
                        </div>
                        <div class="mini-main">
                            <div class="mini-main-line w40"></div>
                            <div class="mini-main-line w80"></div>
                            <div class="mini-main-line w60"></div>
                            <div class="mini-main-line w70"></div>
                        </div>
                    </div>
                </div>
                <div class="layout-label">{t(p.labelKey)}</div>
            </button>
        {/each}
    </div>

    <div class="section-label">{t("settings.appearance.ai_panel_position")}</div>
    <div class="layout-grid">
        {#each ["left", "right"] as const as side}
            <button class="layout-card" class:active={aiPos === side} onclick={() => pickAiPos(side)}>
                <div class="mini-window">
                    <div class="mini-titlebar">
                        <span class="mini-dot red"></span>
                        <span class="mini-dot yellow"></span>
                        <span class="mini-dot green"></span>
                    </div>
                    <div class="mini-body dir-{side}">
                        <div class="mini-ai">
                            <div class="mini-ai-line w70"></div>
                            <div class="mini-ai-line w50"></div>
                            <div class="mini-ai-line w60"></div>
                        </div>
                        <div class="mini-main">
                            <div class="mini-main-line w40"></div>
                            <div class="mini-main-line w80"></div>
                            <div class="mini-main-line w60"></div>
                            <div class="mini-main-line w70"></div>
                        </div>
                    </div>
                </div>
                <div class="layout-label">
                    {side === "left" ? t("settings.appearance.ai_panel.left") : t("settings.appearance.ai_panel.right")}
                </div>
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
    /* ── Mini-window preview cards (used by both menu position and AI panel position) ── */
    .layout-grid {
        display: flex;
        gap: 14px;
        flex-wrap: wrap;
    }
    .layout-card {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 8px;
        padding: 8px;
        border: 2px solid transparent;
        border-radius: var(--radius-sm);
        background: transparent;
        cursor: pointer;
        font-family: inherit;
        color: inherit;
        transition: border-color 0.15s, background 0.15s;
    }
    .layout-card:hover:not(.active) {
        background: var(--surface);
    }
    .layout-card.active {
        border-color: var(--accent);
        background: color-mix(in srgb, var(--accent) 8%, transparent);
    }
    .layout-card:disabled,
    .layout-card.disabled {
        opacity: 0.4;
        cursor: not-allowed;
    }
    .layout-card.disabled:hover {
        background: transparent;
    }
    .mini-window {
        width: 160px;
        height: 100px;
        border: 1px solid var(--divider);
        border-radius: 6px;
        background: var(--surface);
        overflow: hidden;
        display: flex;
        flex-direction: column;
    }
    .mini-titlebar {
        height: 14px;
        background: var(--bg);
        border-bottom: 1px solid var(--divider);
        display: flex;
        align-items: center;
        gap: 4px;
        padding: 0 6px;
    }
    .mini-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
    }
    .mini-dot.red    { background: #e05555; }
    .mini-dot.yellow { background: #ddaa33; }
    .mini-dot.green  { background: #4cb88a; }
    .mini-body {
        flex: 1;
        display: flex;
    }
    /* Direction modifiers — shared by AI panel + menu sidebar */
    .mini-body.dir-left   { flex-direction: row;            }
    .mini-body.dir-right  { flex-direction: row-reverse;    }
    .mini-body.dir-top    { flex-direction: column;         }
    .mini-body.dir-bottom { flex-direction: column-reverse; }

    /* AI panel (purple) */
    .mini-ai {
        width: 38%;
        background: color-mix(in srgb, #a855f7 22%, var(--surface));
        border-right: 1px solid color-mix(in srgb, #a855f7 35%, transparent);
        display: flex;
        flex-direction: column;
        justify-content: center;
        gap: 4px;
        padding: 0 6px;
    }
    .mini-body.dir-right .mini-ai {
        border-right: none;
        border-left: 1px solid color-mix(in srgb, #a855f7 35%, transparent);
    }
    .mini-ai-line {
        height: 3px;
        border-radius: 2px;
        background: color-mix(in srgb, #a855f7 60%, transparent);
    }

    /* Menu sidebar (accent / blue) */
    .mini-sidebar {
        background: color-mix(in srgb, var(--accent) 18%, var(--surface));
        display: flex;
        gap: 3px;
        padding: 4px 5px;
        border-color: color-mix(in srgb, var(--accent) 35%, transparent);
        border-style: solid;
        border-width: 0;
    }
    .mini-body.dir-left   .mini-sidebar { width: 28%; flex-direction: column; align-items: center; border-right-width: 1px; }
    .mini-body.dir-right  .mini-sidebar { width: 28%; flex-direction: column; align-items: center; border-left-width: 1px; }
    .mini-body.dir-top    .mini-sidebar { height: 26%; flex-direction: row;    align-items: center; border-bottom-width: 1px; }
    .mini-body.dir-bottom .mini-sidebar { height: 26%; flex-direction: row;    align-items: center; border-top-width: 1px; }

    .mini-sidebar-logo {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--accent);
        flex-shrink: 0;
    }
    .mini-sidebar-item {
        background: color-mix(in srgb, var(--accent) 35%, transparent);
        border-radius: 2px;
        flex-shrink: 0;
    }
    .mini-sidebar-item.active {
        background: var(--accent);
    }
    /* Vertical sidebar — items are short horizontal bars */
    .mini-body.dir-left   .mini-sidebar-item,
    .mini-body.dir-right  .mini-sidebar-item { width: 60%;  height: 4px; }
    /* Horizontal sidebar — items are short vertical bars */
    .mini-body.dir-top    .mini-sidebar-item,
    .mini-body.dir-bottom .mini-sidebar-item { width: 14px; height: 6px; }

    .mini-main {
        flex: 1;
        display: flex;
        flex-direction: column;
        justify-content: center;
        gap: 4px;
        padding: 6px 8px;
        background: var(--bg);
    }
    .mini-main-line {
        height: 3px;
        border-radius: 2px;
        background: var(--text-dim);
        opacity: 0.5;
    }
    .w40 { width: 40%; }
    .w50 { width: 50%; }
    .w60 { width: 60%; }
    .w70 { width: 70%; }
    .w80 { width: 80%; }
    .layout-label {
        font-size: 12px;
        color: var(--text-sub);
    }
    .layout-card.active .layout-label {
        color: var(--text);
        font-weight: 600;
    }
</style>
