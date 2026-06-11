<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { SerialProfile } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";

  let items = $state<SerialProfile[]>([]);

  onMount(async () => {
    items = await app.loadSerialProfiles();
  });

  /** Compact framing label, e.g. "8N1". parity[0] → N/O/E. */
  function framing(s: SerialProfile): string {
    const p = (s.parity[0] ?? "n").toUpperCase();
    return `${s.data_bits}${p}${s.stop_bits}`;
  }

  let deleting = $state<string | null>(null);
  async function remove(id: string) {
    deleting = id;
    try {
      await invoke("delete_serial_profile", { id });
      items = await app.loadSerialProfiles();
    } catch (e: any) { toast.error(`${t("toast.error.delete")}: ${errMsg(e)}`); }
    finally { deleting = null; }
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-accent btn-sm" onclick={() => app.navigate("serial-profile-edit")}>{t("serial.new")}</button>
  </div>
  {#each items as s (s.id)}
    <div class="card item-row">
      <div class="item-info">
        <div class="item-name">{s.name}</div>
        <div class="item-sub">{s.port} · {s.baud_rate} {framing(s)}</div>
      </div>
      <div class="item-actions">
        <button class="btn btn-sm" onclick={() => app.navigate("serial-profile-edit", s.id)}>{t("common.edit")}</button>
        <button class="btn btn-sm btn-danger" onclick={() => remove(s.id)} disabled={deleting === s.id}>
          {deleting === s.id ? "..." : t("common.delete")}
        </button>
      </div>
    </div>
  {:else}
    <p class="empty">{t("serial.empty")}</p>
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 16px; }
  .item-row { display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px; }
  .item-name { font-weight: 600; font-size: 14px; }
  .item-sub { font-size: 12px; color: var(--text-sub); font-family: monospace; }
  .item-actions { display: flex; gap: 10px; }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
