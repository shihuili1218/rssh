<!--
  SearchSelect — a searchable dropdown: trigger + panel with a search box and
  a filtered option list. The query resets to empty every time the panel opens
  (regaining focus starts a fresh search). Reuses the global .rssh-select-*
  classes (trigger / option / value / arrow) so it matches every other
  dropdown, including the three shape themes.

  `allowCustom` turns it into a combobox: the search box edits the bound value
  live (like a plain text input), so callers like the model picker keep
  free-text entry — typing then saving keeps the entry without picking a row.
  Optional `headerAccessory` snippet renders a control to the right of the
  search box (e.g. a filter toggle). The wrapper is .search-select (NOT
  .rssh-select) to avoid Select.svelte's open-state "transparent floating
  trigger" trick, which does not fit a panel-below-trigger layout.
-->
<script lang="ts">
    import { onMount, type Snippet } from "svelte";

    type Option = { value: string; label: string };

    let {
        value = $bindable<string>(),
        options,
        onchange,
        id,
        ariaLabel,
        placeholder = "",
        searchPlaceholder = "",
        emptyText = "",
        allowCustom = false,
        headerAccessory,
    }: {
        value: string;
        options: Option[];
        onchange?: (value: string) => void;
        id?: string;
        ariaLabel?: string;
        placeholder?: string;
        searchPlaceholder?: string;
        emptyText?: string;
        allowCustom?: boolean;
        headerAccessory?: Snippet;
    } = $props();

    let open = $state(false);
    let query = $state("");
    let dropUp = $state(false);
    let triggerEl: HTMLButtonElement | undefined = $state();
    let panelEl: HTMLDivElement | undefined = $state();
    let searchEl: HTMLInputElement | undefined = $state();

    // Match on label AND value, so typing a model id still surfaces its option
    // even when the label is a display name.
    let filtered = $derived.by(() => {
        const q = query.trim().toLowerCase();
        if (!q) return options;
        return options.filter(
            (o) => o.label.toLowerCase().includes(q) || o.value.toLowerCase().includes(q),
        );
    });

    // Label of the matching option, else the raw value (so custom values still
    // show), else the placeholder.
    let displayLabel = $derived(
        options.find((o) => o.value === value)?.label ?? (value || placeholder),
    );

    function pickPlacement() {
        if (!triggerEl) return;
        const rect = triggerEl.getBoundingClientRect();
        const below = window.innerHeight - rect.bottom;
        dropUp = below < 300 && rect.top > below;
    }

    function openPanel() {
        query = ""; // Regaining focus always starts from an empty search.
        pickPlacement();
        open = true;
        requestAnimationFrame(() => searchEl?.focus());
    }
    function close() { open = false; }
    function toggle() { open ? close() : openPanel(); }

    // Combobox: when custom values are allowed, the search box edits the value
    // live so typing then saving keeps the entry even without picking a row.
    // Pick-only callers (fonts) only filter; their value changes on select().
    function onSearchInput(e: Event) {
        query = (e.currentTarget as HTMLInputElement).value;
        if (allowCustom) value = query;
    }

    function select(v: string) {
        value = v;
        onchange?.(v);
        close();
        triggerEl?.focus();
    }

    function onSearchKeydown(e: KeyboardEvent) {
        if (e.key === "Escape") {
            e.preventDefault();
            close();
            triggerEl?.focus();
        } else if (e.key === "Enter") {
            e.preventDefault();
            if (filtered.length) select(filtered[0].value);
            else { close(); triggerEl?.focus(); } // custom value already committed live
        }
    }

    function onWindowClick(e: MouseEvent) {
        if (!open) return;
        const target = e.target as Node;
        if (triggerEl?.contains(target) || panelEl?.contains(target)) return;
        close();
    }

    function onWindowKeydown(e: KeyboardEvent) {
        if (open && e.key === "Escape") { close(); triggerEl?.focus(); }
    }

    // Close when keyboard focus leaves the control entirely (Tab-out), so an
    // open panel is not left orphaned on screen.
    function onFocusOut(e: FocusEvent) {
        if (!open) return;
        const next = e.relatedTarget as Node | null;
        // No relatedTarget = focus didn't move to another focusable element. On
        // macOS WebView, clicking an option <button> does NOT focus it, so the
        // search box blurs with relatedTarget=null — closing here would unmount
        // the option before its click runs and the pick would be lost. Genuine
        // outside clicks are caught by onWindowClick; real Tab-out always carries
        // a relatedTarget, so this only suppresses the spurious close.
        if (!next) return;
        if (triggerEl?.contains(next) || panelEl?.contains(next)) return;
        close();
    }

    onMount(() => {
        // Reposition on scroll AND resize while open (mirrors Select.svelte) —
        // otherwise a resize/orientation change misplaces the panel until the
        // next scroll.
        window.addEventListener("scroll", pickPlacement, true);
        window.addEventListener("resize", pickPlacement);
        return () => {
            window.removeEventListener("scroll", pickPlacement, true);
            window.removeEventListener("resize", pickPlacement);
        };
    });
