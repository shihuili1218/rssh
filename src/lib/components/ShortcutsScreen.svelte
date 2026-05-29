<script lang="ts">
  import { t } from "../i18n/index.svelte.ts";

  const isMac = navigator.platform?.startsWith("Mac");
  const mod = isMac ? "⌘" : "Ctrl";

  let sections = $derived([
    {
      title: t("shortcuts.section.global"),
      keys: [
        ["Ctrl+Tab", t("shortcuts.next_tab")],
        ["Ctrl+Shift+Tab", t("shortcuts.prev_tab")],
        [`${mod}+W`, t("shortcuts.close_tab")],
        [`${mod}+Shift+D`, t("shortcuts.clone_tab")],
        [`${mod}+Shift+N`, t("shortcuts.open_new_window")],
        ["Esc", t("shortcuts.close_overlay")],
      ],
    },
    {
      title: t("shortcuts.section.home"),
      keys: [
        ["↑ ↓ ← →", t("shortcuts.select_card")],
        ["Enter", t("shortcuts.connect_card")],
      ],
    },
    {
      title: t("shortcuts.section.terminal"),
      keys: [
        [`${mod}+F`, t("shortcuts.search")],
        [`${mod}+S`, t("shortcuts.snippet")],
        [`${mod}+O`, t("shortcuts.open_sftp")],
        ["Any key", t("shortcuts.reconnect_disconnect")],
      ],
    },
    {
      title: t("shortcuts.section.port_forward"),
      keys: [
        ["Any key", t("shortcuts.reconnect_error")],
      ],
    },
  ]);
</script>

<div class="page">
  {#each sections as s}
    <div class="section-label">{s.title}</div>
    <div class="key-list">
      {#each s.keys as [key, desc]}
        <div class="key-row">
          <kbd class="kbd surface-pressed">{key}</kbd>
          <span class="key-desc">{desc}</span>
        </div>
      {/each}
    </div>
  {/each}
</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 4px; }
  .key-list { display: flex; flex-direction: column; gap: 2px; }
  .key-row {
    display: flex; align-items: center; gap: 12px;
    padding: 8px 0;
  }
  .kbd {
    display: inline-block;
    min-width: 80px;
    padding: calc(3px * var(--density)) calc(8px * var(--density));
    background: var(--surface);
    box-shadow: var(--pressed);
    border-radius: 6px;
    font-family: monospace;
    font-size: 12px;
    color: var(--text);
    text-align: center;
  }
  .key-desc { font-size: 13px; color: var(--text-sub); }
</style>
