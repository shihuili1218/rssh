<script lang="ts">
    import * as ai from "./store.svelte.ts";
    import type { AiTargetKind, ChatItem, ConversationMeta } from "./types.ts";
    import CommandConfirmDialog from "./CommandConfirmDialog.svelte";
    import AuditPanel from "./AuditPanel.svelte";
    import DangerModeToggle from "./DangerModeToggle.svelte";
    import { renderMarkdown } from "./markdown.ts";
    import { formatTokenCount } from "./tokens.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import { onMount } from "svelte";

    // tabId 是 AI 会话身份（actor 跟 tab 同寿命，重连不丢）。
    // targetId 是当前 SSH/PTY session_id —— 给 executeCommand 路由 ssh_write/pty_write 用。
    // 重连后 targetId 会换（前端 prop 自动跟随），tabId 不变。
    let { tabId, targetKind, targetId } = $props<{
        tabId: string;
        targetKind: AiTargetKind;
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
    // 本会话累计 token 用量（actor 生命周期，清上下文不归零——花掉的钱不会退）。
    let tokens = $derived(ai.tokenUsage(tabId));
    // Currently running model: prefer the model the active session actually started
    // with (authoritative — a later settings change doesn't affect a live session);
    // fall back to the configured model (what will run) when there's no session yet.
    // Empty string when neither is known — the .model span still works as the spring.
    let currentModel = $derived(session?.model ?? ai.settings()?.model ?? "");

    // 该 profile 下持久化的历史对话 —— 仅会话未启动时展示（picker）。
    // null = 还没加载完，与空数组（确无历史）区分，避免列表闪现。
    let conversations = $state<ConversationMeta[] | null>(null);

    onMount(async () => {
        // 只拉 settings（提示词标题的 danger 旗等要它）。不在这里预启 session ——
        // shell 探测已移到 SSH 连接成功时跑（TerminalPane），开 panel 不再为探测拉
        // 起 actor。会话改为首次发消息时（send → ensureSession）惰性启动。
        if (!ai.settings()) {
            try { await ai.loadSettings(); } catch { /* 静默 */ }
        }
    });

    // 色条"发送到 AI"塞进来的输入：消费一次就清掉，避免切 tab 回来又灌一遍。
    // 无条件先把 $state 读进局部变量，确保依赖被跟踪（即便为 null）——否则首跑
    // 为空时 Svelte 5 不会登记对 _prefill 的依赖，后续 prefill 永远触发不了。
    $effect(() => {
        const p = ai.pendingPrefill();
        if (!p || p.tabId !== tabId) return;
        inputText = p.text;
        ai.clearPrefill(tabId);
        inputEl?.focus();
    });

    // 历史对话随当前 target 重新加载 —— AppShell 复用同一个 ChatPanel 实例，
    // 切 tab 只换 props 不重挂载，onMount 不会再跑，必须用 $effect 跟踪。
    // seq 守卫：快速连续切 tab 时丢弃迟到的旧响应，避免 A 的列表盖到 B 头上。
    let convSeq = 0;
    $effect(() => {
        const kind = targetKind;
        const id = targetId;
        if (session) return; // 会话已存在：picker 不展示，无需拉取
        conversations = null;
        const seq = ++convSeq;
        // 两个回调都同时 gate seq + session：用户开面板后立刻发消息，会话先
        // 起来、列表请求后返回 —— 此时 picker 已无意义，迟到的失败不该在活跃
        // 对话里弹错误 banner（seq 不增长，单靠它挡不住这条路径）。
        ai.listConversations(kind, id)
            .then((list) => { if (seq === convSeq && !session) conversations = list; })
            .catch((e) => {
                // 加载失败不挡新对话，但必须上 banner —— 静默置空会让"有历史但
                // 后端抽风"看起来跟"确无历史"一模一样，用户以为记录丢了。
                console.error("[ai] list conversations:", e);
                if (seq === convSeq && !session) {
                    conversations = [];
                    banner = errMsg(e);
                }
            });
    });

    $effect(() => {
        items.length;
        if (chatBoxEl) {
            queueMicrotask(() => { chatBoxEl!.scrollTop = chatBoxEl!.scrollHeight; });
        }
    });

    /** 单飞 guard：onMount 预热 + send() 都会调 ensureSession，并发时两次都看不到
     *  session 存在（store 写入是 startSession 完成后才落），双 startSession 后端会
     *  报 session_already_exists。promise 复用：第二个调用方等同一个 promise 完成。 */
    let ensureInFlight: Promise<void> | null = null;

    /** 没 session 就先启动；启动失败抛错。
     *  skill 固定 general —— 用户自定义 skill 已自动拼进 master prompt，让 LLM 自己路由。
     *  远端 shell 探测不在这里 —— 它在 SSH 连接时已跑过并写进 profile 缓存，
     *  startSession 从缓存读初始 shell（缓存 miss 则 POSIX 兜底）。 */
    async function ensureSession(): Promise<void> {
        if (session) return;
        if (ensureInFlight) return ensureInFlight;
        ensureInFlight = (async () => {
            const settings = ai.settings() ?? await ai.loadSettings();
            if (!settings.has_api_key) {
                throw new Error(t("ai.error.no_api_key"));
            }
            await ai.startSession({
                tabId, targetKind, targetId, skill: "general",
                provider: settings.provider, model: settings.model,
            });
        })();
        try {
            await ensureInFlight;
        } finally {
            ensureInFlight = null;
        }
    }

    // 同一行的 resume / delete 互斥：删除进行中点恢复同一行会产生可避免的
    // not_found 报错。按行互斥（不全局禁）—— 删 A 的几十毫秒里恢复 B 是合法操作。
    let deletingId = $state<string | null>(null);

    /** 点历史对话：actor 带旧 history 出生，UI 灌回存储的 timeline，直接可续聊。 */
    async function resumeConversation(id: string) {
        if (busy || session || deletingId === id) return;
        banner = null;
        busy = true;
        try {
            const settings = ai.settings() ?? await ai.loadSettings();
            if (!settings.has_api_key) {
                throw new Error(t("ai.error.no_api_key"));
            }
            await ai.resumeSession({
                tabId, targetKind, targetId, skill: "general",
                provider: settings.provider, model: settings.model,
            }, id);
        } catch (e: any) {
            console.error("[ai] resume failed:", e);
            banner = errMsg(e);
        } finally {
            busy = false;
        }
    }

    async function deleteConversation(id: string) {
        if (busy || deletingId) return;
        deletingId = id;
        try {
            await ai.deleteConversation(id);
            conversations = (conversations ?? []).filter((c) => c.id !== id);
        } catch (e) {
            console.error("[ai] delete conversation:", e);
            banner = errMsg(e);
        } finally {
            deletingId = null;
        }
    }

    function fmtDate(ms: number) {
        return new Date(ms).toLocaleString();
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
        <!-- Current model: left-aligned, single line, ellipsis on overflow (full
             text on hover). Also the flex spring (flex:1) that pushes the controls
             to the right — replaces the old empty .grow spacer. -->
        <span class="model" title={currentModel}>{currentModel}</span>
        <span class="tokens" title={t("ai.toolbar.tokens_tip", { tin: tokens.tokens_in, tout: tokens.tokens_out })}>
            ↑{formatTokenCount(tokens.tokens_in)} ↓{formatTokenCount(tokens.tokens_out)}
        </span>
        <!-- Audit log toggle: file-text icon in chat view, chat bubble in audit view (= go back).
             Toolbar controls render unconditionally (stable layout); they disable until the
             session lazy-starts on first send — no actor, nothing to audit or clear. -->
        <button class="btn-icon" onclick={() => (auditOpen = !auditOpen)} disabled={!session}
                title={auditOpen ? t("ai.toolbar.back_to_chat") : t("ai.toolbar.audit")}
                aria-label={auditOpen ? t("ai.toolbar.back_to_chat") : t("ai.toolbar.audit")}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                {#if auditOpen}
                    <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
                {:else}
                    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                    <polyline points="14 2 14 8 20 8"/>
                    <line x1="16" y1="13" x2="8" y2="13"/>
                    <line x1="16" y1="17" x2="8" y2="17"/>
                    <polyline points="10 9 8 9"/>
                {/if}
            </svg>
        </button>
        <!-- 清理上下文：SVG 扫帚图标（22×22）跟"×"视觉重心对齐。 -->
        <button class="btn-icon" onclick={openClearDialog} disabled={!session} title={t("ai.toolbar.clear_context")} aria-label={t("ai.toolbar.clear_context")}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M20 4 L13 11"/>
                <path d="M11 9 L15 13"/>
                <path d="M11 9 L5 15"/>
                <path d="M12.33 10.33 L7 17"/>
                <path d="M13.67 11.67 L9 18.5"/>
                <path d="M15 13 L11 19.5"/>
            </svg>
        </button>
        <!-- Danger-mode toggle: always visible, selected (red) when ON. The toggle
             logic + confirm modal live in DangerModeToggle (shared with AiSettings —
             one safety contract); here we only render the icon. No disabled={!session}
             — danger_mode is a global setting, settable before the session starts. -->
        <DangerModeToggle onError={(m) => (banner = m)}>
            {#snippet trigger(requestToggle, saving)}
                <button class="btn-icon danger-toggle" class:on={dangerMode}
                        onclick={requestToggle} disabled={saving}
                        title={dangerMode ? t("ai.title.danger_tip") : t("ai.toolbar.danger_enable")}
                        aria-label={t("ai.toolbar.danger_aria")} aria-pressed={dangerMode}>
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
                        <line x1="12" y1="9" x2="12" y2="13"/>
                        <line x1="12" y1="17" x2="12.01" y2="17"/>
                    </svg>
                </button>
            {/snippet}
        </DangerModeToggle>
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
                {#if conversations && conversations.length > 0}
                    <div class="history">
                        <div class="history-title">{t("ai.history.title")}</div>
                        {#each conversations as c (c.id)}
                            <div class="history-row">
                                <button class="history-item" onclick={() => resumeConversation(c.id)}
                                        disabled={busy || deletingId === c.id} title={t("ai.history.resume_tip")}>
                                    <span class="history-name">{c.title || t("ai.history.untitled")}</span>
                                    <span class="history-time">{fmtDate(c.updated_at)}</span>
                                </button>
                                <!-- 删除全局互斥（deletingId 只能追踪一个 in-flight），禁用范围
                                     必须跟守卫一致：删除进行中所有删除按钮都禁，恢复按钮仍按行。 -->
                                <button class="btn-icon history-del" onclick={() => deleteConversation(c.id)}
                                        disabled={busy || deletingId !== null}
                                        title={t("ai.history.delete")} aria-label={t("ai.history.delete")}>×</button>
                            </div>
                        {/each}
                    </div>
                {/if}
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
    .model {
        flex: 1;
        min-width: 0;
        font-size: 11px;
        font-family: monospace;
        color: var(--text-dim);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .tokens {
        font-size: 10.5px;
        font-family: monospace;
        color: var(--text-dim);
        white-space: nowrap;
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
    .btn-icon:disabled {
        opacity: 0.35;
        cursor: default;
    }
    .btn-icon:disabled:hover { background: transparent; }
    /* Danger-mode toggle, selected state: red icon + red-tinted fill so it reads
       as "on" among the otherwise-neutral toolbar icons. The :hover rule keeps it
       red (overriding .btn-icon:hover's neutral color via higher specificity). */
    .danger-toggle.on {
        color: var(--error);
        background: color-mix(in srgb, var(--error) 14%, transparent);
    }
    .danger-toggle.on:hover {
        color: var(--error);
        background: color-mix(in srgb, var(--error) 22%, transparent);
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
    .placeholder.dim { font-size: 13px; padding: 32px 32px 8px; }
    .hint { font-size: 12px; }

    /* 历史对话 picker —— 仅空状态（无会话）时出现在欢迎语下方。 */
    .history { padding: 0 16px; display: flex; flex-direction: column; gap: 2px; }
    .history-title {
        font-size: 11px; font-weight: 600; color: var(--text-dim);
        text-transform: uppercase; letter-spacing: 0.05em;
        margin: 8px 0 4px;
    }
    .history-row { display: flex; align-items: center; gap: 2px; }
    .history-item {
        flex: 1; min-width: 0;
        display: flex; align-items: baseline; gap: 8px;
        padding: 5px 8px;
        background: transparent; border: none; cursor: pointer;
        border-radius: 4px; color: var(--text);
        text-align: left; font-size: 12.5px;
    }
    .history-item:hover { background: color-mix(in srgb, var(--text) 8%, transparent); }
    .history-item:disabled { opacity: 0.5; cursor: default; }
    .history-name {
        flex: 1; min-width: 0;
        overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    }
    .history-time {
        font-size: 10.5px; color: var(--text-dim);
        font-family: monospace; flex-shrink: 0;
    }
    .history-del { font-size: 14px; padding: 2px 5px; color: var(--text-dim); }

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
