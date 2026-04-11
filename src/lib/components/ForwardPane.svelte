<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";

  let { tabId, meta = {} }: {
    tabId: string;
    meta: Record<string, string>;
  } = $props();

  let status = $state<"connecting" | "active" | "error" | "stopped">("connecting");
  let activeId = $state<string | null>(null);
  let errorMsg = $state("");

  onMount(connect);

  async function connect() {
    status = "connecting";
    errorMsg = "";
    try {
      activeId = await invoke<string>("forward_start", {
        forwardId: meta.forwardId,
      });
      status = "active";
    } catch (e: any) {
      status = "error";
      errorMsg = String(e);
    }
  }

  async function stop() {
    if (!activeId) return;
    try {
      await invoke("forward_stop", { activeId });
      status = "stopped";
      activeId = null;
    } catch (e: any) { errorMsg = String(e); }
  }

  function handleKeydown(e: KeyboardEvent) {
    if ((status === "error" || status === "stopped") && e.key.length === 1) {
      connect();
    }
  }

  onDestroy(() => {
    if (activeId) invoke("forward_stop", { activeId }).catch(() => {});
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="forward-pane">
  <div class="forward-card neu-raised">
    <h3>{meta.name ?? "Port Forward"}</h3>
    <div class="detail">
      {meta.forwardType === "remote" ? "R" : "L"} :{meta.localPort} → {meta.remoteHost}:{meta.remotePort}
    </div>
    <div class="detail">via {meta.profileName ?? meta.host}</div>

    <div class="status-row">
      {#if status === "connecting"}
        <span class="badge connecting">连接中...</span>
      {:else if status === "active"}
        <span class="badge active">运行中</span>
      {:else if status === "error"}
        <span class="badge error">错误</span>
      {:else}
        <span class="badge stopped">已停止</span>
      {/if}
    </div>

    {#if errorMsg}
      <div class="error-msg">{errorMsg}</div>
    {/if}

    <div class="actions">
      {#if status === "active"}
        <button class="btn btn-danger btn-sm" onclick={stop}>停止</button>
      {:else if status === "error" || status === "stopped"}
        <button class="btn btn-accent btn-sm" onclick={connect}>重连</button>
        <div class="hint">Press any key to reconnect</div>
      {/if}
    </div>
  </div>
</div>

<style>
  .forward-pane {
    height: 100%; display: flex; align-items: center; justify-content: center;
    padding: 24px;
  }
  .forward-card {
    padding: 32px; text-align: center; min-width: 300px; max-width: 400px;
  }
  .forward-card h3 { font-size: 18px; color: var(--text); margin-bottom: 8px; }
  .detail { font-size: 13px; color: var(--text-sub); font-family: monospace; margin-bottom: 4px; }
  .status-row { margin: 16px 0; }
  .badge {
    display: inline-block; padding: 4px 12px; border-radius: 12px;
    font-size: 12px; font-weight: 600;
  }
  .badge.connecting { background: rgba(74,108,247,0.15); color: var(--accent); }
  .badge.active { background: rgba(61,154,114,0.15); color: var(--success); }
  .badge.error { background: rgba(214,68,68,0.15); color: var(--error); }
  .badge.stopped { background: rgba(156,168,187,0.15); color: var(--text-dim); }
  .error-msg { font-size: 12px; color: var(--error); margin-bottom: 12px; word-break: break-all; }
  .actions { display: flex; flex-direction: column; align-items: center; gap: 8px; }
  .hint { font-size: 11px; color: var(--text-dim); }
</style>
