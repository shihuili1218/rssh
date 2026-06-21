<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Forward, Profile, Group } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";

  let items = $state<Forward[]>([]);
  let profiles = $state<Profile[]>([]);
  let groups = $state<Group[]>([]);

  onMount(async () => {
    [items, profiles, groups] = await Promise.all([app.loadForwards(), app.loadProfiles(), app.loadGroups()]);
  });

  function profileName(f: Forward): string {
    return profiles.find((p) => p.id === f.profile_id)?.name ?? "?";
  }

  // Grouped view, mirroring ProfileManager: one section per group (sort_order),
  // then an "Ungrouped" bucket for forwards with no group or a deleted group.
  // Empty groups are hidden. Derived, so delete/reload reflows automatically.
  interface Section { key: string; name: string; color: string | null; items: Forward[]; }
  let sections = $derived.by(() => {
    const known = new Set(groups.map((g) => g.id));
    const out: Section[] = [];
    for (const g of [...groups].sort((a, b) => a.sort_order - b.sort_order)) {
      const its = items.filter((f) => f.group_id === g.id);
      if (its.length) out.push({ key: g.id, name: g.name, color: g.color, items: its });
    }
    const ungrouped = items.filter((f) => !f.group_id || !known.has(f.group_id));
    if (ungrouped.length) out.push({ key: "__ungrouped__", name: t("profile.ungrouped"), color: null, items: ungrouped });
    return out;
  });

  let deleting = $state<string | null>(null);
  async function remove(id: string) {
    deleting = id;
    try {
      await invoke("delete_forward", { id });
      items = await app.loadForwards();
    } catch (e: any) { toast.error(`${t("toast.error.delete")}: ${errMsg(e)}`); }
    finally { deleting = null; }
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-accent btn-sm" onclick={() => app.navigate("forward-edit")}>{t("forward.new")}</button>
  </div>
  {#each sections as sec (sec.key)}
    <div class="group-head">
      {#if sec.color}<span class="dot" style="background:{sec.color}"></span>{/if}
      <span class="group-name">{sec.name}</span>
      <span class="group-count">{sec.items.length}</span>
    </div>
    {#each sec.items as f (f.id)}
      <div class="card item-row">
        <div class="item-info">
          <div class="item-name">{f.name}</div>
          <div class="item-sub">
            {f.type === "local" ? "L" : "R"} :{f.local_port} → {f.remote_host}:{f.remote_port}
            <span class="item-via">via {profileName(f)}</span>
          </div>
        </div>
        <div class="item-actions">
          <button class="btn btn-sm" onclick={() => app.navigate("forward-edit", f.id)}>{t("common.edit")}</button>
          <button class="btn btn-sm btn-danger" onclick={() => remove(f.id)} disabled={deleting === f.id}>
            {deleting === f.id ? "..." : t("common.delete")}
          </button>
        </div>
      </div>
    {/each}
  {:else}
    <p class="empty">{t("forward.empty")}</p>
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 16px; }
  .group-head {
    display: flex; align-items: center; gap: 8px;
    margin: 20px 0 10px; padding-bottom: 6px;
  }
  .group-head:first-of-type { margin-top: 0; }
  .dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
  .group-name { font-weight: 600; font-size: 13px; color: var(--text); }
  .group-count {
    font-size: 11px; color: var(--text-sub);
    background: var(--surface); border-radius: 10px; padding: 1px 8px;
  }
  .item-row { display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px; }
  .item-name { font-weight: 600; font-size: 14px; }
  .item-sub { font-size: 12px; color: var(--text-sub); font-family: monospace; }
  .item-via { color: var(--text-dim); font-family: inherit; }
  .item-actions { display: flex; gap: 10px; }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
