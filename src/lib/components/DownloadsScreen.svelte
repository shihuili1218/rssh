<script lang="ts">
    import * as transfers from "../stores/transfers.svelte.ts";
    import * as app from "../stores/app.svelte.ts";
    import { t } from "../i18n/index.svelte.ts";

    let popEl: HTMLDivElement | undefined;
    let list = $derived(transfers.list());
    /** 跟随 sidebar 位置反向贴边 —— Downloads 入口在哪个 footer，浮窗就从那边弹出。
     *  - sb-left:   入口在左下 → 浮窗左下
     *  - sb-right:  入口在右下 → 浮窗右下
     *  - sb-top:    入口在顶栏右 → 浮窗右上
     *  - sb-bottom: 入口在底栏右 → 浮窗右下贴底栏 */
    const sbPos = $derived(app.sidebarPosition());
    // "已结束" 不包括 queued/running —— 这俩是未完成工作，clear 按钮不应误清。
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

    /** 浮窗触发器（侧栏入口 + SFTP toolbar 图标）打上 data-transfers-trigger，
     *  让 click-outside 排除掉它们，避免"点 trigger 关浮窗 → click 又开浮窗"的回旋。 */
    function isTrigger(node: Node | null): boolean {
        let el = node instanceof Element ? node : (node?.parentElement ?? null);
        return !!el?.closest("[data-transfers-trigger]");
    }

    function onWindowMouseDown(ev: MouseEvent) {
        const target = ev.target as Node | null;
        if (!target) return;
        if (popEl?.contains(target)) return;
        if (isTrigger(target)) return;       // 让 trigger 自己 toggle
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
    /* Popover：max-height 70vh，列表区独立滚动；max-width 460px 让宽屏不至于占太多空间。
       具体的 top/right/bottom/left 由 .sb-* 子类决定，跟着 sidebar 反向贴边。 */
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

    /* sidebar 占左 40px → 浮窗左下贴边，给 sidebar 让出 8px 间距 */
    .downloads.sb-left {
        left: 48px;
        bottom: 12px;
    }
    /* sidebar 占右 40px → 浮窗右下 */
    .downloads.sb-right {
        right: 48px;
        bottom: 12px;
    }
    /* sidebar 顶栏 44px → 浮窗右上贴顶栏下边 */
    .downloads.sb-top {
        top: 52px;
        right: 12px;
    }
    /* sidebar 底栏 44px → 浮窗右下贴底栏上边 */
    .downloads.sb-bottom {
        bottom: 52px;
        right: 12px;
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
        /* 浮窗高度受限，列表区独立滚动 */
        flex: 1 1 auto;
        overflow-y: auto;
        min-height: 0;
    }

    /* Popover 永远窄，row 用堆叠布局：
         line 1: icon | name           | status
         line 2: icon | path           | bytes
         line 3:      | progress (span)
         line 4:      | actions  (span)
       error 行（仅 failed）插在 progress / actions 之间。 */
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
