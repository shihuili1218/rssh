<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import type { Snippet } from "../stores/app.svelte.ts";
  import * as app from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";

  let snippets = $state<Snippet[]>([]);
  let adding = $state(false);
  let editIdx = $state<number | null>(null);

  // Form fields
  let formName = $state("");
  let formCmd = $state("");

  onMount(refresh);

  async function refresh() {
    snippets = await app.loadSnippets();
  }

  function startAdd() {
    adding = true;
    editIdx = null;
    formName = "";
    formCmd = "";
  }

  function startEdit(idx: number) {
    adding = false;
    editIdx = idx;
    formName = snippets[idx].name;
    formCmd = snippets[idx].command;
  }

  function cancelForm() {
    adding = false;
    editIdx = null;
  }

  async function saveNew() {
    const name = formName.trim();
    const cmd = formCmd.trim();
    if (!name || !cmd) return;
    const next = [...snippets, { name, command: cmd }];
    try {
      await invoke("save_snippets", { snippets: next });
      adding = false;
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
  }

  async function saveEdit() {
    if (editIdx === null) return;
    const name = formName.trim();
    const cmd = formCmd.trim();
    if (!name || !cmd) return;
    const next = [...snippets];
    next[editIdx] = { name, command: cmd };
    try {
      await invoke("save_snippets", { snippets: next });
      editIdx = null;
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
  }

  async function remove(idx: number) {
    const next = snippets.filter((_, i) => i !== idx);
    try {
      await invoke("save_snippets", { snippets: next });
      if (editIdx === idx) editIdx = null;
      else if (editIdx !== null && editIdx > idx) editIdx -= 1;
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.delete")}: ${errMsg(e)}`); }
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-accent btn-sm" onclick={startAdd}>+ New Snippet</button>
  </div>

  {#if adding}
    <div class="card inline-form">
      <label>
        <span class="label-text">Name</span>
        <input type="text" bind:value={formName} placeholder="snippet name" />
      </label>
      <label>
        <span class="label-text">Command</span>
        <textarea bind:value={formCmd} placeholder="command (multi-line ok)" rows="2"></textarea>
      </label>
      <div class="form-actions">
        <button class="btn btn-accent btn-sm" onclick={saveNew} disabled={!formName.trim() || !formCmd.trim()}>Save</button>
        <button class="btn btn-sm" onclick={cancelForm}>Cancel</button>
      </div>
    </div>
  {/if}

  {#each snippets as s, i (i)}
    {#if editIdx === i}
      <div class="card inline-form">
        <label>
          <span class="label-text">Name</span>
          <input type="text" bind:value={formName} />
        </label>
        <label>
          <span class="label-text">Command</span>
          <textarea bind:value={formCmd} rows="2"></textarea>
        </label>
        <div class="form-actions">
          <button class="btn btn-accent btn-sm" onclick={saveEdit} disabled={!formName.trim() || !formCmd.trim()}>Save</button>
          <button class="btn btn-sm" onclick={cancelForm}>Cancel</button>
        </div>
      </div>
    {:else}
      <div class="card item-row">
        <div class="item-info">
          <div class="item-name">{s.name}</div>
          <div class="item-sub">{s.command}</div>
        </div>
        <div class="item-actions">
          <button class="btn btn-sm" onclick={() => startEdit(i)}>Edit</button>
          <button class="btn btn-sm btn-danger" onclick={() => remove(i)}>Delete</button>
        </div>
      </div>
    {/if}
  {:else}
    {#if !adding}
      <p class="empty">No command snippets</p>
    {/if}
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 16px; }
  .item-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 16px;
    gap: 12px;
  }
  .item-info { min-width: 0; flex: 1; }
  .item-name { font-weight: 600; font-size: 14px; }
  .item-sub {
    font-size: 12px;
    color: var(--text-sub);
    font-family: monospace;
    white-space: pre-wrap;
    word-break: break-all;
    margin-top: 2px;
  }
  .item-actions { display: flex; gap: 10px; flex-shrink: 0; }
  .inline-form {
    display: flex; flex-direction: column; gap: 8px;
    padding: 14px; margin-bottom: 10px;
  }
  .inline-form label { display: flex; flex-direction: column; gap: 4px; }
  .label-text { font-size: 13px; color: var(--text); }
  .inline-form input,
  .inline-form textarea {
    width: 100%; box-sizing: border-box; font: inherit; font-size: 13px;
  }
  .inline-form textarea {
    font-family: monospace; resize: vertical; min-height: 36px;
  }
  .form-actions { display: flex; gap: 10px; margin-top: 4px; }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
