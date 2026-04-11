<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  let shells = $state<string[]>([]);
  let selectedShell = $state("");
  let verboseLog = $state(true);

  onMount(async () => {
    try { shells = await invoke<string[]>("list_shells"); } catch { shells = []; }
    selectedShell = await invoke<string | null>("get_setting", { key: "local_shell" }) ?? "";
    verboseLog = (await invoke<string | null>("get_setting", { key: "verbose_log" })) !== "false";
  });

  async function saveShell() {
    await invoke("set_setting", { key: "local_shell", value: selectedShell });
  }

  async function saveVerbose() {
    await invoke("set_setting", { key: "verbose_log", value: String(verboseLog) });
  }
</script>

<div class="page">
  <div class="section-label">LOCAL SHELL</div>
  <div class="shell-list">
    {#each shells as sh}
      <button
        class="shell-option neu-sm"
        class:active={selectedShell === sh || (!selectedShell && shells[0] === sh)}
        onclick={() => { selectedShell = sh; saveShell(); }}
      >
        <span class="shell-name">{sh}</span>
        {#if selectedShell === sh || (!selectedShell && shells[0] === sh)}
          <span class="check">&#x2713;</span>
        {/if}
      </button>
    {/each}
    <div class="custom-shell">
      <label>自定义路径</label>
      <input type="text" bind:value={selectedShell} placeholder="/usr/local/bin/fish" onblur={saveShell} />
    </div>
  </div>

  <div class="section-label">CONNECTION LOGGING</div>
  <div class="switch-card">
    <div class="switch-card-body">
      <div class="switch-card-title" class:on={verboseLog} class:off={!verboseLog}>VERBOSE LOG</div>
      <div class="switch-card-desc">Show detailed SSH handshake and authentication messages in terminal.</div>
    </div>
    <label class="switch">
      <input type="checkbox" bind:checked={verboseLog} onchange={saveVerbose} />
      <span class="slider"></span>
    </label>
  </div>
</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 16px; }

  .shell-list { display: flex; flex-direction: column; gap: 6px; }
  .shell-option {
    display: flex; align-items: center; justify-content: space-between;
    padding: 10px 14px; border: none; cursor: pointer;
    font-family: monospace; font-size: 13px;
    color: var(--text-sub); background: var(--bg);
    transition: all 0.15s;
  }
  .shell-option:hover { color: var(--text); }
  .shell-option.active {
    color: var(--accent); font-weight: 600;
    outline: 1px solid var(--accent); outline-offset: -1px;
  }
  .shell-name { flex: 1; }
  .check { color: var(--accent); font-size: 16px; }

  .custom-shell {
    display: flex; flex-direction: column; gap: 4px;
    margin-top: 8px;
  }
</style>
