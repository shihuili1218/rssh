<script lang="ts">
    import * as transfers from "../stores/transfers.svelte.ts";
    import * as app from "../stores/app.svelte.ts";
    import { t } from "../i18n/index.svelte.ts";

    let popEl: HTMLDivElement | undefined;
    let list = $derived(transfers.list());
    /** Anchor the popover to the same edge the sidebar's Downloads entry
     *  lives on, so it pops out from where the user clicked.
     *  - sb-left:   entry at bottom-left  → popover bottom-left
     *  - sb-right:  entry at bottom-right → popover bottom-right
     *  - sb-top:    entry at top bar right → popover top-right
     *  - sb-bottom: entry at bottom bar right → popover bottom-right above bar */
    const sbPos = $derived(app.sidebarPosition());
    // "Finished" excludes queued/running — both are pending work that the
    // Clear button must not sweep away.
    let hasFinished = $derived(list.some(x =>
        x.status === "done" || x.status === "failed" || x.status === "cancelled",
    ));

    function formatSize(bytes: number): string {
        if (bytes < 1024) return `${bytes} B`;
        if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} K`;
        if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1)} M`;
        return `${(bytes / 1073741824).toFixed(1)} G`;
    }

    function pct(t: transfers.Transfer): number {
        if (t.total <= 0) return 0;
        return Math.min(100, (t.transferred / t.total) * 100);
    }

    function basename(p: string): string {
        return p.split(/[\\/]/).pop() || p;
    }

    /** Trigger elements (sidebar entry, SFTP toolbar icon, etc.) are marked
     *  with `data-transfers-trigger` so click-outside ignores them. This
     *  avoids the "click trigger → close popover → click reopens it" loop. */
    function isTrigger(node: Node | null): boolean {
        let el = node instanceof Element ? node : (node?.parentElement ?? null);
        return !!el?.closest("[data-transfers-trigger]");
    }

    function onWindowMouseDown(ev: MouseEvent) {
        const target = ev.target as Node | null;
        if (!target) return;
        if (popEl?.contains(target)) return;
        if (isTrigger(target)) return;       // let the trigger's own click toggle
        app.closeDownloads();
    }

    function onWindowKeyDown(ev: KeyboardEvent) {
        if (ev.key === "Escape") app.closeDownloads();
    }

    $effect(() => {
        window.addEventListener("mousedown", onWindowMouseDown);
        window.addEventListener("keydown", onWindowKeyDown);
        return () => {
            window.removeEventListener("mousedown", onWindowMouseDown);
            window.removeEventListener("keydown", onWindowKeyDown);
        };
    });
</script>

