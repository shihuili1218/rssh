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
      msg = "已导出，可复制保存";
    } catch (e: any) { msg = "导出失败: " + String(e); }
    setTimeout(() => msg = "", 3000);
  }

  async function doImport() {
    if (!importJson.trim()) return;
    if (!confirm("确认导入？这会覆盖本地所有 Profiles、凭证和端口转发！")) return;
    importing = true;
    try {
      await invoke("import_config", { json: importJson });
      importJson = "";
      msg = "导入成功";
    } catch (e: any) { msg = "导入失败: " + String(e); }
    finally { importing = false; }
    setTimeout(() => msg = "", 3000);
  }

  async function importSshConfig() {
    if (!sshConfig.trim()) return;
    try {
      const entries = await invoke<any[]>("import_ssh_config", { content: sshConfig });
      msg = `解析到 ${entries.length} 个 Host 条目`;
      sshConfig = "";
    } catch (e: any) { msg = "解析失败: " + String(e); }
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
      <div class="action-title">导出配置</div>
      <div class="action-desc">将所有 Profiles、凭证和端口转发导出为 JSON 文本</div>
    </div>
    <button class="btn btn-accent btn-sm" onclick={doExport}>导出</button>
  </div>
  {#if exportJson}
    <textarea class="mono-area" readonly value={exportJson}
      onclick={(e) => (e.target as HTMLTextAreaElement).select()}></textarea>
  {/if}

  <!-- Import JSON -->
  <div class="action-card neu-raised">
    <div class="action-info">
      <div class="action-title">导入配置</div>
      <div class="action-desc">粘贴之前导出的 JSON，将覆盖本地所有数据</div>
    </div>
  </div>
  <textarea class="mono-area" bind:value={importJson} rows="4" placeholder="粘贴 JSON..."></textarea>
  <button class="btn btn-sm" onclick={doImport} disabled={importing || !importJson.trim()}>
    {importing ? "导入中..." : "确认导入"}
  </button>

  <div class="divider"></div>

  <!-- SSH Config Import -->
  <div class="action-card neu-raised">
    <div class="action-info">
      <div class="action-title">导入 SSH Config</div>
      <div class="action-desc">粘贴 ~/.ssh/config 内容，自动解析 Host 条目为 Profiles</div>
    </div>
  </div>
  <textarea class="mono-area" bind:value={sshConfig} rows="6" placeholder="Host myserver&#10;  HostName 192.168.1.1&#10;  User root&#10;  Port 22"></textarea>
  <button class="btn btn-sm" onclick={importSshConfig} disabled={!sshConfig.trim()}>解析并导入</button>
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
