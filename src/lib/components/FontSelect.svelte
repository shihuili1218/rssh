<!--
  FontSelect — terminal-font picker. A thin wrapper over SearchSelect that
  adds the font-specific bits: the system font list, the "Default" entry, and
  the monospace filter rendered as the search row's accessory. All the
  dropdown mechanics (search, reset-on-open, placement, keyboard) live in
  SearchSelect.
-->
<script lang="ts">
    import SearchSelect from "./SearchSelect.svelte";
    import { t } from "../i18n/index.svelte.ts";
    import type { FontInfo } from "../themes/term-font.ts";

    let {
        value = $bindable<string>(),
        fonts,
        onchange,
        ariaLabel,
    }: {
        value: string;
        fonts: FontInfo[];
        onchange?: (family: string) => void;
        ariaLabel?: string;
    } = $props();

    // Session-only view filter — not persisted; it only helps locate a font.
    let monospaceOnly = $state(true);

    let options = $derived([
        { value: "", label: t("settings.appearance.font.default") },
        ...fonts
            .filter((f) => !monospaceOnly || f.monospaced)
            .map((f) => ({ value: f.family, label: f.family })),
    ]);
</script>

<SearchSelect
    bind:value={value}
    options={options}
    onchange={onchange}
    ariaLabel={ariaLabel}
    searchPlaceholder={t("settings.appearance.font.search")}
    emptyText={t("settings.appearance.font.none")}
>
    {#snippet headerAccessory()}
        <label class="font-mono-toggle">
            <input type="checkbox" bind:checked={monospaceOnly} />
            {t("settings.appearance.font.mono")}
        </label>
    {/snippet}
</SearchSelect>

<style>
    .font-mono-toggle {
        display: flex;
        align-items: center;
        gap: 5px;
        font-size: 12px;
        color: var(--text-sub);
        white-space: nowrap;
        cursor: pointer;
        user-select: none;
    }
    .font-mono-toggle input { cursor: pointer; }
</style>
