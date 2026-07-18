<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type {
    ConnectionKind,
    Forward,
    Group,
    Profile,
    SerialProfile,
    TelnetProfile,
  } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { errMsg, t } from "../i18n/index.svelte.ts";
  import {
    buildConnectionItems,
    groupConnectionItems,
    type ConnectionListItem,
  } from "./connection-list.ts";
  import AppIcon from "./AppIcon.svelte";
  import { connectionIconName } from "./app-icon";

  let profiles = $state<Profile[]>([]);
  let forwards = $state<Forward[]>([]);
  let serialProfiles = $state<SerialProfile[]>([]);
  let telnetProfiles = $state<TelnetProfile[]>([]);
  let groups = $state<Group[]>([]);
  let loading = $state(true);
  let deletingKey = $state<string | null>(null);

  let items = $derived(buildConnectionItems({ profiles, forwards, serialProfiles, telnetProfiles }));
  let sections = $derived(groupConnectionItems(groups, items, t("profile.ungrouped")));

  onMount(refresh);

  async function loadOrEmpty<T>(loader: () => Promise<T[]>): Promise<T[]> {
    try {
      return await loader();
    } catch (error) {
      toast.error(errMsg(error));
      return [];
    }
  }

  async function refresh() {
    loading = true;
    [profiles, forwards, serialProfiles, telnetProfiles, groups] = await Promise.all([
      loadOrEmpty(app.loadProfiles),
      loadOrEmpty(app.loadForwards),
      app.isMobile ? Promise.resolve([]) : loadOrEmpty(app.loadSerialProfiles),
      loadOrEmpty(app.loadTelnetProfiles),
      loadOrEmpty(app.loadGroups),
    ]);
    loading = false;
  }

  function itemKey(item: ConnectionListItem): string {
    return `${item.kind}:${item.id}`;
  }

  function kindLabel(kind: ConnectionKind): string {
    switch (kind) {
      case "ssh": return t("connection.type.ssh");
      case "forward": return t("connection.type.forward");
      case "serial": return t("connection.type.serial");
      case "telnet": return t("connection.type.telnet");
    }
  }

  function deleteCommand(kind: ConnectionKind): string {
    switch (kind) {
      case "ssh": return "delete_profile";
      case "forward": return "delete_forward";
      case "serial": return "delete_serial_profile";
      case "telnet": return "delete_telnet_profile";
    }
  }

  async function remove(item: ConnectionListItem) {
    const key = itemKey(item);
    deletingKey = key;
    try {
      await invoke(deleteCommand(item.kind), { id: item.id });
      await refresh();
    } catch (error) {
      toast.error(`${t("toast.error.delete")}: ${errMsg(error)}`);
    } finally {
      deletingKey = null;
    }
  }
</script>

<div class="page" aria-busy={loading}>
  <div class="toolbar">
    <button type="button" class="btn btn-accent btn-sm" onclick={() => app.openConnectionCreate()}>
      {t("connection.new")}
    </button>
  </div>

  {#if loading}
    <p class="empty" aria-live="polite">{t("common.loading")}</p>
  {:else}
    {#each sections as section (section.key)}
      <div class="group-head">
        {#if section.color}<span class="dot" style:background={section.color} aria-hidden="true"></span>{/if}
        <span class="group-name">{section.name}</span>
        <span class="group-count">{section.items.length}</span>
      </div>
      {#each section.items as item (itemKey(item))}
        <div class="card item-row">
          <div class="item-main">
            <span class="item-icon"><AppIcon name={connectionIconName(item.kind)} size={18} /></span>
            <div class="item-info">
              <div class="item-title-row">
                <span class="item-name">{item.name}</span>
                <span class="item-kind">{kindLabel(item.kind)}</span>
              </div>
              <div class="item-sub">{item.detail}</div>
            </div>
          </div>
          <div class="item-actions">
            <button
              type="button"
              class="btn btn-sm"
              aria-label={`${t("common.copy")} ${kindLabel(item.kind)} ${item.name}`}
              onclick={() => app.openConnectionCopy(item.kind, item.id)}
            >
              {t("common.copy")}
            </button>
            <button
              type="button"
              class="btn btn-sm"
              aria-label={`${t("common.edit")} ${kindLabel(item.kind)} ${item.name}`}
              onclick={() => app.openConnectionEdit(item.kind, item.id)}
            >
              {t("common.edit")}
            </button>
            <button
              type="button"
              class="btn btn-sm btn-danger"
              aria-label={`${t("common.delete")} ${kindLabel(item.kind)} ${item.name}`}
              aria-busy={deletingKey === itemKey(item)}
              onclick={() => remove(item)}
              disabled={deletingKey === itemKey(item)}
            >
              {deletingKey === itemKey(item) ? "…" : t("common.delete")}
            </button>
          </div>
        </div>
      {/each}
    {:else}
      <p class="empty">{t("connection.empty")}</p>
    {/each}
  {/if}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 16px; }
  .group-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 20px 0 10px;
    padding-bottom: 6px;
  }
  .toolbar + .group-head { margin-top: 0; }
  .dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; }
  .group-name { color: var(--text); font-size: 13px; font-weight: 600; }
  .group-count {
    padding: 1px 8px;
    border-radius: 10px;
    background: var(--surface);
    color: var(--text-sub);
    font-size: 11px;
  }
  .item-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 16px;
  }
  .item-main { display: flex; align-items: center; gap: 12px; min-width: 0; }
  .item-icon {
    display: grid;
    place-items: center;
    flex: 0 0 auto;
    width: 32px;
    height: 32px;
    border-radius: 7px;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 12px;
    font-weight: 700;
  }
  .item-info { min-width: 0; }
  .item-title-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .item-name { overflow: hidden; font-size: 14px; font-weight: 600; text-overflow: ellipsis; white-space: nowrap; }
  .item-kind {
    flex-shrink: 0;
    padding: 1px 6px;
    border: 1px solid var(--divider);
    border-radius: 999px;
    color: var(--text-sub);
    font-size: 10px;
  }
  .item-sub {
    overflow: hidden;
    color: var(--text-sub);
    font-family: monospace;
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item-actions { display: flex; flex: 0 0 auto; gap: 10px; }
  .empty { padding: 32px; color: var(--text-dim); text-align: center; }

  @media (max-width: 720px) {
    .item-row { align-items: flex-start; flex-direction: column; }
    .item-actions { align-self: flex-end; flex-wrap: wrap; justify-content: flex-end; }
  }
</style>