<div class="downloads {`sb-${sbPos}`}" bind:this={popEl} role="dialog" aria-label={t("downloads.title")}>
    <header>
        <h2>{t("downloads.title")}</h2>
        <div class="header-actions">
            {#if hasFinished}
                <button class="btn btn-sm" onclick={() => transfers.clearFinished()}>
                    {t("downloads.clear_finished")}
                </button>
            {/if}
            <button type="button" class="btn-icon" onclick={() => app.closeDownloads()} aria-label={t("common.close")} title={t("common.close")}>×</button>
        </div>
    </header>

    {#if list.length === 0}
        <p class="empty">{t("downloads.empty")}</p>
    {:else}
        <ul class="list">
            {#each list as item (item.id)}
                {@const showError = !!(item.error && item.status === "failed")}
                <li class="row surface-pressed"
                    class:failed={item.status === "failed"}
                    class:done={item.status === "done"}
                    class:cancelled={item.status === "cancelled"}
                    class:has-error={showError}>
                    <span class="kind" title={item.kind}>{item.kind === "download" ? "↓" : "↑"}</span>

                    <div class="name" title={item.kind === "download" ? item.remotePath : item.localPath}>
                        {basename(item.kind === "download" ? item.remotePath : item.localPath)}
                    </div>
                    <div class="path" title={item.kind === "download" ? item.localPath : item.remotePath}>
                        → {item.kind === "download" ? item.localPath : item.remotePath}
                    </div>

                    <div class="track">
                        <div class="fill"
                             class:fill-fail={item.status === "failed"}
                             class:fill-cancel={item.status === "cancelled"}
                             style="width: {pct(item).toFixed(1)}%"></div>
                    </div>

                    <div class="bytes">
                        {formatSize(item.transferred)}{item.total > 0 ? ` / ${formatSize(item.total)}` : ""}
                    </div>

                    <span class="status status-{item.status}">{t(`downloads.status.${item.status}`)}</span>

                    <div class="row-actions">
                        {#if item.status === "running" || item.status === "queued"}
                            <button class="btn btn-sm btn-ghost" onclick={() => transfers.cancel(item.id)}>
                                {t("downloads.cancel")}
                            </button>
                        {/if}
                        {#if item.status === "failed" || item.status === "cancelled"}
                            <button class="btn btn-sm" onclick={() => transfers.retry(item.id)}>
                                {t("downloads.retry")}
                            </button>
                        {/if}
                        {#if item.status === "done" || item.status === "failed" || item.status === "cancelled"}
                            <button class="btn btn-sm btn-ghost" onclick={() => transfers.remove(item.id)}>
                                {t("downloads.remove")}
                            </button>
                        {/if}
                    </div>

                    {#if showError}
                        <div class="error">{item.error}</div>
                    {/if}
                </li>
            {/each}
        </ul>
    {/if}
</div>

<style>
    /* Popover: max-height 70vh with an independently scrolling list; max-width
       460px keeps wide screens from being swallowed. Concrete edge offsets are
       set by the .sb-* variants below to mirror the sidebar's anchor edge. */
    .downloads {
        position: fixed;
        width: min(460px, calc(100vw - 24px));
        max-height: 70vh;
        background: var(--bg);
        border: 1px solid var(--divider);
        border-radius: var(--radius-md, 8px);
        box-shadow: 0 10px 36px rgba(0, 0, 0, 0.28), 0 2px 8px rgba(0, 0, 0, 0.18);
        padding: 14px 16px 12px;
        display: flex;
        flex-direction: column;
        z-index: 1000;
        box-sizing: border-box;
    }

    /* Edge offsets follow the --sb-* layout vars set by AppShell on .shell.
       This keeps popover anchoring in lock-step with the sidebar thickness —
       if the sidebar resize ever changes, the popover follows automatically.
       The extra 8px gap separates the popover from the sidebar visually. */
    .downloads.sb-left {
        left:   calc(var(--sb-left, 0px) + 8px);
        bottom: 12px;
    }
    .downloads.sb-right {
        right:  calc(var(--sb-right, 0px) + 8px);
        bottom: 12px;
    }
    .downloads.sb-top {
        top:   calc(var(--sb-top, 0px) + 8px);
        right: 12px;
    }
    .downloads.sb-bottom {
        bottom: calc(var(--sb-bottom, 0px) + 8px);
        right:  12px;
    }

    header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 10px;
        flex-shrink: 0;
    }

    .header-actions {
        display: flex;
        align-items: center;
        gap: 6px;
    }

    .btn-icon {
        width: 24px;
        height: 24px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        border: none;
        background: transparent;
        color: var(--text-sub);
        font-size: 18px;
        line-height: 1;
        border-radius: var(--radius-sm);
        cursor: pointer;
    }
    .btn-icon:hover { color: var(--text); background: var(--accent-soft); }

    h2 {
        margin: 0;
        font-size: 14px;
        font-weight: 600;
        color: var(--text);
        letter-spacing: 0.2px;
    }

    .empty {
        text-align: center;
        color: var(--text-dim);
        padding: 48px 24px;
        font-size: 13px;
    }

    .list {
        list-style: none;
        padding: 0;
        margin: 0;
        display: flex;
        flex-direction: column;
        gap: 8px;
        /* Popover height is bounded, so the list area scrolls independently. */
        flex: 1 1 auto;
        overflow-y: auto;
        min-height: 0;
    }

    /* The popover is always narrow; each row uses a stacked layout:
         line 1: icon | name           | status
         line 2: icon | path           | bytes
         line 3:      | progress (span)
         line 4:      | actions  (span)
       The error row (failed only) sits between progress and actions. */
    .row {
        background: var(--bg);
        box-shadow: var(--pressed);
        border-radius: var(--radius-sm);
        padding: calc(10px * var(--density)) calc(14px * var(--density));
        display: grid;
        grid-template-columns: 28px 1fr auto;
        grid-template-areas:
            "icon name     status"
            "icon path     bytes"
            ".    progress progress"
            ".    actions  actions";
        column-gap: 12px;
        row-gap: 4px;
        align-items: center;
    }

    .row.has-error {
        grid-template-areas:
            "icon name     status"
            "icon path     bytes"
            ".    progress progress"
            ".    error    error"
            ".    actions  actions";
    }

    .kind {
        grid-area: icon;
        width: 28px;
        height: 28px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        background: var(--surface);
        border-radius: 6px;
        font-size: 14px;
        font-weight: 700;
        color: var(--accent);
        align-self: center;
    }

    .name {
        grid-area: name;
        font-size: 13px;
        font-weight: 600;
        color: var(--text);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        min-width: 0;
    }

    .path {
        grid-area: path;
        font-size: 11px;
        color: var(--text-dim);
        font-family: monospace;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        min-width: 0;
    }

    .track {
        grid-area: progress;
        height: 6px;
        background: var(--surface);
        border-radius: 3px;
        overflow: hidden;
        min-width: 0;
    }

    .fill {
        height: 100%;
        background: var(--accent);
        border-radius: 3px;
        transition: width 0.15s linear;
    }

    .fill-fail   { background: var(--error); }
    .fill-cancel { background: var(--text-dim); }

    .bytes {
        grid-area: bytes;
        font-size: 11px;
        color: var(--text-sub);
        white-space: nowrap;
        font-variant-numeric: tabular-nums;
        text-align: right;
    }

    .status {
        grid-area: status;
        font-size: 11px;
        padding: 3px 10px;
        border-radius: 10px;
        white-space: nowrap;
        text-align: center;
    }

    .status-running   { background: color-mix(in srgb, var(--success) 15%, transparent); color: var(--success); }
    .status-queued    { background: color-mix(in srgb, var(--text-sub) 15%, transparent); color: var(--text-sub); }
    .status-done      { background: var(--surface); color: var(--text-dim); }
    .status-failed    { background: color-mix(in srgb, var(--error) 15%, transparent); color: var(--error); }
    .status-cancelled { background: var(--surface); color: var(--text-sub); }

    .row-actions {
        grid-area: actions;
        display: flex;
        gap: 6px;
        justify-content: flex-end;
        flex-wrap: nowrap;
    }

    .error {
        grid-area: error;
        font-size: 11px;
        color: var(--error);
        word-break: break-all;
        font-family: monospace;
        padding-top: 4px;
    }

    .btn-ghost {
        background: transparent;
        color: var(--text-dim);
        box-shadow: none;
    }
    .btn-ghost:hover { color: var(--text); }
</style>
