<script lang="ts">
    import * as app from "../stores/app.svelte.ts";

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
    <button class="key" class:active={app.keyboardVisible()} title="Keyboard" onpointerdown={prevent} onclick={() => app.toggleKeyboard()}>⌨</button>
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
    .key.mod.active, .key.active {
        background: var(--accent);
        color: #fff;
    }
</style>
