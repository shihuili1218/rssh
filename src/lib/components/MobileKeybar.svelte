<script lang="ts">
    import * as app from "../stores/app.svelte.ts";

    function prevent(e: Event) { e.preventDefault(); }

    function send(seq: string, arrow = false) {
        const ctrl = app.ctrlActive();
        const alt = app.altActive();
        if (arrow && (ctrl || alt)) {
            const mod = (ctrl && alt) ? 7 : ctrl ? 5 : 3;
            seq = seq.replace(/\x1b\[([A-D])/, `\x1b[1;${mod}$1`);
        }
        app.sendToTerminal(seq);
        app.clearModifiers();
    }
</script>

<div class="keybar">
    <button class="key mod" class:active={app.ctrlActive()} onpointerdown={prevent} onclick={() => app.setCtrl(!app.ctrlActive())}>Ctrl</button>
    <button class="key mod" class:active={app.altActive()} onpointerdown={prevent} onclick={() => app.setAlt(!app.altActive())}>Alt</button>
    <button class="key" onpointerdown={prevent} onclick={() => send('\x1b')}>Esc</button>
    <button class="key" onpointerdown={prevent} onclick={() => send('\t')}>Tab</button>
    <button class="key" onpointerdown={prevent} onclick={() => send('\x1b[A', true)}>↑</button>
    <button class="key" onpointerdown={prevent} onclick={() => send('\x1b[B', true)}>↓</button>
    <button class="key" onpointerdown={prevent} onclick={() => send('\x1b[D', true)}>←</button>
    <button class="key" onpointerdown={prevent} onclick={() => send('\x1b[C', true)}>→</button>
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
</style>
