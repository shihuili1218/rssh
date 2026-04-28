<script lang="ts">
    import * as ai from "./store.svelte.ts";
    import type { ChatItem } from "./types.ts";
    import CommandConfirmDialog from "./CommandConfirmDialog.svelte";
    import AuditPanel from "./AuditPanel.svelte";
    import { renderMarkdown } from "./markdown.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import { onMount } from "svelte";

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

    let session = $derived(ai.sessionForTarget(targetId));
    let items: ChatItem[] = $derived(session ? ai.chatItems(session.session_id) : []);

    onMount(() => {
        if (!ai.settings()) ai.loadSettings().catch(() => {});
    });

    $effect(() => {
        items.length;
        if (chatBoxEl) {
            queueMicrotask(() => { chatBoxEl!.scrollTop = chatBoxEl!.scrollHeight; });
        }
    });

    /** 没 session 就先启动，然后返回 session_id；启动失败抛错。
     *  skill 固定 general —— 用户自定义 skill 已自动拼进 master prompt，让 LLM 自己路由。 */
    async function ensureSession(): Promise<string> {
        if (session) return session.session_id;
        const settings = ai.settings() ?? await ai.loadSettings();
        if (!settings.has_api_key) {
            throw new Error(t("ai.error.no_api_key"));
        }
        const info = await ai.startSession({
            targetKind, targetId, skill: "general",
            provider: settings.provider, model: settings.model,
        });
        return info.session_id;
    }

    async function send() {
        const text = inputText.trim();
        if (!text || busy) return;
        banner = null;
        busy = true;
        try {
            const sid = await ensureSession();
            inputText = "";
            await ai.sendMessage(sid, text);
        } catch (e: any) {
            console.error("[ai] send failed:", e);
            banner = errMsg(e);
        } finally {
            busy = false;
        }
    }

    /** 关面板 = 停止会话（一个语义比两个简单） */
    async function closePanel() {
        if (session) {
            try { await ai.stopSession(session.session_id); } catch (e) { console.error("[ai] stop:", e); }
        }
        ai.closePanel();
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
        {#if session}
            <button class="btn btn-ghost btn-sm" onclick={() => (auditOpen = !auditOpen)}>
                {auditOpen ? t("ai.toolbar.back_to_chat") : t("ai.toolbar.audit")}
            </button>
        {/if}
        <span class="grow"></span>
        <button class="btn-icon" onclick={closePanel} title={t("ai.toolbar.close_session")}>×</button>
    </div>

    {#if banner}
        <div class="banner">
            <span>{banner}</span>
            <button class="btn-icon" onclick={() => (banner = null)}>×</button>
        </div>
    {/if}

    {#if auditOpen && session}
        <AuditPanel sessionId={session.session_id} />
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
                        <div class="bubble assistant md" class:streaming={item.streaming}>
                            {@html renderMarkdown(item.text || "…")}
                        </div>
                    {:else if item.kind === "command" && session}
                        <CommandConfirmDialog
                            sessionId={session.session_id}
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
                placeholder={busy ? (session ? t("ai.input.replying") : t("ai.input.starting")) : t("ai.input.placeholder")}
                onkeydown={onKeyDown}
                disabled={busy}
            ></textarea>
            <button class="btn btn-primary" onclick={send} disabled={!inputText.trim() || busy}>
                {busy && !session ? t("ai.input.starting_short") : t("ai.input.send")}
            </button>
        </div>
    {/if}
</div>

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
    .grow { flex: 1; }
    .btn-primary { background: #4a86e8; color: #fff; border-color: #4a86e8; }
    .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
    .btn-ghost { background: transparent; }
    .btn-icon {
        background: transparent; border: none;
        font-size: 18px; cursor: pointer;
        color: var(--text); padding: 0 6px;
    }
    .banner {
        display: flex; align-items: center; gap: 8px;
        padding: 8px 12px;
        background: color-mix(in srgb, #c0392b 18%, var(--bg));
        color: #c0392b;
        border-bottom: 1px solid var(--divider);
        font-size: 12px;
        flex-shrink: 0;
    }
    .banner span { flex: 1; word-break: break-word; }

    .placeholder {
        padding: 24px; text-align: center;
        color: var(--text-dim, #888);
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
        font-size: 10px; color: var(--text-dim, #888);
        font-family: monospace;
    }
    .bubble {
        padding: 5px 9px; border-radius: 6px;
        max-width: 95%; word-break: break-word; white-space: pre-wrap;
        font-size: 13px;
    }
    .bubble.user {
        background: #4a86e8; color: #fff;
        align-self: flex-end;
    }
    .bubble.assistant {
        background: color-mix(in srgb, var(--text) 8%, var(--bg));
        align-self: flex-start;
    }
    .bubble.assistant.streaming {
        position: relative;
    }
    .bubble.assistant.streaming::after {
        content: "▋";
        display: inline-block;
        margin-left: 2px;
        animation: blink 1s steps(2, start) infinite;
        color: var(--text-dim, #888);
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
    .bubble.md :global(a) { color: var(--accent, #4a86e8); }
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
        color: var(--text-dim, #888);
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
        background: color-mix(in srgb, #c0392b 15%, var(--bg));
        color: #c0392b;
        font-size: 12px;
    }
    .bubble.note {
        background: transparent;
        color: var(--text-dim, #888);
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
</style>
