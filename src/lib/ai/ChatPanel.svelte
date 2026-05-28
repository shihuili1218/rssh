<script lang="ts">
    import * as ai from "./store.svelte.ts";
    import type { ChatItem } from "./types.ts";
    import CommandConfirmDialog from "./CommandConfirmDialog.svelte";
    import AuditPanel from "./AuditPanel.svelte";
    import { renderMarkdown } from "./markdown.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import { onMount } from "svelte";

    // tabId 是 AI 会话身份（actor 跟 tab 同寿命，重连不丢）。
    // targetId 是当前 SSH/PTY session_id —— 给 executeCommand 路由 ssh_write/pty_write 用。
    // 重连后 targetId 会换（前端 prop 自动跟随），tabId 不变。
    let { tabId, targetKind, targetId } = $props<{
        tabId: string;
        targetKind: "ssh" | "local";
        targetId: string;
    }>();

    let inputText = $state("");
    let auditOpen = $state(false);
    let busy = $state(false);
    let banner = $state<string | null>(null);
    let inputEl = $state<HTMLTextAreaElement | null>(null);
    let chatBoxEl = $state<HTMLDivElement | null>(null);
    let showClearDialog = $state(false);

    let session = $derived(ai.sessionForTab(tabId));
    let items: ChatItem[] = $derived(ai.chatItems(tabId));
    // 流式响应进行中 —— send 按钮换成"停止"按钮。依赖 items 变化重算（last item 的 streaming flag）。
    let streaming = $derived(ai.isStreaming(tabId));
    // 危险模式标记 —— 用户在 AI Settings 里切换后，标题旁的红色后缀立刻同步。
    // 走 ai.settings() 读 store 的 $state，自动响应式（不需要手动 loadSettings 触发）。
    let dangerMode = $derived(ai.settings()?.danger_mode === true);

    onMount(() => {
        if (!ai.settings()) ai.loadSettings().catch(() => {});
    });

    $effect(() => {
        items.length;
        if (chatBoxEl) {
            queueMicrotask(() => { chatBoxEl!.scrollTop = chatBoxEl!.scrollHeight; });
        }
    });

    /** 没 session 就先启动；启动失败抛错。
     *  skill 固定 general —— 用户自定义 skill 已自动拼进 master prompt，让 LLM 自己路由。 */
    async function ensureSession(): Promise<void> {
        if (session) return;
        const settings = ai.settings() ?? await ai.loadSettings();
        if (!settings.has_api_key) {
            throw new Error(t("ai.error.no_api_key"));
        }
        await ai.startSession({
            tabId, targetKind, targetId, skill: "general",
            provider: settings.provider, model: settings.model,
        });
    }

    async function send() {
        const text = inputText.trim();
        if (!text || busy) return;
        banner = null;
        busy = true;
        try {
            await ensureSession();
            inputText = "";
            await ai.sendMessage(tabId, text);
        } catch (e: any) {
            console.error("[ai] send failed:", e);
            banner = errMsg(e);
        } finally {
            busy = false;
        }
    }

    /** 关面板 = 仅隐藏 UI。actor 跟 tab 同寿命，下次开面板上下文还在。
     *  真正销毁 actor 在 app.closeTab() 里挂钩。 */
    function closePanel() {
        ai.closePanel();
    }

    /** 点扫帚按钮：开二次确认模态。actor 不在就不弹（清个空气没意义）。 */
    function openClearDialog() {
        if (!session) return;
        showClearDialog = true;
    }

    /** 用户在模态里点"清空"：actor 不死，只把 history 清空 —— 下条消息从头来过。
     *  若正在流式响应，先把流停掉，避免 in-flight delta 落到已清空的气泡数组。 */
    async function clearContext() {
        showClearDialog = false;
        if (!session) return;
        try {
            if (streaming) {
                await ai.cancelStream(tabId);
            }
            await ai.clearContext(tabId);
        } catch (e) {
            console.error("[ai] clear context:", e);
            banner = errMsg(e);
        }
    }

    /** 打断当前流式响应；会话上下文保留，用户可立刻发下一条纠正。 */
    async function stopStreaming() {
        if (!session) return;
        try {
            await ai.cancelStream(tabId);
        } catch (e) {
            // 不能只 console.error 就完事——失败的话用户还卡在 streaming/disabled 状态，
            // 看不到任何错误反馈。复用 banner 让用户知道"停止没生效，再点一次或刷新"。
            console.error("[ai] cancel stream:", e);
            banner = errMsg(e);
        }
    }

    function onKeyDown(e: KeyboardEvent) {
        if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            send();
        }
    }

    function fmt(ts: number) {
        return new Date(ts).toLocaleTimeString();
    }
</script>

