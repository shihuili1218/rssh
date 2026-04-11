<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Snippet } from "../stores/app.svelte.ts";

  let items = $state<Snippet[]>([]);
  let newName = $state(""); let newCmd = $state("");

  onMount(async () => { items = await app.loadSnippets(); });

  async function add() {
    if (!newName || !newCmd) return;
    items.push({ name: newName, command: newCmd });
    await invoke("save_snippets", { snippets: items });
    newName = ""; newCmd = "";
  }
  async function remove(idx: number) {
    items.splice(idx, 1);
    await invoke("save_snippets", { snippets: [...items] });
  }
</script>

<div class="page">
  <div class="add-row">
    <input type="text" bind:value={newName} placeholder="名称" />
    <input type="text" bind:value={newCmd} placeholder="命令" style="flex:2" />
    <button class="btn btn-accent btn-sm" onclick={add}>添加</button>
  </div>
  {#each items as s, i (s.name + i)}
    <div class="card item-row">
      <div class="item-info">
        <div class="item-name">{s.name}</div>
        <div class="item-sub">{s.command}</div>
      </div>
      <button class="btn btn-sm btn-danger" onclick={() => remove(i)}>×</button>
    </div>
  {:else}
    <p class="empty">暂无命令片段</p>
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .add-row { display: flex; gap: 8px; margin-bottom: 12px; }
  .add-row input { flex: 1; }
  .item-row { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
  .item-name { font-weight: 600; font-size: 13px; }
  .item-sub { font-size: 12px; color: var(--text-sub); font-family: monospace; }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
