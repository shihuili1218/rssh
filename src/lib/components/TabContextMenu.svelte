<script module lang="ts">
    export interface CtxMenuItem {
        label: string;
        shortcut?: string;
        onClick: () => void;
        disabled?: boolean;
        /** Optional flyout children. Items without it render exactly as before. */
        submenu?: CtxMenuItem[];
    }
</script>

<script lang="ts">
    import {onMount} from "svelte";

    let {x, y, sections, onClose}: {
        x: number;
        y: number;
        sections: CtxMenuItem[][];
        onClose: () => void;
    } = $props();

    let menuEl: HTMLDivElement | undefined;
    let dx = $state(0);
    let dy = $state(0);
    let ready = $state(false);
    // Submenus open to the right / downward by default; flip when out of room.
    let subFlipLeft = $state(false);
    let subFlipUp = $state(false);
    const SUBMENU_EST_W = 160;
    const SUBMENU_EST_H = 160;

    onMount(() => {
        if (!menuEl) return;
        const r = menuEl.getBoundingClientRect();
        if (r.right > window.innerWidth) dx = window.innerWidth - r.right - 4;
        if (r.bottom > window.innerHeight) dy = window.innerHeight - r.bottom - 4;
        subFlipLeft = r.right + dx + SUBMENU_EST_W > window.innerWidth;
        // Vertical: measure the real flyout anchor (one submenu today) rather
        // than guess item heights. dy is about to shift the menu up, so add it.
        // If a downward flyout would overflow the bottom, open it upward instead.
        const wrap = menuEl.querySelector(".ctx-sub-wrap");
        if (wrap) {
            const wrapTop = wrap.getBoundingClientRect().top + dy;
            subFlipUp = wrapTop + SUBMENU_EST_H > window.innerHeight;
        }
        ready = true;
    });

    function handleKeydown(e: KeyboardEvent) {
        if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onClose();
        }
    }

    function handleClick(item: CtxMenuItem) {
        if (item.disabled) return;
        item.onClick();
        onClose();
    }
</script>

<svelte:window onkeydown={handleKeydown}/>

<!-- backdrop = 纯遮罩，键盘用户走顶层 Escape（handleKeydown）退出，不需要键盘 click 等价物 -->
<div class="ctx-backdrop"
     onclick={onClose}
     oncontextmenu={(e) => { e.preventDefault(); onClose(); }}
     role="presentation"></div>

<!-- 故意不用 role="menu" / "menuitem"——那两个 role 暗示完整 ARIA menu pattern
     （方向键导航 + roving tabindex），但本组件只有 Esc 关闭，假装是 menu 会让屏幕阅读器
     用户期待方向键导航却得不到。承认它就是一组上下文按钮：<button> 默认就是 button 角色，
     Tab 键可遍历、Enter/Space 可激活，简单诚实。 -->
<div class="ctx-menu surface-raised"
     class:ready
     bind:this={menuEl}
     style="left: {x + dx}px; top: {y + dy}px;">
    {#each sections as section, si (si)}
        {#if si > 0}<div class="ctx-sep"></div>{/if}
        {#each section as item, ii (ii)}
            {#if item.submenu}
                <!-- Parent stays clickable (= plain new window); hover/focus reveals
                     the flyout. Flyout is a SIBLING, not a child — <button> can't nest. -->
                <div class="ctx-sub-wrap" class:flip-left={subFlipLeft} class:flip-up={subFlipUp}>
                    <button class="ctx-item"
                            class:disabled={item.disabled}
                            disabled={item.disabled}
                            onclick={() => handleClick(item)}>
                        <span class="ctx-label">{item.label}</span>
                        {#if item.shortcut}<span class="ctx-shortcut">{item.shortcut}</span>{/if}
                        <span class="ctx-caret" aria-hidden="true">›</span>
                    </button>
                    <div class="ctx-submenu surface-raised">
                        {#each item.submenu ?? [] as sub, ki (ki)}
                            <button class="ctx-item"
                                    class:disabled={sub.disabled}
                                    disabled={sub.disabled}
                                    onclick={() => handleClick(sub)}>
                                <span class="ctx-label">{sub.label}</span>
                                {#if sub.shortcut}<span class="ctx-shortcut">{sub.shortcut}</span>{/if}
                            </button>
                        {/each}
                    </div>
                </div>
            {:else}
                <button class="ctx-item"
                        class:disabled={item.disabled}
                        disabled={item.disabled}
                        onclick={() => handleClick(item)}>
                    <span class="ctx-label">{item.label}</span>
                    {#if item.shortcut}<span class="ctx-shortcut">{item.shortcut}</span>{/if}
                </button>
            {/if}
        {/each}
    {/each}
</div>

<style>
    .ctx-backdrop {
        position: fixed;
        inset: 0;
        z-index: 500;
    }
    .ctx-menu {
        position: fixed;
        z-index: 501;
        min-width: 200px;
        padding: calc(4px * var(--density));
        background: var(--bg);
        box-shadow: var(--raised);
        border-radius: var(--radius);
        display: flex;
        flex-direction: column;
        gap: 1px;
        /* Hidden until positioned to avoid flash off-screen */
        visibility: hidden;
    }
    .ctx-menu.ready {
        visibility: visible;
    }
    .ctx-item {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 20px;
        padding: 7px 12px;
        border: none;
        border-radius: var(--radius-sm);
        background: transparent;
        color: var(--text);
        font-family: inherit;
        font-size: 13px;
        text-align: left;
        cursor: pointer;
    }
    .ctx-item:hover:not(:disabled) {
        background: var(--surface);
    }
    .ctx-item:disabled,
    .ctx-item.disabled {
        color: var(--text-dim);
        cursor: not-allowed;
    }
    .ctx-label {
        flex: 1;
        white-space: nowrap;
    }
    .ctx-shortcut {
        color: var(--text-dim);
        font-size: 11px;
        font-family: monospace;
        letter-spacing: 0.5px;
    }
    .ctx-sep {
        height: 1px;
        background: var(--divider);
        margin: 4px 6px;
    }
    .ctx-sub-wrap {
        position: relative;
        display: flex;
        flex-direction: column;
    }
    .ctx-caret {
        color: var(--text-dim);
        font-size: 14px;
        line-height: 1;
    }
    /* Flyout abuts the menu edge (no gap) so the pointer never crosses dead
       space on its way in — otherwise :hover drops and the submenu closes. */
    .ctx-submenu {
        position: absolute;
        top: 0;
        left: 100%;
        z-index: 502;
        min-width: 140px;
        padding: calc(4px * var(--density));
        background: var(--bg);
        box-shadow: var(--raised);
        border-radius: var(--radius);
        display: none;
        flex-direction: column;
        gap: 1px;
    }
    .ctx-sub-wrap:hover > .ctx-submenu,
    .ctx-sub-wrap:focus-within > .ctx-submenu {
        display: flex;
    }
    .ctx-sub-wrap.flip-left > .ctx-submenu {
        left: auto;
        right: 100%;
    }
    .ctx-sub-wrap.flip-up > .ctx-submenu {
        top: auto;
        bottom: 0;
    }
</style>
