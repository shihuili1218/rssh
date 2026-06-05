<script lang="ts">
    import { onMount } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import * as app from "../stores/app.svelte.ts";
    import * as ai from "../ai/store.svelte.ts";
    import * as theme from "../themes/store.svelte.ts";
    import type { PaletteId } from "../themes/palettes.ts";
    import { parseCustomTermJson, type TermPaletteRef } from "../themes/term-palettes.ts";
    import { composeTermFontStack, type FontInfo } from "../themes/term-font.ts";
    import FontSelect from "./FontSelect.svelte";
    import { t } from "../i18n/index.svelte.ts";

    const SCHEMES_URL = "https://github.com/mbadolato/iTerm2-Color-Schemes/tree/master/windowsterminal";
    function openSchemesUrl() {
        invoke("open_external_url", { url: SCHEMES_URL }).catch(e =>
            console.error("open_external_url failed:", e)
        );
    }

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

    // ─── AI panel position ────────────────────────────────────────────
    let aiPos = $state<ai.AiPosition>(ai.position());
    function pickAiPos(p: ai.AiPosition) {
        aiPos = p;
        ai.setPosition(p);
    }

    // ─── Theme: palette ──────────────────────────────────────────────
    const palettes = theme.listPalettes();
    let paletteId = $state<PaletteId>(theme.paletteId());
    async function pickPalette(id: PaletteId) {
        paletteId = id;
        await theme.setPalette(id);
    }

    // ─── Theme: shape (surface style) ────────────────────────────────
    const shapes = theme.listShapes();
    let shapeId = $state<theme.ShapeId>(theme.shapeId());
    async function pickShape(id: theme.ShapeId) {
        shapeId = id;
        await theme.setShape(id);
    }

    // ─── Theme: density ──────────────────────────────────────────────
    const densities = theme.listDensities();
    let densityId = $state<theme.DensityId>(theme.densityId());
    async function pickDensity(id: theme.DensityId) {
        densityId = id;
        await theme.setDensity(id);
    }

    // ─── Theme: terminal palette (xterm colors, independent of UI) ───
    const termPresets = theme.listTermPresets();
    let termRef = $state<TermPaletteRef>(theme.termPaletteRef());
    let termBgFollow = $state<boolean>(theme.termBgFollowsTheme());

    function isInherit(): boolean { return termRef.kind === "inherit"; }
    function isPreset(id: string): boolean {
        return termRef.kind === "preset" && termRef.id === id;
    }
    function isCustom(): boolean { return termRef.kind === "custom"; }

    async function saveTermBgFollow() {
        await theme.setTermBgFollowsTheme(termBgFollow);
    }

    async function pickInherit() {
        termRef = { kind: "inherit" };
        await theme.setTermPalette(termRef);
    }
    async function pickPreset(id: string) {
        termRef = { kind: "preset", id };
        await theme.setTermPalette(termRef);
    }

    // Custom import dialog
    let showCustomDialog = $state(false);
    let customJsonInput = $state("");
    let customError = $state("");

    function openCustomDialog() {
        // Pre-fill with current custom if any, else a minimal template.
        if (termRef.kind === "custom") {
            customJsonInput = JSON.stringify(termRef.term, null, 2);
        } else {
            customJsonInput = '{\n  "background": "#1e1e1e",\n  "foreground": "#d4d4d4"\n}';
        }
        customError = "";
        showCustomDialog = true;
    }
    async function importCustom() {
        try {
            const term = parseCustomTermJson(customJsonInput);
            termRef = { kind: "custom", term };
            await theme.setTermPalette(termRef);
            showCustomDialog = false;
        } catch (e: any) {
            customError = e.message || String(e);
        }
    }

    // ─── Theme: terminal font ────────────────────────────────────────
    // Fonts come from the system (Rust list_fonts); the chosen family is
    // prepended to the base stack. Search + the monospace filter live inside
    // FontSelect; here we just hold the list, the current choice, and persist.
    let fonts = $state<FontInfo[]>([]);
    let fontChoice = $state<string>(theme.termFont());
    async function pickFont(name: string) {
        fontChoice = name;
        await theme.setTermFont(name);
    }

    // Font size — px, clamped to the store's bounds. Mirrors the shell
    // page's numeric inputs: save on blur / Enter, snapping the box back
    // to the clamped value so an out-of-range entry can't linger.
    const fontSizeBounds = theme.termFontSizeBounds;
    let fontSize = $state<number>(theme.termFontSize());
    async function saveFontSize() {
        // 委托给 store 的 clampFontSize（唯一 clamp 真相）：0 夹到下限 8、空输入(NaN)
        // 落到默认。避免本地 `Math.round(x) || def` 把合法的 0 当假值错跳成默认值。
        await theme.setTermFontSize(fontSize);
        fontSize = theme.termFontSize();
    }
    onMount(async () => {
        try {
            fonts = await invoke<FontInfo[]>("list_fonts");
        } catch (e) {
            console.error("list_fonts failed:", e);
        }
    });