</script>

<svelte:window onclick={onWindowClick} onkeydown={onWindowKeydown} />

<div class="search-select" class:open class:drop-up={dropUp} onfocusout={onFocusOut}>
    <button
        {id}
        type="button"
        class="rssh-select-trigger"
        aria-haspopup="true"
        aria-expanded={open}
        aria-label={ariaLabel}
        onclick={toggle}
        bind:this={triggerEl}
    >
        <span class="rssh-select-value" class:placeholder={displayLabel === placeholder}>{displayLabel || " "}</span>
        <span class="rssh-select-arrow" aria-hidden="true"></span>
    </button>

    {#if open}
        <div class="search-select-panel surface-raised" bind:this={panelEl}>
            <div class="search-select-head">
                <input
                    class="search-select-search"
                    type="text"
                    value={query}
                    bind:this={searchEl}
                    placeholder={searchPlaceholder}
                    oninput={onSearchInput}
                    onkeydown={onSearchKeydown}
                    spellcheck="false"
                    autocomplete="off"
                    aria-label={searchPlaceholder || ariaLabel}
                />
                {#if headerAccessory}{@render headerAccessory()}{/if}
            </div>
            <ul class="search-select-list">
                {#each filtered as opt (opt.value)}
                    <li>
                        <button
                            type="button"
                            class="rssh-select-option"
                            class:active={opt.value === value}
                            onclick={() => select(opt.value)}
                        >{opt.label}</button>
                    </li>
                {/each}
                {#if filtered.length === 0}
                    <li class="search-select-empty">{emptyText}</li>
                {/if}
            </ul>
        </div>
    {/if}
</div>

<style>
    .search-select {
        position: relative;
        width: 100%;
    }
    /* Flip the shared arrow when open (the shared .rssh-select.open rule does
       not apply: the wrapper is .search-select, not .rssh-select). */
    .search-select.open :global(.rssh-select-arrow) {
        transform: rotate(180deg);
    }

    .search-select-panel {
        position: absolute;
        left: 0;
        right: 0;
        z-index: 10;
        padding: 6px;
        background: var(--surface);
        border-radius: 6px;
        box-shadow: var(--raised-sm);
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .search-select:not(.drop-up) .search-select-panel { top: calc(100% + 4px); }
    .search-select.drop-up .search-select-panel { bottom: calc(100% + 4px); }

    .search-select-head {
        display: flex;
        align-items: center;
        gap: 8px;
    }
    .search-select-search {
        flex: 1;
        min-width: 0;
        padding: 6px 10px;
        border: 1px solid var(--divider);
        border-radius: 4px;
        background: var(--bg);
        color: var(--text);
        font-size: 13px;
        font-family: inherit;
    }
    .search-select-search:focus-visible {
        outline: none;
        border-color: var(--accent);
    }

    .search-select-list {
        list-style: none;
        margin: 0;
        padding: 0;
        max-height: 240px;
        overflow-y: auto;
    }
    .search-select-empty {
        padding: 10px;
        text-align: center;
        font-size: 12px;
        color: var(--text-dim);
    }
</style>
