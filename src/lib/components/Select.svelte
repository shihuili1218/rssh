<!--
  自定义 Select 组件 —— 原生 <select> 的展开态由浏览器 / OS 渲染，不响应 CSS，
  跟三主题（neumorphism / flat / material）的设计语言无法对齐。这里用 button + 列表
  实现，trigger 共享全局 input/select 视觉，下拉列表用 .surface-raised 卡片。

  API 模仿原生：
    <Select bind:value={x} options={[{value, label}]} onchange={cb} id />
  options 用数组而不是 children，因为大多数使用点都是动态构造（profiles, credentials, locales）。
-->
<script lang="ts">
    import { onMount } from "svelte";

    type OptionValue = string | number | null;
    type Option = { value: OptionValue; label: string; disabled?: boolean };

    let {
        value = $bindable<OptionValue>(),
        options,
        onchange,
        id,
        disabled = false,
        placeholder = "",
        ariaLabel,
        ariaLabelledby,
    }: {
        value: OptionValue;
        options: Option[];
        onchange?: (v: OptionValue) => void;
        id?: string;
        disabled?: boolean;
        placeholder?: string;
        /** Accessible name 给 SR — 当组件不在 <label for=...> 关联里时用 */
        ariaLabel?: string;
        /** Accessible name 引用方式 — 指向页面上其他 element 的 id */
        ariaLabelledby?: string;
    } = $props();

    let open = $state(false);
    let triggerEl: HTMLButtonElement | undefined = $state();
    let listEl: HTMLUListElement | undefined = $state();
    /** 下拉向上展开还是向下展开 —— 视 trigger 距视口底部的剩余空间决定。 */
    let dropUp = $state(false);
    /** trigger 实际高度，传给 list 作 padding-top —— list 是 absolute 浮层，要在
     *  自身内部留出跟 trigger 等高的空白区，让 trigger 的内容能"叠"在 list 顶部
     *  而不被 list 的选项盖住。font / density 改变时重测。 */
    let triggerH = $state(38);

    /** 当前选中项的 label，空时回落到 placeholder。 */
    let displayLabel = $derived(
        options.find((o) => o.value === value)?.label ?? placeholder,
    );

    function measureTrigger() {
        if (!triggerEl) return;
        const h = triggerEl.offsetHeight;
        if (h > 0) triggerH = h;
    }

    function pickPlacement() {
        if (!triggerEl) return;
        const rect = triggerEl.getBoundingClientRect();
        const spaceBelow = window.innerHeight - rect.bottom;
        const spaceAbove = rect.top;
        // 选项面板 max-height 240 + gap 4 ≈ 244。能放下就向下，否则向上。
        dropUp = spaceBelow < 244 && spaceAbove > spaceBelow;
    }

    function toggle() {
        if (disabled) return;
        if (!open) {
            measureTrigger();
            pickPlacement();
        }
        open = !open;
    }

    function close() {
        open = false;
    }

    function select(opt: Option) {
        if (opt.disabled) return;
        value = opt.value;
        onchange?.(opt.value);
        close();
        triggerEl?.focus();
    }

    function onWindowClick(e: MouseEvent) {
        if (!open) return;
        const target = e.target as Node;
        if (triggerEl?.contains(target) || listEl?.contains(target)) return;
        close();
    }

    function onKeydown(e: KeyboardEvent) {
        if (!open) return;
        if (e.key === "Escape") {
            e.preventDefault();
            close();
            triggerEl?.focus();
        }
    }

    /** Tab 移焦点出 select 时关掉下拉 —— 否则点开后 Tab 到下个控件，
     *  下拉还浮着挡视野。relatedTarget 是 focus 下一个落点，null 通常是点 chrome 之外。 */
    function onFocusOut(e: FocusEvent) {
        if (!open) return;
        const next = e.relatedTarget as Node | null;
        if (next && (triggerEl?.contains(next) || listEl?.contains(next))) return;
        close();
    }

    onMount(() => {
        measureTrigger();
        const onResize = () => { measureTrigger(); pickPlacement(); };
        window.addEventListener("resize", onResize);
        window.addEventListener("scroll", pickPlacement, true);
        return () => {
            window.removeEventListener("resize", onResize);
            window.removeEventListener("scroll", pickPlacement, true);
        };
    });
