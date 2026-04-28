<script lang="ts">
    import * as app from "../stores/app.svelte.ts";
    import * as ai from "../ai/store.svelte.ts";
    import { toast } from "../stores/toast.svelte.ts";
    import { t } from "../i18n/index.svelte.ts";

    function prevent(e: Event) { e.preventDefault(); }

    function send(seq: string) {
        app.sendToTerminal(seq);
        app.clearModifiers();
    }

    function arrow(dir: app.ArrowDir) {
        const ctrl = app.ctrlActive();
        const alt = app.altActive();
        const mod = (ctrl && alt) ? 7 : ctrl ? 5 : alt ? 3 : 0;
        app.sendArrow(dir, mod);
        app.clearModifiers();
    }

    // 当前 tab 是否有活跃 SSH/local session——AI 面板要求已连接的终端做诊断对象。
    // 没连接就让按钮 disabled，避免点了没反应（aiVisible 在 AppShell 层会因 session 缺失静默不渲染）。
    let canOpenAi = $derived.by(() => {
        const tab = app.activeTab();
        if (!tab || (tab.type !== "ssh" && tab.type !== "local")) return false;
        return !!app.sessionIdForTab(tab.id);
    });

    // 移动端唤起 AI 时提示一次：建议横屏 + 两个工具不可用。
    // 模块级 flag——一次 app run 提一次；togglePanel 只有"开"动作时才提。
    let mobileHintShown = false;
    function toggleAi() {
        if (!ai.isOpen() && !canOpenAi) {
            toast.info(t("ai.no_session"));
            return;
        }
        if (!ai.isOpen() && !mobileHintShown) {
            toast.info(t("ai.mobile.hint"));
            mobileHintShown = true;
        }
        ai.togglePanel();
    }
</script>

<div class="keybar">
    <button class="key mod" class:active={app.ctrlActive()} onpointerdown={prevent} onclick={() => app.setCtrl(!app.ctrlActive())}>Ctrl</button>
    <button class="key mod" class:active={app.altActive()} onpointerdown={prevent} onclick={() => app.setAlt(!app.altActive())}>Alt</button>
    <button class="key" onpointerdown={prevent} onclick={() => send('\x1b')}>Esc</button>
    <button class="key" onpointerdown={prevent} onclick={() => send('\t')}>Tab</button>
    <button class="key" onpointerdown={prevent} onclick={() => arrow('A')}>↑</button>
    <button class="key" onpointerdown={prevent} onclick={() => arrow('B')}>↓</button>
    <button class="key" onpointerdown={prevent} onclick={() => arrow('D')}>←</button>
    <button class="key" onpointerdown={prevent} onclick={() => arrow('C')}>→</button>
    <button class="key" title="Snippets" onpointerdown={prevent} onclick={() => app.openSnippetPicker()}>⚡</button>
    <button class="key" class:active={ai.isOpen()} class:dim={!ai.isOpen() && !canOpenAi} title="AI Chat" onpointerdown={prevent} onclick={toggleAi}>AI</button>
</div>

<style>
    .keybar {
        display: flex;
        gap: 4px;
        padding: 6px 8px;
        background: var(--bg);
        border-top: 1px solid var(--divider);
        flex-shrink: 0;
    }
    .key {
        flex: 1;
        height: 36px;
        border: none;
        border-radius: 6px;
        background: var(--surface);
        color: var(--text-sub);
        font-family: inherit;
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
        -webkit-tap-highlight-color: transparent;
        user-select: none;
    }
    .key:active {
        background: var(--divider);
    }
    .key.mod.active {
        background: var(--accent);
        color: #fff;
    }
    .key.dim { opacity: 0.45; }
</style>
