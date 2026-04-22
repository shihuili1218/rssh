<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Forward, Profile } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t } from "../i18n/index.svelte.ts";

  let items = $state<Forward[]>([]);
  let profiles = $state<Profile[]>([]);

  onMount(async () => {
    [items, profiles] = await Promise.all([app.loadForwards(), app.loadProfiles()]);
  });

  function profileName(f: Forward): string {
    return profiles.find(p => p.id === f.profile_id)?.name ?? "?";
  }

  let deleting = $state<string | null>(null);
  async function remove(id: string) {
    deleting = id;
    try {
      await invoke("delete_forward", { id });
      items = await app.loadForwards();
    } catch (e: any) { toast.error(`${t("toast.error.delete")}: ${String(e)}`); }
    finally { deleting = null; }
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-accent btn-sm" onclick={() => app.navigate("forward-edit")}>+ New</button>
  </div>
  {#each items as f (f.id)}
    <div class="card item-row">
      <div class="item-info">
        <div class="item-name">{f.name}</div>
        <div class="item-sub">
          {f.type === "local" ? "L" : "R"} :{f.local_port} → {f.remote_host}:{f.remote_port}
          <span class="item-via">via {profileName(f)}</span>
        </div>
      </div>
      <div class="item-actions">
        <button class="btn btn-sm" onclick={() => app.navigate("forward-edit", f.id)}>Edit</button>
        <button class="btn btn-sm btn-danger" onclick={() => remove(f.id)} disabled={deleting === f.id}>
          {deleting === f.id ? "..." : "Delete"}
        </button>
      </div>
    </div>
  {:else}
    <p class="empty">No port forward rules</p>
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 16px; }
  .item-row { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; }
  .item-name { font-weight: 600; font-size: 14px; }
  .item-sub { font-size: 12px; color: var(--text-sub); font-family: monospace; }
  .item-via { color: var(--text-dim); font-family: inherit; }
  .item-actions { display: flex; gap: 6px; }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
