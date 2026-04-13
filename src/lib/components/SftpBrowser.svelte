<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { RemoteEntry } from "../stores/app.svelte.ts";

  let { meta }: { meta: Record<string, string> } = $props();

  let sftpId = $state<string | null>(null);
  let cwd = $state("/");
  let entries = $state<RemoteEntry[]>([]);
  let loading = $state(true);
  let error = $state("");

  onMount(async () => {
    try {
      const id = await invoke<string>("sftp_connect", {
        host: meta.host, port: Number(meta.port),
        username: meta.username, authType: meta.authType, secret: meta.secret || null,
      });
      sftpId = id;
      const home = await invoke<string>("sftp_home", { sftpId: id });
      cwd = home;
      await listDir(home);
    } catch (e: any) {
      error = String(e);
      loading = false;
    }
  });

  onDestroy(() => { if (sftpId) invoke("sftp_close", { sftpId }); });

  async function listDir(path: string) {
    loading = true; error = "";
    try {
      entries = await invoke<RemoteEntry[]>("sftp_list", { sftpId, path });
      cwd = path;
    } catch (e: any) { error = String(e); }
    loading = false;
  }

  function goUp() {
    const parent = cwd.replace(/\/[^/]+\/?$/, "") || "/";
    listDir(parent);
  }

  function openEntry(e: RemoteEntry) {
    if (e.is_dir) listDir(cwd === "/" ? `/${e.name}` : `${cwd}/${e.name}`);
  }

  async function download(e: RemoteEntry) {
    try {
      const path = cwd === "/" ? `/${e.name}` : `${cwd}/${e.name}`;
      const data = await invoke<number[]>("sftp_download", { sftpId, path });
      const blob = new Blob([new Uint8Array(data)]);
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url; a.download = e.name; a.click();
      URL.revokeObjectURL(url);
    } catch (e: any) { error = String(e); }
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} K`;
    if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1)} M`;
    return `${(bytes / 1073741824).toFixed(1)} G`;
  }
</script>

<div class="sftp">
  <div class="header">
    <button class="btn btn-sm" onclick={() => app.navigate("main")}>← Terminal</button>
    <h2>SFTP</h2>
    <button class="btn btn-sm" onclick={goUp}>↑ Up</button>
    <button class="btn btn-sm" onclick={() => listDir(cwd)}>Refresh</button>
  </div>
  <div class="breadcrumb">{cwd}</div>

  {#if error}
    <div class="error-banner">{error}</div>
  {/if}

  {#if loading}
    <p class="loading">Loading...</p>
  {:else}
    <div class="file-list">
      {#each entries as e (e.name)}
        <div class="file-row" class:dir={e.is_dir}>
          <button class="file-name" onclick={() => openEntry(e)}>
            <span class="file-icon">{e.is_dir ? "📁" : "📄"}</span>
            {e.name}
          </button>
          <span class="file-size">{e.is_dir ? "" : formatSize(e.size)}</span>
          {#if !e.is_dir}
            <button class="btn btn-sm" onclick={() => download(e)}>Download</button>
          {/if}
        </div>
      {:else}
        <p class="empty">Empty directory</p>
      {/each}
    </div>
  {/if}
</div>

<style>
  .sftp { padding: 16px; max-width: 700px; margin: 0 auto; }
  .header { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; }
  .header h2 { flex: 1; font-size: 16px; }
  .breadcrumb {
    font-family: monospace; font-size: 12px; color: var(--text-sub);
    padding: 6px 10px; margin-bottom: 8px;
    background: var(--bg); box-shadow: var(--pressed); border-radius: var(--radius-sm);
  }
  .error-banner { background: rgba(214,68,68,0.1); border-left: 3px solid var(--error); color: var(--error); padding: 8px 12px; border-radius: var(--radius-sm); margin-bottom: 8px; font-size: 12px; }
  .loading { text-align: center; color: var(--text-dim); padding: 24px; }
  .file-list { display: flex; flex-direction: column; gap: 2px; }
  .file-row { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: var(--radius-sm); transition: background 0.1s; }
  .file-row:hover { background: rgba(163,177,198,0.15); }
  .file-name { flex: 1; border: none; background: none; text-align: left; font-family: inherit; font-size: 13px; color: var(--text); cursor: pointer; display: flex; align-items: center; gap: 6px; }
  .file-row.dir .file-name { font-weight: 600; color: var(--accent); }
  .file-icon { font-size: 14px; }
  .file-size { font-size: 11px; color: var(--text-dim); width: 60px; text-align: right; }
  .empty { text-align: center; color: var(--text-dim); padding: 24px; }
</style>
