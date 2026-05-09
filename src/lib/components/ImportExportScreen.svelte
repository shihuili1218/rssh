<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";

  let importing = $state(false);
  let msg = $state("");

  // 多行错误（如 import_partial_failed 列每条失败）需要更久才看得完。
  function clearMsgLater() {
    const ttl = msg.includes("\n") ? 15000 : 4000;
    setTimeout(() => (msg = ""), ttl);
  }

  async function doExport() {
    try {
      const path = await invoke<string | null>("export_config_to_file");
      msg = path ? `Exported to ${path}` : "";
    } catch (e: any) { msg = `${t("toast.error.export")}: ${errMsg(e)}`; }
    clearMsgLater();
  }

  async function doImport() {
    importing = true;
    try {
      const path = await invoke<string | null>("import_config_from_file");
      msg = path ? `Imported from ${path}` : "";
    } catch (e: any) { msg = `${t("toast.error.import")}: ${errMsg(e)}`; }
    finally { importing = false; }
    clearMsgLater();
  }

  function gotoSshImport() {
    app.settingsNavigate("import-ssh-config");
  }
</script>

<div class="page">
  {#if msg}
    <div class="toast">{msg}</div>
  {/if}

  <div class="action-card surface-raised">
    <div class="action-info">
      <div class="action-title">Export Config</div>
      <div class="action-desc">Save all Profiles, Credentials, and Port Forwards to a JSON file</div>
    </div>
    <button class="btn btn-accent btn-sm" onclick={doExport}>Export</button>
  </div>

  <div class="action-card surface-raised">
    <div class="action-info">
      <div class="action-title">Import Config</div>
      <div class="action-desc">Load a previously exported JSON file. Merges into local config: matching entries (by id) are overwritten, others stay.</div>
    </div>
    <button class="btn btn-sm" onclick={doImport} disabled={importing}>
      {importing ? "Importing..." : "Import"}
    </button>
  </div>

  <div class="action-card surface-raised">
    <div class="action-info">
      <div class="action-title">Import SSH Config</div>
      <div class="action-desc">Read ~/.ssh/config and pick which Host entries to import as Profiles</div>
    </div>
    <button class="btn btn-sm" onclick={gotoSshImport}>Open</button>
  </div>
</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 12px; }
  .action-card {
    display: flex; align-items: center; justify-content: space-between;
    padding: 14px 16px; gap: 12px;
  }
  .action-title { font-size: 14px; font-weight: 600; color: var(--text); margin-bottom: 2px; }
  .action-desc { font-size: 12px; color: var(--text-sub); line-height: 1.4; }
  .toast {
    padding: 8px 14px;
    background: var(--accent-soft);
    color: var(--accent);
    border-radius: var(--radius-sm);
    font-size: 12px;
    font-weight: 500;
    white-space: pre-line; /* 让 import_partial_failed 等多行错误的 \n 真正换行 */
  }
</style>
