<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";

  let importJson = $state("");
  let exportJson = $state("");
  let importing = $state(false);
  let sshConfig = $state("");
  let msg = $state("");

  async function doExport() {
    try {
      exportJson = await invoke<string>("export_config");
      msg = "Exported. Copy and save.";
    } catch (e: any) { msg = "Export failed: " + String(e); }
    setTimeout(() => msg = "", 3000);
  }

  async function doImport() {
    if (!importJson.trim()) return;
    if (!confirm("Confirm import? This will overwrite all local Profiles, Credentials, and Port Forwards!")) return;
    importing = true;
    try {
      await invoke("import_config", { json: importJson });
      importJson = "";
      msg = "Import successful";
    } catch (e: any) { msg = "Import failed: " + String(e); }
    finally { importing = false; }
    setTimeout(() => msg = "", 3000);
  }

  async function importSshConfig() {
    if (!sshConfig.trim()) return;
    try {
      const entries = await invoke<any[]>("import_ssh_config", { content: sshConfig });
      msg = `Parsed ${entries.length} Host entries`;
      sshConfig = "";
    } catch (e: any) { msg = "Parse failed: " + String(e); }
    setTimeout(() => msg = "", 3000);
  }
</script>

<div class="page">
  {#if msg}
    <div class="toast">{msg}</div>
  {/if}

  <!-- Export -->
  <div class="action-card neu-raised">
    <div class="action-info">
      <div class="action-title">Export Config</div>
      <div class="action-desc">Export all Profiles, Credentials, and Port Forwards as JSON</div>
    </div>
    <button class="btn btn-accent btn-sm" onclick={doExport}>Export</button>
  </div>
  {#if exportJson}
    <textarea class="mono-area" readonly value={exportJson}
      onclick={(e) => (e.target as HTMLTextAreaElement).select()}></textarea>
  {/if}

  <!-- Import JSON -->
  <div class="action-card neu-raised">
    <div class="action-info">
      <div class="action-title">Import Config</div>
      <div class="action-desc">Paste previously exported JSON. This will overwrite all local data.</div>
    </div>
  </div>
  <textarea class="mono-area" bind:value={importJson} rows="4" placeholder="Paste JSON..."></textarea>
  <button class="btn btn-sm" onclick={doImport} disabled={importing || !importJson.trim()}>
    {importing ? "Importing..." : "Confirm Import"}
  </button>

  <div class="divider"></div>

  <!-- SSH Config Import -->
  <div class="action-card neu-raised">
    <div class="action-info">
      <div class="action-title">Import SSH Config</div>
      <div class="action-desc">Paste ~/.ssh/config contents to auto-parse Host entries into Profiles</div>
    </div>
  </div>
  <textarea class="mono-area" bind:value={sshConfig} rows="6" placeholder="Host myserver&#10;  HostName 192.168.1.1&#10;  User root&#10;  Port 22"></textarea>
  <button class="btn btn-sm" onclick={importSshConfig} disabled={!sshConfig.trim()}>Parse & Import</button>
</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 12px; }
  .action-card {
    display: flex; align-items: center; justify-content: space-between;
    padding: 14px 16px; gap: 12px;
  }
  .action-title { font-size: 14px; font-weight: 600; color: var(--text); margin-bottom: 2px; }
  .action-desc { font-size: 12px; color: var(--text-sub); line-height: 1.4; }
  .mono-area {
    font-family: monospace; font-size: 11px;
    resize: vertical; min-height: 60px;
  }
  .toast {
    padding: 8px 14px;
    background: var(--accent-soft);
    color: var(--accent);
    border-radius: var(--radius-sm);
    font-size: 12px;
    font-weight: 500;
  }
</style>
