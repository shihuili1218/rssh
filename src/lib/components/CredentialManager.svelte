<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Credential } from "../stores/app.svelte.ts";

  let items = $state<Credential[]>([]);
  onMount(async () => { items = await app.loadCredentials(); });
  let deleting = $state<string | null>(null);
  async function remove(id: string) {
    deleting = id;
    try {
      await invoke("delete_credential", { id });
      items = await app.loadCredentials();
    } catch (e: any) { alert("删除失败: " + String(e)); }
    finally { deleting = null; }
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-accent btn-sm" onclick={() => app.navigate("credential-edit")}>+ 新建</button>
  </div>
  {#each items as c (c.id)}
    <div class="card item-row">
      <div class="item-info">
        <div class="item-name">{c.name}</div>
        <div class="item-sub">{c.username} · {c.type}</div>
      </div>
      <div class="item-actions">
        <button class="btn btn-sm" onclick={() => app.navigate("credential-edit", c.id)}>编辑</button>
        <button class="btn btn-sm btn-danger" onclick={() => remove(c.id)} disabled={deleting === c.id}>
          {deleting === c.id ? "..." : "删除"}
        </button>
      </div>
    </div>
  {:else}
    <p class="empty">暂无凭证</p>
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 16px; }
  .item-row { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; }
  .item-name { font-weight: 600; font-size: 14px; }
  .item-sub { font-size: 12px; color: var(--text-sub); }
  .item-actions { display: flex; gap: 6px; }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
