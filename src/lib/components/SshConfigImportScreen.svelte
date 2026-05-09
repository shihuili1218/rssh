<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";

  type SshConfigEntry = {
    host_alias: string;
    hostname: string;
    port: number;
    user: string | null;
    identity_file: string | null;
    proxy_jump: string | null;
  };

  type SshImportError = {
    host_alias: string;
    kind: string;
    code: string;
  };

  type SshImportResult = {
    profiles_created: number;
    credentials_created: number;
    errors: SshImportError[];
  };

  let entries = $state<SshConfigEntry[]>([]);
  let selected = $state<Set<number>>(new Set());
  let loading = $state(true);
  let importing = $state(false);
  let lastResult = $state<SshImportResult | null>(null);

  onMount(async () => {
    try {
      entries = await invoke<SshConfigEntry[]>("read_ssh_config_default");
      // 默认全选
      selected = new Set(entries.map((_, i) => i));
    } catch (e: any) {
      toast.error(`${t("toast.error.parse")}: ${errMsg(e)}`);
    } finally {
      loading = false;
    }
  });

  let allChecked = $derived(entries.length > 0 && selected.size === entries.length);
  let someChecked = $derived(selected.size > 0 && selected.size < entries.length);

  let headerCheckbox: HTMLInputElement | null = $state(null);
  // indeterminate 是 DOM property 不是 attribute，必须直接赋值才能触发视觉态。
  $effect(() => {
    if (headerCheckbox) headerCheckbox.indeterminate = someChecked;
  });

  function toggle(i: number) {
    const next = new Set(selected);
    if (next.has(i)) next.delete(i); else next.add(i);
    selected = next;
  }

  function toggleAll() {
    selected = allChecked ? new Set() : new Set(entries.map((_, i) => i));
  }

  async function doImport() {
    if (selected.size === 0) return;
    importing = true;
    lastResult = null;
    try {
      const picked = entries.filter((_, i) => selected.has(i));
      const result = await invoke<SshImportResult>("import_ssh_entries", { entries: picked });
      lastResult = result;
      if (result.errors.length === 0) {
        toast.success(`Imported ${result.profiles_created} profile(s), ${result.credentials_created} credential(s)`);
      } else {
        toast.error(`Imported with ${result.errors.length} error(s)`);
      }
    } catch (e: any) {
      toast.error(`${t("toast.error.import")}: ${errMsg(e)}`);
    } finally {
      importing = false;
    }
  }
</script>

<div class="page">
  <div class="header">
    <button class="btn btn-sm" onclick={() => app.settingsNavigate("import-export")}>← {t("common.back")}</button>
    <h2>Import SSH Config</h2>
  </div>

  {#if loading}
    <p class="empty">Loading ~/.ssh/config...</p>
  {:else if entries.length === 0}
    <p class="empty">No Host entries found in ~/.ssh/config</p>
  {:else}
    <div class="toolbar">
      <span class="count">{selected.size} / {entries.length} selected</span>
      <button class="btn btn-accent btn-sm" onclick={doImport} disabled={importing || selected.size === 0}>
        {importing ? "Importing..." : "Import Selected"}
      </button>
    </div>

    <div class="table surface-raised">
      <div class="row head">
        <div class="cell-check">
          <input
            bind:this={headerCheckbox}
            type="checkbox"
            checked={allChecked}
            onchange={toggleAll}
          />
        </div>
        <div class="cell-host">Host</div>
        <div class="cell-target">HostName:Port</div>
        <div class="cell-user">User</div>
        <div class="cell-key">IdentityFile</div>
      </div>
      {#each entries as e, i (i)}
        <label class="row" class:selected={selected.has(i)}>
          <div class="cell-check">
            <input type="checkbox" checked={selected.has(i)} onchange={() => toggle(i)} />
          </div>
          <div class="cell-host">{e.host_alias}</div>
          <div class="cell-target">{e.hostname || "—"}:{e.port}</div>
          <div class="cell-user">{e.user ?? "—"}</div>
          <div class="cell-key" title={e.identity_file ?? ""}>{e.identity_file ?? "—"}</div>
        </label>
      {/each}
    </div>

    {#if lastResult && lastResult.errors.length > 0}
      <div class="error-list surface-raised">
        <div class="error-title">Errors</div>
        {#each lastResult.errors as err}
          <div class="error-row">
            <span class="error-host">{err.host_alias}</span>
            <span class="error-kind">{err.kind}</span>
            <span class="error-code">{err.code}</span>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 12px; }
  .header { display: flex; align-items: center; gap: 12px; }
  .header h2 { font-size: 16px; font-weight: 600; margin: 0; }
  .toolbar { display: flex; justify-content: space-between; align-items: center; padding: 0 4px; }
  .count { font-size: 12px; color: var(--text-sub); }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }

  .table { display: flex; flex-direction: column; padding: 4px; border-radius: var(--radius-sm); }
  .row {
    display: grid;
    grid-template-columns: 32px 1.2fr 1.5fr 1fr 2fr;
    gap: 8px;
    padding: 8px 12px;
    align-items: center;
    font-size: 12px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .row:hover:not(.head) { background: var(--accent-soft); }
  .row.head { font-weight: 600; color: var(--text-sub); cursor: default; padding-bottom: 6px; border-bottom: 1px solid var(--divider); margin-bottom: 4px; }
  .row.head:hover { background: transparent; }
  .row.selected { background: var(--accent-soft); }
  .cell-check { display: flex; align-items: center; justify-content: center; }
  .cell-host { font-weight: 600; color: var(--text); }
  .cell-target, .cell-user, .cell-key { color: var(--text-sub); font-family: monospace; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .error-list { padding: 12px 16px; display: flex; flex-direction: column; gap: 4px; }
  .error-title { font-size: 13px; font-weight: 600; color: var(--text); margin-bottom: 6px; }
  .error-row { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; font-size: 11px; font-family: monospace; }
  .error-host { color: var(--text); }
  .error-kind { color: var(--text-sub); }
  .error-code { color: var(--danger, #c00); }
</style>
