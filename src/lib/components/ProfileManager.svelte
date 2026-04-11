<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Profile } from "../stores/app.svelte.ts";

  let profiles = $state<Profile[]>([]);
  onMount(async () => { profiles = await app.loadProfiles(); });

  let deleting = $state<string | null>(null);
  async function remove(id: string) {
    deleting = id;
    try {
      await invoke("delete_profile", { id });
      profiles = await app.loadProfiles();
    } catch (e: any) { alert("删除失败: " + String(e)); }
    finally { deleting = null; }
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-accent btn-sm" onclick={() => app.navigate("profile-edit")}>+ 新建</button>
  </div>
  {#each profiles as p (p.id)}
    <div class="card item-row">
      <div class="item-info">
        <div class="item-name">{p.name}</div>
        <div class="item-sub">{p.host}:{p.port}</div>
      </div>
      <div class="item-actions">
        <button class="btn btn-sm" onclick={() => app.navigate("profile-edit", p.id)}>编辑</button>
        <button class="btn btn-sm btn-danger" onclick={() => remove(p.id)} disabled={deleting === p.id}>
          {deleting === p.id ? "..." : "删除"}
        </button>
      </div>
    </div>
  {:else}
    <p class="empty">暂无 Profile</p>
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 16px; }
  .item-row { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; }
  .item-name { font-weight: 600; font-size: 14px; }
  .item-sub { font-size: 12px; color: var(--text-sub); font-family: monospace; }
  .item-actions { display: flex; gap: 6px; }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
