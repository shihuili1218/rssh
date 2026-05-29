<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Profile, Group } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";

  let profiles = $state<Profile[]>([]);
  let groups = $state<Group[]>([]);
  onMount(async () => {
    [profiles, groups] = await Promise.all([app.loadProfiles(), app.loadGroups()]);
  });

  // Grouped view: one section per group (ordered by sort_order) holding its
  // profiles, then an "Ungrouped" bucket for profiles whose group_id is null
  // or points to a deleted group. Empty groups are hidden — this is a display,
  // not a group editor. Derived, so delete/reload reflows automatically.
  interface Section { key: string; name: string; color: string | null; items: Profile[]; }
  let sections = $derived.by(() => {
    const known = new Set(groups.map((g) => g.id));
    const out: Section[] = [];
    for (const g of [...groups].sort((a, b) => a.sort_order - b.sort_order)) {
      const items = profiles.filter((p) => p.group_id === g.id);
      if (items.length) out.push({ key: g.id, name: g.name, color: g.color, items });
    }
    const ungrouped = profiles.filter((p) => !p.group_id || !known.has(p.group_id));
    if (ungrouped.length) out.push({ key: "__ungrouped__", name: t("profile.ungrouped"), color: null, items: ungrouped });
    return out;
  });

  let deleting = $state<string | null>(null);
  async function remove(id: string) {
    deleting = id;
    try {
      await invoke("delete_profile", { id });
      profiles = await app.loadProfiles();
    } catch (e: any) { toast.error(`${t("toast.error.delete")}: ${errMsg(e)}`); }
    finally { deleting = null; }
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-accent btn-sm" onclick={() => app.navigate("profile-edit")}>{t("profile.new")}</button>
  </div>
  {#each sections as sec (sec.key)}
    <div class="group-head">
      {#if sec.color}<span class="dot" style="background:{sec.color}"></span>{/if}
      <span class="group-name">{sec.name}</span>
      <span class="group-count">{sec.items.length}</span>
    </div>
    {#each sec.items as p (p.id)}
      <div class="card item-row">
        <div class="item-info">
          <div class="item-name">{p.name}</div>
          <div class="item-sub">{p.host}:{p.port}</div>
        </div>
        <div class="item-actions">
          <button class="btn btn-sm" onclick={() => app.copyProfile(p.id)}>{t("common.copy")}</button>
          <button class="btn btn-sm" onclick={() => app.navigate("profile-edit", p.id)}>{t("common.edit")}</button>
          <button class="btn btn-sm btn-danger" onclick={() => remove(p.id)} disabled={deleting === p.id}>
            {deleting === p.id ? "..." : t("common.delete")}
          </button>
        </div>
      </div>
    {/each}
  {:else}
    <p class="empty">{t("profile.empty")}</p>
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 16px; }
  .group-head {
    display: flex; align-items: center; gap: 8px;
    margin: 20px 0 10px; padding-bottom: 6px;
    border-bottom: 1px solid var(--divider);
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
  .item-actions { display: flex; gap: 10px; }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
