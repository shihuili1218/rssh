<script lang="ts">
    /**
     * 通用浮动右键菜单。当前只服务 block-bar 的"折叠/展开"。
     * 留通用接口（items 数组）以便后续可加更多操作（如"导出到剪贴板"等）。
     */
    import { onMount } from "svelte";

    export interface MenuItem {
        label: string;
        action: () => void;
        /** 置灰但仍显示：让用户看到"这个操作存在，只是当前不可用"。 */
        disabled?: boolean;
    }

    let {
        x,
        y,
        items,
        onClose,
    }: {
        x: number;
        y: number;
        items: MenuItem[];
        onClose: () => void;
    } = $props();

    let containerEl: HTMLDivElement;
    // 视口边缘 clamp 偏移：先以 (x, y) 渲染，量出尺寸再算修正量，
    // 跟 TabContextMenu 一个套路。`ready` 在测量前隐藏，避免视觉闪一帧。
    let dx = $state(0);
    let dy = $state(0);
    let ready = $state(false);

    function onWindowMouseDown(e: MouseEvent) {
        if (containerEl && !containerEl.contains(e.target as Node)) onClose();
    }

    function onKeyDown(e: KeyboardEvent) {
        if (e.key === "Escape") {
            // 终端在底下接 keydown，Esc 会顺路被 vim/less/shell 吃掉。
            // preventDefault + stopPropagation 把事件吃干净，菜单消化完就结束。
            e.preventDefault();
            e.stopPropagation();
            onClose();
        }
    }

    onMount(() => {
        if (containerEl) {
            const r = containerEl.getBoundingClientRect();
            if (r.right > window.innerWidth) dx = window.innerWidth - r.right - 4;
            if (r.bottom > window.innerHeight) dy = window.innerHeight - r.bottom - 4;
            ready = true;
        }
        // 下一帧再挂监听，避免捕获触发本菜单的那一次 mousedown
        const id = setTimeout(() => {
            window.addEventListener("mousedown", onWindowMouseDown);
            window.addEventListener("keydown", onKeyDown);
        }, 0);
        return () => {
            clearTimeout(id);
            window.removeEventListener("mousedown", onWindowMouseDown);
            window.removeEventListener("keydown", onKeyDown);
        };
    });
</script>

<div bind:this={containerEl} class="block-menu" class:ready role="menu"
     style="left: {x + dx}px; top: {y + dy}px;">
    {#each items as item}
        <button
            class="menu-item"
            role="menuitem"
            disabled={item.disabled}
            onclick={() => {
                if (item.disabled) return;
                item.action();
                onClose();
            }}
        >
            {item.label}
        </button>
    {/each}
</div>

<style>
    .block-menu {
        position: fixed;
        z-index: 9999;
        background: var(--surface);
        border: 1px solid var(--divider);
        border-radius: 4px;
        box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
        padding: 4px 0;
        min-width: 120px;
        /* 测量前隐藏，避免初始位置在边缘时闪一下再被 clamp 拉回来 */
        visibility: hidden;
    }
    .block-menu.ready {
        visibility: visible;
    }
    .menu-item {
        display: block;
        width: 100%;
        padding: 6px 12px;
        background: transparent;
        color: var(--text);
        border: none;
        text-align: left;
        font-size: 13px;
        cursor: pointer;
        font-family: inherit;
    }
    .menu-item:hover:not(:disabled) {
        background: color-mix(in srgb, var(--text) 8%, transparent);
    }
    .menu-item:disabled {
        color: var(--text-dim);
        cursor: not-allowed;
    }
</style>
