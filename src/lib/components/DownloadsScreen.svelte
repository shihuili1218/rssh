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
                <li class="row" class:failed={item.status === "failed"} class:done={item.status === "done"}>
                    <div class="row-head">
                        <span class="kind" title={item.kind}>{item.kind === "download" ? "↓" : "↑"}</span>
                        <span class="name" title={item.kind === "download" ? item.remotePath : item.localPath}>
                            {basename(item.kind === "download" ? item.remotePath : item.localPath)}
                        </span>
                        <span class="status status-{item.status}">{t(`downloads.status.${item.status}`)}</span>
                    </div>

                    <div class="row-meta">
                        <span class="path" title={item.kind === "download" ? item.localPath : item.remotePath}>
                            → {item.kind === "download" ? item.localPath : item.remotePath}
                        </span>
                    </div>

                    <div class="row-bar">
                        <div class="track">
                            <div class="fill" class:fill-fail={item.status === "failed"} style="width: {pct(item).toFixed(1)}%"></div>
                        </div>
                        <span class="bytes">
                            {formatSize(item.transferred)}{item.total > 0 ? ` / ${formatSize(item.total)}` : ""}
                        </span>
                    </div>

                    {#if item.error}
                        <div class="error">{item.error}</div>
                    {/if}

                    <div class="row-actions">
                        {#if item.status === "failed"}
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
                </li>
            {/each}
        </ul>
    {/if}
</div>

<style>
    .downloads {
        padding: 20px;
        max-width: 760px;
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

    .row {
        background: var(--bg);
        box-shadow: var(--pressed);
        border-radius: var(--radius-sm);
        padding: 10px 12px;
        display: flex;
        flex-direction: column;
        gap: 6px;
    }

    .row-head {
        display: flex;
        align-items: center;
        gap: 8px;
    }

    .kind {
        width: 18px;
        height: 18px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        background: var(--surface);
        border-radius: 4px;
        font-size: 12px;
        font-weight: 700;
        color: var(--accent);
        flex-shrink: 0;
    }

    .name {
        flex: 1;
        font-size: 13px;
        font-weight: 600;
        color: var(--text);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .status {
        font-size: 11px;
        padding: 2px 8px;
        border-radius: 10px;
        flex-shrink: 0;
    }

    .status-running { background: rgba(76, 184, 138, 0.15); color: #4cb88a; }
    .status-done    { background: var(--surface); color: var(--text-dim); }
    .status-failed  { background: rgba(214, 68, 68, 0.15); color: var(--error); }

    .row-meta {
        font-size: 11px;
        color: var(--text-dim);
        font-family: monospace;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .row-bar {
        display: flex;
        align-items: center;
        gap: 10px;
    }

    .track {
        flex: 1;
        height: 6px;
        background: var(--surface);
        border-radius: 3px;
        overflow: hidden;
    }

    .fill {
        height: 100%;
        background: var(--accent);
        border-radius: 3px;
        transition: width 0.15s linear;
    }

    .fill-fail { background: var(--error); }

    .bytes {
        font-size: 11px;
        color: var(--text-sub);
        white-space: nowrap;
        font-variant-numeric: tabular-nums;
    }

    .error {
        font-size: 11px;
        color: var(--error);
        word-break: break-all;
        font-family: monospace;
    }

    .row-actions {
        display: flex;
        gap: 6px;
        justify-content: flex-end;
    }

    .btn-ghost {
        background: transparent;
        color: var(--text-dim);
        box-shadow: none;
    }

    .btn-ghost:hover { color: var(--text); }
</style>