<div class="ai-panel">
    <div class="toolbar">
        <span class="title">{t("ai.title")}</span>
        {#if dangerMode}
            <span class="title-danger" title={t("ai.title.danger_tip")}>{t("ai.title.danger_suffix")}</span>
        {/if}
        {#if session}
            <button class="btn btn-ghost btn-sm audit-toggle" onclick={() => (auditOpen = !auditOpen)}>
                {auditOpen ? t("ai.toolbar.back_to_chat") : t("ai.toolbar.audit")}
            </button>
        {/if}
        <span class="grow"></span>
        {#if session}
            <!-- 清理上下文：仅会话存在时露出。SVG 扫帚图标（22×22）跟"×"视觉重心对齐。 -->
            <button class="btn-icon" onclick={openClearDialog} title={t("ai.toolbar.clear_context")} aria-label={t("ai.toolbar.clear_context")}>
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M19.36 2.72l1.42 1.42-5.72 5.71-1.42-1.42 5.72-5.71z"/>
                    <path d="M14.13 8.05l-6.36 6.36c-.78.78-2.05.78-2.83 0l-.71-.71c-.39-.39-.39-1.02 0-1.41l7.07-7.07c.39-.39 1.02-.39 1.41 0l.71.71c.78.78.78 2.04 0 2.82"/>
                    <path d="M12 14l-3 7"/>
                    <path d="M9 14l-1.5 7"/>
                    <path d="M6 14l0 7"/>
                </svg>
            </button>
        {/if}
        <button class="btn-icon" onclick={closePanel} title={t("ai.toolbar.close_panel")} aria-label={t("ai.toolbar.close_panel")}>×</button>
    </div>

    {#if banner}
        <div class="banner">
            <span>{banner}</span>
            <button class="btn-icon" onclick={() => (banner = null)}>×</button>
        </div>
    {/if}

    {#if auditOpen && session}
        <AuditPanel {tabId} />
    {:else}
        <div class="chat" bind:this={chatBoxEl}>
            {#each items as item, i (i)}
                <div class="item item-{item.kind}">
                    {#if item.kind === "user"}
                        <div class="ts">{fmt(item.at)}</div>
                        <div class="bubble user">{item.text}</div>
                    {:else if item.kind === "assistant"}
                        <div class="ts">{fmt(item.at)}</div>
                        <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                        <div class="bubble assistant md" class:streaming={item.streaming} class:cancelled={item.cancelled}>
                            {#if item.text}
                                {@html renderMarkdown(item.text)}
                            {:else if !item.cancelled}
                                …
                            {/if}
                            {#if item.cancelled}
                                <span class="cancelled-tag">{t("ai.bubble.cancelled")}</span>
                            {/if}
                        </div>
                    {:else if item.kind === "command" && session}
                        <CommandConfirmDialog
                            {tabId}
                            targetKind={targetKind}
                            targetSessionId={targetId}
                            cmd={item.cmd}
                            result={item.result}
                            rejected={item.rejected}
                        />
                    {:else if item.kind === "error"}
                        <div class="bubble error">{item.text}</div>
                    {:else if item.kind === "note"}
                        <div class="bubble note">{item.text}</div>
                    {/if}
                </div>
            {/each}
            {#if items.length === 0 && !session}
                <div class="placeholder dim">
                    <p>{t("ai.placeholder.welcome")}</p>
                    <p class="hint">{t("ai.placeholder.example_hint")}</p>
                    <p class="hint">{t("ai.placeholder.confirm_hint")}</p>
                </div>
            {/if}
        </div>

        <div class="input-area">
            <textarea
                bind:this={inputEl}
                bind:value={inputText}
                placeholder={busy ? (session ? t("ai.input.replying") : t("ai.input.starting")) : (streaming ? t("ai.input.replying") : t("ai.input.placeholder"))}
                onkeydown={onKeyDown}
                disabled={busy}
                readonly={streaming}
            ></textarea>
            {#if streaming}
                <button class="btn btn-stop" onclick={stopStreaming} title={t("ai.input.stop")}>
                    {t("ai.input.stop")}
                </button>
            {:else}
                <button class="btn btn-primary" onclick={send} disabled={!inputText.trim() || busy}>
                    {busy && !session ? t("ai.input.starting_short") : t("ai.input.send")}
                </button>
            {/if}
        </div>
    {/if}
</div>

<!-- Clear-context confirmation. Tauri webview drops native confirm() silently,
     so we use the same custom modal pattern as AiSettings' danger-mode dialog. -->
{#if showClearDialog}
    <div class="dialog-backdrop" onclick={() => (showClearDialog = false)} role="presentation">
        <div class="dialog surface-raised" onclick={(e) => e.stopPropagation()}
             role="dialog" aria-modal="true"
             aria-labelledby="clear-dialog-title"
             aria-describedby="clear-dialog-body">
            <h3 id="clear-dialog-title" class="dialog-title">{t("ai.toolbar.clear_confirm_title")}</h3>
            <div id="clear-dialog-body" class="dialog-body">{t("ai.toolbar.clear_confirm")}</div>
            <div class="btn-row">
                <button class="btn btn-sm" onclick={() => (showClearDialog = false)}>
                    {t("common.cancel")}
                </button>
                <button class="btn btn-sm btn-primary" onclick={clearContext}>
                    {t("ai.toolbar.clear_confirm_action")}
                </button>
            </div>
        </div>
    </div>
{/if}

<style>
    .ai-panel {
        display: flex;
        flex-direction: column;
        height: 100%;
        background: var(--bg);
        border-left: 1px solid var(--divider);
        border-right: 1px solid var(--divider);
    }
    .toolbar {
        display: flex; align-items: center; gap: 8px;
        padding: 8px; border-bottom: 1px solid var(--divider);
        flex-shrink: 0;
    }
    .title { font-weight: 600; font-size: 13px; }
    .title-danger {
        font-size: 11px;
        font-weight: 600;
        color: var(--error);
        padding: 1px 6px;
        border: 1px solid var(--error);
        border-radius: 3px;
        background: color-mix(in srgb, var(--error) 8%, transparent);
    }
    .grow { flex: 1; }
    /* 审计/对话同一颗按钮，label 在两种语言下宽度不一（"✎𓂃审计" vs "← 对话"，
       "✎𓂃Audit" vs "← Chat"）。固定宽高让它在 toggle 时不抖动，也跟工具栏其他
       元素的视觉重心稳定一致。padding 归零交给 .btn 的 flex 居中处理。 */
    .audit-toggle {
        width: calc(88px * var(--density));
        height: calc(30px * var(--density));
        padding: 0;
        flex-shrink: 0;
    }
    .btn-primary { background: var(--accent); color: var(--white); border-color: var(--accent); }
    .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
    .btn-stop {
        background: var(--error);
        color: var(--white);
        border-color: var(--error);
        cursor: pointer;
    }
    .btn-stop:hover { opacity: 0.85; }
    .btn-ghost { background: transparent; }
    .btn-icon {
        background: transparent; border: none;
        font-size: 18px; cursor: pointer;
        color: var(--text); padding: 4px 6px;
        display: inline-flex; align-items: center; justify-content: center;
        line-height: 1;
        border-radius: 4px;
    }
    .btn-icon:hover {
        background: color-mix(in srgb, var(--text) 8%, transparent);
        color: var(--text);
    }
    .banner {
        display: flex; align-items: center; gap: 8px;
        padding: 8px 12px;
        background: color-mix(in srgb, var(--error) 18%, var(--bg));
        color: var(--error);
        border-bottom: 1px solid var(--divider);
        font-size: 12px;
        flex-shrink: 0;
    }
    .banner span { flex: 1; word-break: break-word; }

    .placeholder {
        padding: 24px; text-align: center;
        color: var(--text-dim);
        line-height: 1.6;
    }
    .placeholder.dim { font-size: 13px; padding: 32px; }
    .hint { font-size: 12px; }

    .chat {
        flex: 1; overflow-y: auto; padding: 6px;
        display: flex; flex-direction: column; gap: 3px;
    }
    .item { display: flex; flex-direction: column; gap: 1px; }
    .ts {
        font-size: 10px; color: var(--text-dim);
        font-family: monospace;
    }
    .bubble {
        padding: 5px 9px; border-radius: 6px;
        max-width: 95%; word-break: break-word; white-space: pre-wrap;
        font-size: 13px;
    }
    .bubble.user {
        background: var(--accent); color: var(--white);
        align-self: flex-end;
    }
    .bubble.assistant {
        background: color-mix(in srgb, var(--text) 8%, var(--bg));
        align-self: flex-start;
    }
    .bubble.assistant.streaming {
        position: relative;
    }
    /* 用户打断的响应：气泡尾部跟一个本地化小徽章，区别于"AI 自己结束的对话"。
       徽章本身在 ChatPanel 模板里用 i18n 渲染，避免把英文 marker 硬塞进 LLM 输出文本。 */
    .cancelled-tag {
        display: inline-block;
        margin-left: 6px;
        padding: 1px 6px;
        border-radius: 3px;
        background: color-mix(in srgb, var(--text-dim) 18%, transparent);
        color: var(--text-dim);
        font-size: 10.5px;
        font-weight: 500;
        vertical-align: middle;
    }
    .bubble.assistant.streaming::after {
        content: "▋";
        display: inline-block;
        margin-left: 2px;
        animation: blink 1s steps(2, start) infinite;
        color: var(--text-dim);
    }
    @keyframes blink {
        to { visibility: hidden; }
    }
    /* Markdown 内容样式 — 极致紧凑 */
    /* 关键：覆盖 .bubble 默认的 pre-wrap。marked 输出的 HTML 标签间有 source-only `\n`，
       pre-wrap 会把那些 `\n` 渲染成可见空行——经典 bug，markdown 气泡必须用 normal。 */
    .bubble.md { line-height: 1.32; font-size: 12.5px; white-space: normal; }
    .bubble.md :global(> *:first-child) { margin-top: 0; }
    .bubble.md :global(> *:last-child) { margin-bottom: 0; }
    .bubble.md :global(p) { margin: 0; }
    .bubble.md :global(p + p) { margin-top: 0; }
    .bubble.md :global(br) { line-height: 1; }
    .bubble.md :global(code) {
        background: color-mix(in srgb, var(--text) 12%, transparent);
        padding: 0 3px; border-radius: 2px;
        font-family: monospace; font-size: 11.5px;
    }
    .bubble.md :global(pre) {
        background: color-mix(in srgb, var(--text) 8%, var(--bg));
        padding: 4px 6px; border-radius: 3px;
        overflow-x: auto; font-size: 11.5px;
        margin: 2px 0; line-height: 1.3;
    }
    .bubble.md :global(pre code) { background: transparent; padding: 0; font-size: inherit; }
    .bubble.md :global(ul), .bubble.md :global(ol) { margin: 1px 0; padding-left: 16px; }
    .bubble.md :global(li) { margin: 0; }
    .bubble.md :global(li > p) { margin: 0; }
    .bubble.md :global(li > ul), .bubble.md :global(li > ol) { margin: 0; }
    .bubble.md :global(strong) { font-weight: 600; }
    .bubble.md :global(em) { font-style: italic; }
    .bubble.md :global(a) { color: var(--accent); }
    .bubble.md :global(h1),
    .bubble.md :global(h2),
    .bubble.md :global(h3),
    .bubble.md :global(h4) {
        margin: 3px 0 1px; font-weight: 600; line-height: 1.2;
    }
    .bubble.md :global(:first-child:is(h1, h2, h3, h4)) { margin-top: 0; }
    .bubble.md :global(h1) { font-size: 14px; }
    .bubble.md :global(h2) { font-size: 13px; }
    .bubble.md :global(h3), .bubble.md :global(h4) { font-size: 12.5px; }
    .bubble.md :global(blockquote) {
        border-left: 2px solid var(--divider);
        padding-left: 5px; margin: 1px 0;
        color: var(--text-dim);
    }
    .bubble.md :global(hr) {
        border: 0; border-top: 1px solid var(--divider);
        margin: 3px 0;
    }
    .bubble.md :global(table) {
        border-collapse: collapse; margin: 2px 0; font-size: 11.5px;
    }
    .bubble.md :global(th), .bubble.md :global(td) {
        border: 1px solid var(--divider); padding: 1px 5px;
    }
    .bubble.error {
        background: color-mix(in srgb, var(--error) 15%, var(--bg));
        color: var(--error);
        font-size: 12px;
    }
    .bubble.note {
        background: transparent;
        color: var(--text-dim);
        font-size: 12px;
        font-style: italic;
        align-self: center;
    }

    .input-area {
        display: flex; gap: 8px; padding: 8px;
        border-top: 1px solid var(--divider);
        flex-shrink: 0;
    }
    textarea {
        flex: 1; min-height: 36px; max-height: 120px; resize: none;
        padding: 6px 8px; border: 1px solid var(--divider);
        border-radius: 4px; background: var(--bg); color: var(--text);
        font-family: inherit; font-size: 13px;
    }

    /* Clear-context confirmation modal — mirrors AiSettings' danger-mode dialog. */
    .dialog-backdrop {
        position: fixed;
        inset: 0;
        z-index: 500;
        background: var(--overlay-strong);
        display: flex;
        align-items: center;
        justify-content: center;
    }
    .dialog {
        background: var(--bg);
        box-shadow: var(--raised);
        border-radius: var(--radius);
        padding: calc(24px * var(--density));
        max-width: 420px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }
    .dialog-title {
        font-size: 15px;
        font-weight: 600;
        color: var(--text);
    }
    .dialog-body {
        font-size: 13px;
        color: var(--text);
        line-height: 1.55;
        white-space: pre-line;
    }
    .btn-row {
        display: flex;
        gap: 8px;
        justify-content: flex-end;
        margin-top: 4px;
    }
</style>