</script>

<div class="page">
    <div class="section-label">{t("settings.appearance.color_palette")}</div>
    <div class="layout-grid">
        {#each palettes as p}
            <button
                class="layout-card"
                class:active={paletteId === p.id}
                onclick={() => pickPalette(p.id)}
            >
                <div
                    class="palette-preview"
                    style="background: {p.ui.bg}; border-color: {p.ui.divider};"
                >
                    <div class="palette-row">
                        <span class="swatch" style="background: {p.ui.surface}"></span>
                        <span class="swatch" style="background: {p.ui.text}"></span>
                        <span class="swatch" style="background: {p.ui.accent}"></span>
                    </div>
                    <div class="palette-row">
                        <span class="swatch" style="background: {p.ui.success}"></span>
                        <span class="swatch" style="background: {p.ui.warning}"></span>
                        <span class="swatch" style="background: {p.ui.error}"></span>
                    </div>
                </div>
                <div class="layout-label">{p.label}</div>
            </button>
        {/each}
    </div>

    <div class="section-label">{t("settings.appearance.terminal_palette")}</div>
    <!-- Background-follow toggle + terminal font in one card — "main row +
         divider + second row", mirroring the shell page's Selection & Mouse. -->
    <div class="card surface-raised term-card">
        <div class="term-row">
            <div class="switch-card-body">
                <div class="switch-card-title"
                     class:on={termBgFollow} class:off={!termBgFollow}>
                    {t("settings.appearance.term.bg_follow")}
                </div>
                <div class="switch-card-desc">{t("settings.appearance.term.bg_follow_desc")}</div>
            </div>
            <label class="switch">
                <input type="checkbox" bind:checked={termBgFollow} onchange={saveTermBgFollow} />
                <span class="slider"></span>
            </label>
        </div>

        <div class="term-divider"></div>

        <div class="term-row">
            <div class="switch-card-body">
                <div class="switch-card-title">{t("settings.appearance.terminal_font")}</div>
                <div class="switch-card-desc">{t("settings.appearance.font.row_desc")}</div>
            </div>
            <div class="term-font-control">
                <FontSelect
                    bind:value={fontChoice}
                    fonts={fonts}
                    onchange={pickFont}
                    ariaLabel={t("settings.appearance.terminal_font")}
                />
            </div>
        </div>

        <div class="term-divider"></div>

        <div class="term-row">
            <div class="switch-card-body">
                <div class="switch-card-title">{t("settings.appearance.font.size")}</div>
                <div class="switch-card-desc">
                    {t("settings.appearance.font.size_desc", { min: fontSizeBounds.min, max: fontSizeBounds.max })}
                </div>
            </div>
            <input
                class="term-font-size-input"
                type="number"
                bind:value={fontSize}
                min={fontSizeBounds.min}
                max={fontSizeBounds.max}
                onblur={saveFontSize}
                onkeydown={(e) => { if (e.key === "Enter") saveFontSize(); }}
                aria-label={t("settings.appearance.font.size")}
            />
        </div>
    </div>
    <div class="layout-grid" style="--preview-font: {composeTermFontStack(fontChoice)};">
        <!-- Inherit: follow the UI palette -->
        <button
            class="layout-card"
            class:active={isInherit()}
            onclick={pickInherit}
        >
            <div class="term-preview term-inherit">
                <div class="term-inherit-label">↳ {t("settings.appearance.term.inherit")}</div>
            </div>
            <div class="layout-label">{t("settings.appearance.term.inherit")}</div>
        </button>

        <!-- Built-in presets — preview is a mini ls --color session.
             Preview background mirrors the runtime: when "bg follows theme"
             is on (default) the terminal uses the UI palette's --bg; when
             off the terminal keeps the scheme's own background. Anything
             else would lie about what the user sees after picking. -->
        {#each termPresets as p}
            <button
                class="layout-card"
                class:active={isPreset(p.id)}
                onclick={() => pickPreset(p.id)}
            >
                <div class="term-preview" style="background: {termBgFollow ? 'var(--bg)' : p.term.background};">
                    <div class="term-line">
                        <span style="color: {p.term.green};">~/code</span><span
                              style="color: {p.term.foreground};">$ </span><span
                              style="color: {p.term.foreground};">ls</span>
                    </div>
                    <div class="term-line">
                        <span style="color: {p.term.blue};">bin</span>
                        <span style="color: {p.term.green};">build.sh</span>
                        <span style="color: {p.term.foreground};">README</span>
                    </div>
                    <div class="term-line">
                        <span style="color: {p.term.yellow};">warn:</span>
                        <span style="color: {p.term.red};">error</span>
                    </div>
                </div>
                <div class="layout-label">{p.label}</div>
            </button>
        {/each}

        <!-- Custom: paste xterm.js JSON -->
        <button
            class="layout-card"
            class:active={isCustom()}
            onclick={openCustomDialog}
        >
            {#if isCustom() && termRef.kind === "custom"}
                <div class="term-preview" style="background: {termBgFollow ? 'var(--bg)' : termRef.term.background};">
                    <div class="term-line">
                        <span style="color: {termRef.term.green ?? termRef.term.foreground};">~/code</span><span
                              style="color: {termRef.term.foreground};">$ </span><span
                              style="color: {termRef.term.foreground};">ls</span>
                    </div>
                    <div class="term-line">
                        <span style="color: {termRef.term.blue ?? termRef.term.foreground};">bin</span>
                        <span style="color: {termRef.term.green ?? termRef.term.foreground};">build.sh</span>
                        <span style="color: {termRef.term.foreground};">README</span>
                    </div>
                    <div class="term-line">
                        <span style="color: {termRef.term.yellow ?? termRef.term.foreground};">warn:</span>
                        <span style="color: {termRef.term.red ?? termRef.term.foreground};">error</span>
                    </div>
                </div>
            {:else}
                <div class="term-preview term-custom">
                    <div class="term-custom-icon">+</div>
                    <div class="term-custom-label">{t("settings.appearance.term.custom")}</div>
                </div>
            {/if}
            <div class="layout-label">{t("settings.appearance.term.custom")}</div>
        </button>
    </div>

    <div class="section-label">{t("settings.appearance.surface_style")}</div>
    <div class="density-row">
        {#each densities as d}
            <button
                    class="density-btn"
                    class:active={densityId === d.id}
                    onclick={() => pickDensity(d.id)}
            >{t(`settings.appearance.density.${d.id}` as any)}</button>
        {/each}
    </div>
    <div class="layout-grid">
        {#each shapes as s}
            <button
                class="layout-card"
                class:active={shapeId === s.id}
                onclick={() => pickShape(s.id)}
            >
                <div class="shape-preview shape-{s.id}">
                    <div class="shape-card"></div>
                    <div class="shape-btn">Aa</div>
                </div>
                <div class="layout-label">{s.label}</div>
            </button>
        {/each}
    </div>



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
                <div class="layout-label">{t(side === "left" ? "settings.appearance.pos.left" : "settings.appearance.pos.right")}</div>
            </button>
        {/each}
    </div>

</div>

{#if showCustomDialog}
<div class="dialog-backdrop" onclick={() => showCustomDialog = false} role="presentation">
    <div class="dialog surface-raised" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
        <div class="dialog-title">{t("settings.appearance.term.import_title")}</div>
        <div class="dialog-hint">
            {t("settings.appearance.term.import_hint")}
            <button class="link-btn" type="button" onclick={openSchemesUrl}>
                {t("settings.appearance.term.import_link")}
            </button>
        </div>
        <textarea
            bind:value={customJsonInput}
            spellcheck="false"
            rows="14"
            class="dialog-textarea"
        ></textarea>
        {#if customError}
            <div class="dialog-error">{customError}</div>
        {/if}
        <div class="dialog-actions">
            <button class="btn" onclick={() => showCustomDialog = false}>
                {t("settings.appearance.term.cancel")}
            </button>
            <button class="btn btn-accent" onclick={importCustom}>
                {t("settings.appearance.term.import")}
            </button>
        </div>
    </div>
</div>
{/if}

<style>
    .page {
        padding: 24px;
        display: flex;
        flex-direction: column;
        gap: 12px;
        /* Required so the bottom sections stay reachable when the
           panel content is taller than the window — settings now has
           7 stacked sections (palette / term-palette / shape /
           density / sidebar pos / AI pos / terminal display). */
        flex: 1;
        overflow-y: auto;
        min-height: 0;
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

    /* Palette preview card — same footprint as .mini-window so the
       layout grid stays uniform across palette / position cards. */
    .palette-preview {
        width: 160px;
        height: 100px;
        border: 1px solid;
        border-radius: 6px;
        overflow: hidden;
        display: flex;
        flex-direction: column;
        justify-content: center;
        gap: 8px;
        padding: 12px;
    }
    .palette-row {
        display: flex;
        gap: 6px;
        justify-content: center;
    }
    .swatch {
        width: 24px;
        height: 24px;
        border-radius: 50%;
        box-shadow: 0 0 0 1px color-mix(in srgb, var(--text-dim) 30%, transparent);
    }

    /* Shape preview — each card renders a sample card + button using
       the shape's own style, NOT the active [data-shape], so users
       can compare all four side-by-side. Uniform footprint with the
       palette / position cards. */
    .shape-preview {
        width: 160px;
        height: 100px;
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 12px;
        background: var(--bg);
        border-radius: 6px;
        overflow: hidden;
    }
    .shape-card {
        width: 50px;
        height: 50px;
        background: var(--bg);
        border-radius: 8px;
    }
    .shape-btn {
        padding: 6px 12px;
        font-size: 12px;
        font-weight: 600;
        color: var(--white);
        background: var(--accent);
        border-radius: 8px;
    }

    /* Neumorphism: dual-shadow embossed surface */
    .shape-neumorphism .shape-card {
        box-shadow: 3px 3px 6px var(--shadow-dark), -3px -3px 6px var(--shadow-light);
    }
    .shape-neumorphism .shape-btn {
        box-shadow:
            2px 2px 5px color-mix(in srgb, var(--accent) 30%, transparent),
            -1px -1px 4px var(--shadow-light);
    }

    /* Flat: thin border, no shadow */
    .shape-flat .shape-card {
        border: 1px solid var(--divider);
    }
    .shape-flat .shape-btn {
        border: 1px solid var(--accent);
    }

    /* Material: single-direction drop shadow */
    .shape-material .shape-card {
        box-shadow:
            0 2px 4px color-mix(in srgb, var(--shadow-dark) 50%, transparent),
            0 4px 12px color-mix(in srgb, var(--shadow-dark) 30%, transparent);
        border-radius: 12px;
    }
    .shape-material .shape-btn {
        box-shadow:
            0 2px 4px color-mix(in srgb, var(--accent) 35%, transparent),
            0 4px 12px color-mix(in srgb, var(--accent) 25%, transparent);
        border-radius: 12px;
    }

    /* Density: simple segmented control. Three buttons, active one
       gets the accent color. */
    .density-row {
        display: flex;
        gap: 8px;
        flex-wrap: wrap;
    }
    .density-btn {
        padding: 8px 18px;
        border: 1px solid var(--divider);
        border-radius: var(--radius-sm);
        background: var(--bg);
        color: var(--text);
        font-family: inherit;
        font-size: 13px;
        font-weight: 500;
        cursor: pointer;
        transition: border-color 0.15s, background 0.15s, color 0.15s;
    }
    .density-btn:hover:not(.active) {
        background: var(--surface);
    }
    .density-btn.active {
        border-color: var(--accent);
        background: color-mix(in srgb, var(--accent) 12%, var(--bg));
        color: var(--accent);
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
    .mini-dot.red    { background: var(--error); }
    .mini-dot.yellow { background: var(--warning); }
    .mini-dot.green  { background: var(--success); }
    .mini-body {
        flex: 1;
        display: flex;
    }
    /* Direction modifiers — shared by AI panel + menu sidebar */
    .mini-body.dir-left   { flex-direction: row;            }
    .mini-body.dir-right  { flex-direction: row-reverse;    }
    .mini-body.dir-top    { flex-direction: column;         }
    .mini-body.dir-bottom { flex-direction: column-reverse; }

    /* AI panel (purple) — 只支持 left/right */
    .mini-ai {
        width: 38%;
        background: color-mix(in srgb, var(--purple) 22%, var(--surface));
        border-right: 1px solid color-mix(in srgb, var(--purple) 35%, transparent);
        display: flex;
        flex-direction: column;
        justify-content: center;
        gap: 4px;
        padding: 0 6px;
    }
    .mini-body.dir-right .mini-ai {
        border-right: none;
        border-left: 1px solid color-mix(in srgb, var(--purple) 35%, transparent);
    }
    .mini-ai-line {
        height: 3px;
        border-radius: 2px;
        background: color-mix(in srgb, var(--purple) 60%, transparent);
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

    /* ── Terminal palette preview cards ── */
    /* Mini terminal session: prompt + ls + warn/error lines, each
       span colored by the actual ANSI role (blue=dir, green=exec,
       yellow=warn, red=error). User sees colors in context. */
    .term-preview {
        width: 160px;
        height: 100px;
        border: 1px solid var(--divider);
        border-radius: 6px;
        overflow: hidden;
        display: flex;
        flex-direction: column;
        justify-content: center;
        gap: 4px;
        padding: 10px 12px;
        font-family: var(--preview-font, 'JetBrainsMono Nerd Font', Menlo, Monaco, monospace);
        font-size: 11px;
        line-height: 1.3;
    }
    .term-line {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        white-space: nowrap;
    }
    /* Inherit card — neutral, with arrow-style label */
    .term-inherit {
        background: var(--surface);
        align-items: center;
        justify-content: center;
    }
    .term-inherit-label {
        font-size: 14px;
        font-weight: 600;
        font-family: monospace;
        color: var(--text-sub);
    }
    /* Custom card — empty placeholder when no custom set */
    .term-custom {
        background: var(--surface);
        color: var(--text-dim);
        border-style: dashed;
        align-items: center;
        justify-content: center;
        gap: 6px;
    }
    .term-custom-icon {
        font-size: 28px;
        font-weight: 300;
        line-height: 1;
        color: var(--text-sub);
    }
    .term-custom-label {
        font-size: 12px;
        font-weight: 500;
    }

    /* ── Custom JSON import dialog ── */
    .dialog-backdrop {
        position: fixed;
        inset: 0;
        z-index: 500;
        background: var(--overlay-strong);
        display: flex;
        align-items: center;
        justify-content: center;
        padding: 20px;
    }
    .dialog {
        width: min(560px, 100%);
        max-height: 90vh;
        display: flex;
        flex-direction: column;
        gap: 12px;
        padding: 24px;
        overflow-y: auto;
    }
    .dialog-title {
        font-size: 15px;
        font-weight: 600;
    }
    .dialog-hint {
        font-size: 12px;
        color: var(--text-sub);
        line-height: 1.5;
    }
    .dialog-hint .link-btn {
        background: none;
        border: none;
        padding: 0;
        margin: 0;
        font: inherit;
        color: var(--accent);
        cursor: pointer;
        text-decoration: none;
    }
    .dialog-hint .link-btn:hover {
        text-decoration: underline;
    }
    .dialog-textarea {
        width: 100%;
        font-family: monospace;
        font-size: 12px;
        line-height: 1.5;
        resize: vertical;
        min-height: 200px;
    }
    .dialog-error {
        font-size: 12px;
        color: var(--error);
        padding: 8px 12px;
        background: color-mix(in srgb, var(--error) 12%, transparent);
        border-radius: var(--radius-sm);
        font-family: monospace;
    }
    .dialog-actions {
        display: flex;
        justify-content: flex-end;
        gap: 8px;
        margin-top: 4px;
    }

    /* ── Terminal palette + font card (Selection & Mouse pattern) ── */
    .term-card {
        padding: 18px;
        display: flex;
        flex-direction: column;
        gap: 14px;
    }
    .term-row {
        display: flex;
        align-items: center;
        gap: 16px;
        flex-wrap: wrap;
    }
    .term-divider {
        height: 1px;
        background: var(--divider);
    }
    /* Font picker sits on the right of its row, like the shell page's
       right-click Select (.rca-select). */
    .term-font-control {
        width: 260px;
        max-width: 100%;
        flex-shrink: 0;
    }
    .term-font-size-input {
        width: 72px;
        flex-shrink: 0;
        text-align: center;
    }
</style>
