<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { HighlightRule } from "../stores/app.svelte.ts";

  let items = $state<HighlightRule[]>([]);
  let newKw = $state("");
  let newColor = $state("#FF6B6B");

  onMount(refresh);
  async function refresh() { items = await app.loadHighlights(); }

  async function add() {
    if (!newKw.trim()) return;
    try {
      await invoke("add_highlight", { rule: { keyword: newKw.trim(), color: newColor, enabled: true } });
      newKw = "";
      await refresh();
    } catch (e: any) { alert("Add failed: " + String(e)); }
  }

  async function remove(kw: string) {
    await invoke("remove_highlight", { keyword: kw });
    await refresh();
  }

  async function resetDefaults() {
    try {
      await invoke("reset_highlights");
      await refresh();
    } catch (e: any) { alert("Reset failed: " + String(e)); }
  }
</script>

<div class="page">
  <div class="add-card neu-raised">
    <input type="text" bind:value={newKw} placeholder="Enter keyword..."
      onkeydown={(e) => { if (e.key === "Enter") add(); }} />
    <div class="color-row">
      <input type="color" bind:value={newColor} />
      <span class="color-hex">{newColor}</span>
    </div>
    <button class="btn btn-accent btn-sm" onclick={add} disabled={!newKw.trim()}>Add</button>
  </div>

  <div class="rules-list">
    {#each items as h (h.keyword)}
      <div class="rule-row">
        <span class="rule-dot" style="background: {h.color};"></span>
        <span class="rule-kw">{h.keyword}</span>
        <span class="rule-color">{h.color}</span>
        <button class="rule-del" onclick={() => remove(h.keyword)}>&times;</button>
      </div>
    {:else}
      <p class="empty">No highlight rules</p>
    {/each}
  </div>

  <button class="btn btn-sm reset-btn" onclick={resetDefaults}>Reset to Defaults</button>
</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 16px; }

  .add-card { padding: 16px; display: flex; flex-direction: column; gap: 10px; }
  .color-row { display: flex; align-items: center; gap: 10px; }
  .color-row input[type="color"] {
    width: 48px; height: 32px; padding: 2px;
    border: 1px solid var(--divider); border-radius: 4px;
    cursor: pointer; box-shadow: none;
  }
  .color-hex { font-size: 12px; font-family: monospace; color: var(--text-dim); }

  .rules-list { display: flex; flex-direction: column; gap: 4px; }
  .rule-row {
    display: flex; align-items: center; gap: 10px;
    padding: 10px 14px;
    background: var(--bg);
    box-shadow: var(--raised-sm);
    border-radius: var(--radius-sm);
  }
  .rule-dot { width: 12px; height: 12px; border-radius: 50%; flex-shrink: 0; }
  .rule-kw { font-weight: 600; font-family: monospace; font-size: 13px; flex: 1; }
  .rule-color { font-size: 11px; color: var(--text-dim); }
  .rule-del {
    background: none; border: none; font-size: 18px; color: var(--text-dim);
    cursor: pointer; padding: 0 4px; transition: color 0.1s;
  }
  .rule-del:hover { color: var(--error); }

  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
  .reset-btn { align-self: flex-start; }
</style>
