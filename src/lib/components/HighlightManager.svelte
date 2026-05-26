<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import type { HighlightRule } from "../stores/app.svelte.ts";
  import * as app from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";

  let items = $state<HighlightRule[]>([]);
  let adding = $state(false);
  // Edit identity = keyword as currently stored on the backend (rename uses old → new).
  let editKw = $state<string | null>(null);

  // Form fields
  let formKw = $state("");
  let formColor = $state("#FF6B6B");
  let formEnabled = $state(true);

  onMount(refresh);

  async function refresh() {
    items = await app.loadHighlights();
    // Tell open TerminalPanes their highlight regex is stale. Local-only
    // bump (no backend round-trip) — TerminalPane's $effect re-reads the
    // DB and recompiles its regex. Without this, edits here only take
    // effect after the next terminal reconnect.
    app.bumpHighlights();
  }

  function startAdd() {
    adding = true;
    editKw = null;
    formKw = "";
    formColor = "#FF6B6B";
    formEnabled = true;
  }

  function startEdit(h: HighlightRule) {
    adding = false;
    editKw = h.keyword;
    formKw = h.keyword;
    formColor = h.color;
    formEnabled = h.enabled;
  }

  function cancelForm() {
    adding = false;
    editKw = null;
  }

  async function saveNew() {
    const kw = formKw.trim();
    if (!kw) return;
    try {
      await invoke("add_highlight", { rule: { keyword: kw, color: formColor, enabled: formEnabled } });
      adding = false;
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.add")}: ${errMsg(e)}`); }
  }

  async function saveEdit() {
    if (editKw === null) return;
    const kw = formKw.trim();
    if (!kw) return;
    try {
      await invoke("update_highlight", {
        oldKeyword: editKw,
        rule: { keyword: kw, color: formColor, enabled: formEnabled },
      });
      editKw = null;
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
  }

  async function remove(keyword: string) {
    try {
      await invoke("remove_highlight", { keyword });
      if (editKw === keyword) editKw = null;
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.delete")}: ${errMsg(e)}`); }
  }

  async function resetDefaults() {
    try {
      await invoke("reset_highlights");
      cancelForm();
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.reset")}: ${errMsg(e)}`); }
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-sm" onclick={resetDefaults}>Reset to Defaults</button>
    <button class="btn btn-accent btn-sm" onclick={startAdd}>+ New Highlight</button>
  </div>

  {#if adding}
    <div class="card inline-form">
      <label>
        <span class="label-text">Keyword</span>
        <input type="text" bind:value={formKw} placeholder="ERROR / WARN / your pattern"
          onkeydown={(e) => { if (e.key === "Enter") saveNew(); }} />
      </label>
      <label>
        <span class="label-text">Color</span>
        <div class="color-row">
          <input type="color" bind:value={formColor} />
          <span class="color-hex">{formColor}</span>
        </div>
      </label>
      <div class="form-actions">
        <button class="btn btn-accent btn-sm" onclick={saveNew} disabled={!formKw.trim()}>Save</button>
        <button class="btn btn-sm" onclick={cancelForm}>Cancel</button>
      </div>
    </div>
  {/if}

  {#each items as h (h.keyword)}
    {#if editKw === h.keyword}
      <div class="card inline-form">
        <label>
          <span class="label-text">Keyword</span>
          <input type="text" bind:value={formKw}
            onkeydown={(e) => { if (e.key === "Enter") saveEdit(); }} />
        </label>
        <label>
          <span class="label-text">Color</span>
          <div class="color-row">
            <input type="color" bind:value={formColor} />
            <span class="color-hex">{formColor}</span>
          </div>
        </label>
        <div class="form-actions">
          <button class="btn btn-accent btn-sm" onclick={saveEdit} disabled={!formKw.trim()}>Save</button>
          <button class="btn btn-sm" onclick={cancelForm}>Cancel</button>
        </div>
      </div>
    {:else}
      <div class="card item-row">
        <div class="item-info">
          <span class="color-swatch" style="background: {h.color}"></span>
          <div>
            <div class="item-name">{h.keyword}</div>
            <div class="item-sub">{h.color}</div>
          </div>
        </div>
        <div class="item-actions">
          <button class="btn btn-sm" onclick={() => startEdit(h)}>Edit</button>
          <button class="btn btn-sm btn-danger" onclick={() => remove(h.keyword)}>Delete</button>
        </div>
      </div>
    {/if}
  {:else}
    {#if !adding}
      <p class="empty">No highlight rules</p>
    {/if}
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; gap: 8px; margin-bottom: 16px; }
  .item-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 16px;
    gap: 12px;
  }
  .item-info { display: flex; align-items: center; gap: 10px; min-width: 0; flex: 1; }
  .item-name { font-weight: 600; font-size: 14px; font-family: monospace; }
  .item-sub { font-size: 12px; color: var(--text-sub); font-family: monospace; }
  .item-actions { display: flex; gap: 10px; flex-shrink: 0; }
  .color-swatch {
    width: 20px; height: 20px; border-radius: 4px; flex-shrink: 0;
    border: 1px solid var(--divider);
  }

  .inline-form {
    display: flex; flex-direction: column; gap: 8px;
    padding: 14px; margin-bottom: 10px;
  }
  .inline-form label { display: flex; flex-direction: column; gap: 4px; }
  .label-text { font-size: 13px; color: var(--text); }
  .inline-form input[type="text"] {
    width: 100%; box-sizing: border-box; font: inherit; font-size: 13px;
  }
  .color-row { display: flex; align-items: center; gap: 10px; }
  .color-row input[type="color"] {
    width: 48px; height: 32px; padding: 2px;
    border: 1px solid var(--divider); border-radius: 4px;
    cursor: pointer; box-shadow: none;
  }
  .color-hex { font-size: 12px; font-family: monospace; color: var(--text-dim); }
  .form-actions { display: flex; gap: 10px; margin-top: 4px; }

  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