</script>

<svelte:window onclick={onWindowClick} onkeydown={onKeydown} />

<div class="rssh-select" class:open class:disabled class:drop-up={dropUp}
     style:--rssh-trigger-h="{triggerH}px"
     onfocusout={onFocusOut}>
    <!-- 不声明 role="listbox" / role="option"：ARIA listbox 模式要求 ArrowKey
         导航 + active-descendant，rssh 当前实现是 button 列表（鼠标交互为主）。
         role lying 比 no role 更糟 —— screen reader 会按 listbox 期望去找
         active option 然后困惑。aria-haspopup="true" 仍传达"按下有 popup"。 -->
    <button
        {id}
        type="button"
        class="rssh-select-trigger"
        {disabled}
        aria-haspopup="true"
        aria-expanded={open}
        aria-label={ariaLabel}
        aria-labelledby={ariaLabelledby}
        onclick={toggle}
        bind:this={triggerEl}
    >
        <span class="rssh-select-value" class:placeholder={value === null || value === undefined || value === "" || displayLabel === placeholder}>
            {displayLabel || "\u00A0"}
        </span>
        <span class="rssh-select-arrow" aria-hidden="true"></span>
    </button>

    {#if open}
        <ul class="rssh-select-list surface-raised" bind:this={listEl}>
            {#each options as opt (opt.value)}
                <li>
                    <button
                        type="button"
                        class="rssh-select-option"
                        class:active={opt.value === value}
                        disabled={opt.disabled}
                        aria-pressed={opt.value === value}
                        onclick={() => select(opt)}
                    >
                        {opt.label}
                    </button>
                </li>
            {/each}
        </ul>
    {/if}
</div>

<!-- 所有 .rssh-select-* 都用 :global() 暴露给外部主题样式钩子（shapes/*.css）。
     不这样的话 Svelte 给 class 加 hash，全局 css 里裸 .rssh-select-trigger 选不中。 -->
<style>
    /* Wrapper 仅作定位锚 —— 不撑高度、不画阴影、不画背景。
       Open 态的"整体凸起卡片"由 list 作为 absolute 浮层接管：list 自身包含 trigger
       那一块区域（padding-top = trigger 高度），整张卡片只有一个阴影 box-shadow，
       trigger 在 list 顶部透明浮起当 click 区。
       这样不挤压周围内容，又保留"trigger + 选项一体"的视觉。 */
    :global(.rssh-select) {
        position: relative;
        width: 100%;
    }

    /* Trigger：跟全局 input/select 同源视觉（surface bg + pressed 凹陷），
       三个 shape 主题各自的覆盖见 styles/shapes/*.css。 */
    :global(.rssh-select-trigger) {
        display: flex;
        align-items: center;
        justify-content: space-between;
        width: 100%;
        padding: 8px 12px;
        background: var(--surface);
        box-shadow: var(--pressed);
        border-radius: 6px;
        border: 1.5px solid transparent;
        color: var(--text);
        font-size: 13px;
        font-family: inherit;
        line-height: 1.4;
        cursor: pointer;
        transition: box-shadow 0.13s, border-color 0.13s, border-radius 0.13s;
    }
    :global(.rssh-select-trigger:hover:not(:disabled)) {
        box-shadow: var(--raised-sm);
    }
    :global(.rssh-select-trigger:focus-visible) {
        outline: none;
        border-color: var(--accent);
    }
    :global(.rssh-select.disabled .rssh-select-trigger),
    :global(.rssh-select-trigger:disabled) {
        cursor: not-allowed;
        opacity: 0.5;
    }

    /* Trigger 在 open 态：透明 + 无阴影，z-index 高过 list，作为浮在 list 顶部的 click 区。
       List 自身的 background/box-shadow 透过 trigger 显出来，整体看是 list 这一张卡片。 */
    :global(.rssh-select.open .rssh-select-trigger) {
        background: transparent;
        box-shadow: none;
        border-color: transparent;
        position: relative;
        z-index: 2;
    }
    /* open 态的 transparent border 把 :focus-visible 的 accent 焦点环也盖了。
       键盘 Tab 到 open trigger 必须看到焦点 —— 加一条 specificity 更高的复合
       规则把 accent border 恢复回来。 */
    :global(.rssh-select.open .rssh-select-trigger:focus-visible) {
        border-color: var(--accent);
    }

    :global(.rssh-select-value) {
        flex: 1;
        text-align: left;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    :global(.rssh-select-value.placeholder) {
        color: var(--text-dim);
    }

    /* 箭头用 CSS 三角而不是 SVG，跟着 currentColor / 主题色走，open 时翻转。 */
    :global(.rssh-select-arrow) {
        width: 0;
        height: 0;
        margin-left: 8px;
        border-left: 4px solid transparent;
        border-right: 4px solid transparent;
        border-top: 5px solid var(--text-dim);
        transition: transform 0.15s;
        flex-shrink: 0;
    }
    :global(.rssh-select.open .rssh-select-arrow) {
        transform: rotate(180deg);
    }

    /* 下拉列表：absolute 浮层，覆盖 trigger 区域 + 向下/向上延伸 240px 选项区。
       一张完整 box-shadow + 一个统一 surface 背景 = 一体感。
       padding-top (drop-down) 或 padding-bottom (drop-up) 等于 trigger 高度，
       option 在剩余空间里渲染；接缝处 ::before 画 1px divider 分隔 trigger 跟选项区。 */
    :global(.rssh-select-list) {
        position: absolute;
        left: 0;
        right: 0;
        z-index: 1;
        margin: 0;
        padding: 4px;
        list-style: none;
        max-height: calc(var(--rssh-trigger-h, 38px) + 248px);
        overflow-y: auto;
        background: var(--surface);
        border-radius: 6px;
        box-shadow: var(--raised-sm);
    }
    :global(.rssh-select:not(.drop-up) .rssh-select-list) {
        top: 0;
        padding-top: calc(var(--rssh-trigger-h, 38px) + 4px);
    }
    :global(.rssh-select.drop-up .rssh-select-list) {
        bottom: 0;
        padding-bottom: calc(var(--rssh-trigger-h, 38px) + 4px);
    }
    /* 接缝 divider —— 一条横线，紧贴 trigger 跟 option 列表之间。 */
    :global(.rssh-select-list::before) {
        content: "";
        position: absolute;
        left: 8px;
        right: 8px;
        height: 1px;
        background: var(--divider);
    }
    :global(.rssh-select:not(.drop-up) .rssh-select-list::before) {
        top: var(--rssh-trigger-h, 38px);
    }
    :global(.rssh-select.drop-up .rssh-select-list::before) {
        bottom: var(--rssh-trigger-h, 38px);
    }

    :global(.rssh-select-option) {
        display: block;
        width: 100%;
        padding: 6px 10px;
        background: transparent;
        border: none;
        border-radius: 4px;
        color: var(--text);
        font-size: 13px;
        font-family: inherit;
        text-align: left;
        cursor: pointer;
        box-shadow: none;
        transition: background 0.1s, color 0.1s;
    }
    :global(.rssh-select-option:hover:not(:disabled)) {
        background: color-mix(in srgb, var(--accent) 14%, transparent);
    }
    :global(.rssh-select-option.active) {
        color: var(--accent);
        background: color-mix(in srgb, var(--accent) 10%, transparent);
        font-weight: 600;
    }
    :global(.rssh-select-option:disabled) {
        opacity: 0.4;
        cursor: not-allowed;
    }
</style>
