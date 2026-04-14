<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  let { tabId, meta = {} }: {
    tabId: string;
    meta: Record<string, string>;
  } = $props();

  let status = $state<"connecting" | "active" | "error" | "stopped">("connecting");
  let activeId = $state<string | null>(null);
  let errorMsg = $state("");
  let bytesTx = $state(0);
  let bytesRx = $state(0);
  let connections = $state(0);
  let pollTimer = 0;

  function formatBytes(b: number): string {
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
    if (b < 1024 * 1024 * 1024) return `${(b / 1024 / 1024).toFixed(1)} MB`;
    return `${(b / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  async function pollStats() {
    if (!activeId || status !== "active") return;
    try {
      const s = await invoke<{ bytes_tx: number; bytes_rx: number; connections: number }>(
        "forward_stats", { activeId }
      );
      bytesTx = s.bytes_tx;
      bytesRx = s.bytes_rx;
      connections = s.connections;
    } catch { /* forward may have stopped */ }
  }

  function startPolling() {
    stopPolling();
    pollTimer = window.setInterval(pollStats, 2000);
  }

  function stopPolling() {
    if (pollTimer) { clearInterval(pollTimer); pollTimer = 0; }
  }

  onMount(connect);

  async function connect() {
    status = "connecting";
    errorMsg = "";
    bytesTx = 0; bytesRx = 0; connections = 0;
    try {
      activeId = await invoke<string>("forward_start", { forwardId: meta.forwardId });
      status = "active";
      startPolling();
    } catch (e: any) {
      status = "error";
      errorMsg = String(e);
    }
  }

  async function stop() {
    if (!activeId) return;
    stopPolling();
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
    stopPolling();
    if (activeId) invoke("forward_stop", { activeId }).catch(() => {});
  });

  const isRemote = $derived(meta.forwardType === "remote");
  const dirLabel = $derived(isRemote ? "Remote" : "Local");
  const arrow = $derived(isRemote
    ? `remote:${meta.localPort} \u2192 localhost:${meta.remotePort}`
    : `localhost:${meta.localPort} \u2192 ${meta.remoteHost}:${meta.remotePort}`);
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="forward-pane">
  <div class="card">
    <div class="header">
      <span class="type-badge" class:remote={isRemote}>{dirLabel}</span>
      <h3>{meta.name ?? "Port Forward"}</h3>
    </div>

    <div class="route">{arrow}</div>
    <div class="via">via {meta.profileName ?? meta.host}</div>

    <div class="status-area">
      {#if status === "connecting"}
        <span class="indicator connecting"></span> <span class="status-text">Connecting...</span>
      {:else if status === "active"}
        <span class="indicator active"></span> <span class="status-text">Active</span>
      {:else if status === "error"}
        <span class="indicator error"></span> <span class="status-text">Error</span>
      {:else}
        <span class="indicator stopped"></span> <span class="status-text">Stopped</span>
      {/if}
    </div>

    {#if status === "active"}
      <div class="stats">
        <div class="stat">
          <span class="stat-label">Connections</span>
          <span class="stat-value">{connections}</span>
        </div>
        <div class="stat">
          <span class="stat-label">TX</span>
          <span class="stat-value">{formatBytes(bytesTx)}</span>
        </div>
        <div class="stat">
          <span class="stat-label">RX</span>
          <span class="stat-value">{formatBytes(bytesRx)}</span>
        </div>
      </div>
    {/if}

    {#if errorMsg}
      <div class="error-msg">{errorMsg}</div>
    {/if}

    <div class="actions">
      {#if status === "active"}
        <button class="btn-stop" onclick={stop}>Stop</button>
      {:else if status === "error" || status === "stopped"}
        <button class="btn-reconnect" onclick={connect}>Reconnect</button>
        <div class="hint">Press any key to reconnect</div>
      {/if}
    </div>
  </div>
</div>

<style>
  .forward-pane {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }

  .card {
    background: var(--surface);
    border: 1px solid var(--divider);
    border-radius: 12px;
    padding: 28px 32px;
    min-width: 340px;
    max-width: 420px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .header {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .header h3 {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
    margin: 0;
  }

  .type-badge {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    padding: 2px 8px;
    border-radius: 4px;
    background: rgba(74,108,247,0.15);
    color: var(--accent);
    flex-shrink: 0;
  }

  .type-badge.remote {
    background: rgba(155,114,228,0.15);
    color: var(--magenta, #9B72E4);
  }

  .route {
    font-family: monospace;
    font-size: 13px;
    color: var(--text);
    background: var(--bg);
    padding: 8px 12px;
    border-radius: 6px;
    text-align: center;
  }

  .via {
    font-size: 12px;
    color: var(--text-sub);
    text-align: center;
  }

  .status-area {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 4px 0;
  }

  .indicator {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .indicator.connecting { background: var(--accent); animation: pulse 1.2s infinite; }
  .indicator.active { background: var(--success); }
  .indicator.error { background: var(--error); }
  .indicator.stopped { background: var(--text-dim); }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }

  .status-text {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-sub);
  }

  .stats {
    display: flex;
    gap: 4px;
  }

  .stat {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 4px;
    background: var(--bg);
    border-radius: 6px;
  }

  .stat-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-dim);
  }

  .stat-value {
    font-family: monospace;
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }

  .error-msg {
    font-size: 12px;
    color: var(--error);
    word-break: break-all;
    text-align: center;
  }

  .actions {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding-top: 4px;
  }

  .btn-stop, .btn-reconnect {
    padding: 6px 20px;
    border-radius: 6px;
    border: none;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }

  .btn-stop {
    background: rgba(214,68,68,0.15);
    color: var(--error);
  }

  .btn-stop:hover {
    background: rgba(214,68,68,0.25);
  }

  .btn-reconnect {
    background: var(--accent);
    color: var(--bg);
  }

  .hint {
    font-size: 11px;
    color: var(--text-dim);
  }
</style>
