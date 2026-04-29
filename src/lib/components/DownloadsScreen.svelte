<script lang="ts">
    import * as transfers from "../stores/transfers.svelte.ts";
    import { t } from "../i18n/index.svelte.ts";

    let list = $derived(transfers.list());
    let hasFinished = $derived(list.some(x => x.status !== "running"));

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
</script>

<div class="downloads">
    <header>
        <h2>{t("downloads.title")}</h2>
        {#if hasFinished}
            <button class="btn btn-sm" onclick={() => transfers.clearFinished()}>
                {t("downloads.clear_finished")}
            </button>
        {/if}
    </header>

    {#if list.length === 0}
        <p class="empty">{t("downloads.empty")}</p>
    {:else}
        <ul class="list">
            {#each list as item (item.id)}
                {@const showError = !!(item.error && item.status === "failed")}
                <li class="row"
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
                        {#if item.status === "running"}
                            <button class="btn btn-sm btn-ghost" onclick={() => transfers.cancel(item.id)}>
                                {t("downloads.cancel")}
                            </button>
                        {/if}
                        {#if item.status === "failed" || item.status === "cancelled"}
                            <button class="btn btn-sm" onclick={() => transfers.retry(item.id)}>
                                {t("downloads.retry")}
                            </button>
                        {/if}
                        {#if item.status !== "running"}
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
    .downloads {
        padding: 20px 24px;
        max-width: 1400px;
        margin: 0 auto;
        height: 100%;
        overflow-y: auto;
    }

    header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: 16px;
    }

    h2 {
        margin: 0;
        font-size: 16px;
        font-weight: 600;
        color: var(--text);
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
    }

    /* ── 桌面端：用 grid 把每条传输铺成一横排两行 ──
       Col 1: icon
       Col 2: name + path（两行同列，name 在上 path 在下）
       Col 3: progress bar（跨两行垂直居中）
       Col 4: bytes（跨两行）
       Col 5: status badge（跨两行）
       Col 6: actions（跨两行）

       error 行（仅 failed）独立第三行跨满。 */
    .row {
        background: var(--bg);
        box-shadow: var(--pressed);
        border-radius: var(--radius-sm);
        padding: 10px 14px;
        display: grid;
        grid-template-columns:
            28px              /* icon */
            minmax(160px, 1.4fr)  /* name / path */
            minmax(180px, 2.4fr)  /* progress */
            max-content       /* bytes */
            max-content       /* status */
            max-content;      /* actions */
        grid-template-areas:
            "icon name progress bytes status actions"
            "icon path progress bytes status actions";
        column-gap: 14px;
        row-gap: 2px;
        align-items: center;
    }

    .row.has-error {
        grid-template-areas:
            "icon name     progress bytes status actions"
            "icon path     progress bytes status actions"
            ".    error    error    error error error";
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
        min-width: 110px;
    }

    .status {
        grid-area: status;
        font-size: 11px;
        padding: 3px 10px;
        border-radius: 10px;
        white-space: nowrap;
        text-align: center;
    }

    .status-running   { background: rgba(76, 184, 138, 0.15); color: #4cb88a; }
    .status-done      { background: var(--surface); color: var(--text-dim); }
    .status-failed    { background: rgba(214, 68, 68, 0.15); color: var(--error); }
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

    /* ── 窄屏：grid 撑不下了，回到堆叠 ── */
    @media (max-width: 720px) {
        .downloads { padding: 16px; }
        .row {
            grid-template-columns: 28px 1fr auto;
            grid-template-areas:
                "icon name     status"
                "icon path     bytes"
                ".    progress progress"
                ".    actions  actions";
        }
        .row.has-error {
            grid-template-areas:
                "icon name     status"
                "icon path     bytes"
                ".    progress progress"
                ".    error    error"
                ".    actions  actions";
        }
        .bytes { min-width: 0; }
        .row-actions { justify-content: flex-end; }
    }
</style>
