<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { TelnetProfile, Group } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";

  let items = $state<TelnetProfile[]>([]);
  let groups = $state<Group[]>([]);

  onMount(async () => {
    [items, groups] = await Promise.all([app.loadTelnetProfiles(), app.loadGroups()]);
  });

  // Grouped view, mirroring SerialProfileManager: one section per group
  // (sort_order), then an "Ungrouped" bucket for profiles with no group or a
  // deleted group. Empty groups hidden. Derived, so delete/reload reflows.
  interface Section { key: string; name: string; color: string | null; items: TelnetProfile[]; }
  let sections = $derived.by(() => {
    const known = new Set(groups.map((g) => g.id));
    const out: Section[] = [];
    for (const g of [...groups].sort((a, b) => a.sort_order - b.sort_order)) {
      const its = items.filter((s) => s.group_id === g.id);
      if (its.length) out.push({ key: g.id, name: g.name, color: g.color, items: its });
    }
    const ungrouped = items.filter((s) => !s.group_id || !known.has(s.group_id));
    if (ungrouped.length) out.push({ key: "__ungrouped__", name: t("profile.ungrouped"), color: null, items: ungrouped });
    return out;
  });

  let deleting = $state<string | null>(null);
  async function remove(id: string) {
    deleting = id;
    try {
      await invoke("delete_telnet_profile", { id });
      items = await app.loadTelnetProfiles();
    } catch (e: any) { toast.error(`${t("toast.error.delete")}: ${errMsg(e)}`); }
    finally { deleting = null; }
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-accent btn-sm" onclick={() => app.navigate("telnet-profile-edit")}>{t("telnet.new")}</button>
  </div>
  {#each sections as sec (sec.key)}
    <div class="group-head">
      {#if sec.color}<span class="dot" style="background:{sec.color}"></span>{/if}
      <span class="group-name">{sec.name}</span>
      <span class="group-count">{sec.items.length}</span>
    </div>
    {#each sec.items as s (s.id)}
      <div class="card item-row">
        <div class="item-info">
          <div class="item-name">{s.name}</div>
          <div class="item-sub">{s.host}:{s.port}</div>
        </div>
        <div class="item-actions">
          <button class="btn btn-sm" onclick={() => app.navigate("telnet-profile-edit", s.id)}>{t("common.edit")}</button>
          <button class="btn btn-sm btn-danger" onclick={() => remove(s.id)} disabled={deleting === s.id}>
            {deleting === s.id ? "..." : t("common.delete")}
          </button>
        </div>
      </div>
    {/each}
  {:else}
    <p class="empty">{t("telnet.empty")}</p>
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 16px; }
  .group-head {
    display: flex; align-items: center; gap: 8px;
    margin: 20px 0 10px; padding-bottom: 6px;
  }
  /* :first-of-type never matched (the toolbar div is the first of the type);
     the first section head always sits right after the toolbar. */
  .toolbar + .group-head { margin-top: 0; }
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
