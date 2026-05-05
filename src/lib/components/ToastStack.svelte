<script lang="ts">
    import { toasts, dismiss } from "../stores/toast.svelte.ts";
</script>

<div class="toast-stack">
    {#each toasts() as item (item.id)}
        <button class="toast toast-{item.kind} surface-raised" onclick={() => dismiss(item.id)}>
            {item.message}
        </button>
    {/each}
</div>

<style>
    .toast-stack {
        position: fixed;
        top: 16px;
        right: 16px;
        z-index: 1000;
        display: flex;
        flex-direction: column;
        gap: 8px;
        max-width: 380px;
        pointer-events: none;
    }
    .toast {
        padding: calc(10px * var(--density)) calc(14px * var(--density));
        border: none;
        border-radius: var(--radius-sm);
        background: var(--bg);
        box-shadow: var(--raised);
        color: var(--text);
        font-family: inherit;
        font-size: 13px;
        text-align: left;
        pointer-events: auto;
        cursor: pointer;
        animation: slide-in 0.15s ease-out;
        word-break: break-word;
    }
    .toast-error { border-left: 3px solid var(--error); }
    .toast-success { border-left: 3px solid var(--success); }
    .toast-info { border-left: 3px solid var(--accent); }
    @keyframes slide-in {
        from { transform: translateX(16px); opacity: 0; }
        to { transform: translateX(0); opacity: 1; }
    }
</style>
