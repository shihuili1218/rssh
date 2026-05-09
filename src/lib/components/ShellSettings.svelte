<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import { t } from "../i18n/index.svelte.ts";

  let shells = $state<string[]>([]);
  let selectedShell = $state("");
  let verboseLog = $state(true);
  let connectTimeout = $state(10);
  let commandBlockBar = $state(true);

  onMount(async () => {
    try { shells = await invoke<string[]>("list_shells"); } catch { shells = []; }
    selectedShell = await invoke<string | null>("get_setting", { key: "local_shell" }) ?? "";
    verboseLog = (await invoke<string | null>("get_setting", { key: "verbose_log" })) !== "false";
    const ts = await invoke<string | null>("get_setting", { key: "connect_timeout" });
    if (ts) connectTimeout = parseInt(ts, 10) || 10;
    commandBlockBar = await app.loadCommandBlockBar();
  });

  async function saveShell() {
    await invoke("set_setting", { key: "local_shell", value: selectedShell });
  }

  async function saveVerbose() {
    await invoke("set_setting", { key: "verbose_log", value: String(verboseLog) });
  }

  async function saveTimeout() {
    const val = Math.max(1, Math.min(300, connectTimeout));
    connectTimeout = val;
    await invoke("set_setting", { key: "connect_timeout", value: String(val) });
  }

  async function saveCommandBlockBar() {
    await app.setCommandBlockBar(commandBlockBar);
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
      <label>Custom Path</label>
      <input type="text" bind:value={selectedShell} placeholder="/usr/local/bin/fish" onblur={saveShell} />
    </div>
  </div>

  <div class="section-label">CONNECTION TIMEOUT</div>
  <div class="timeout-row">
    <label>Timeout (seconds)</label>
    <input type="number" bind:value={connectTimeout} min="1" max="300" onblur={saveTimeout}
      onkeydown={(e) => { if (e.key === "Enter") saveTimeout(); }} />
    <span class="timeout-hint">1–300s, default 10s</span>
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

  <div class="section-label">{t("settings.shell.command_block")}</div>
  <div class="switch-card">
    <div class="switch-card-body">
      <div class="switch-card-title"
           class:on={commandBlockBar} class:off={!commandBlockBar}>
        {t("settings.shell.command_block_bar")}
      </div>
      <div class="switch-card-desc">{t("settings.shell.command_block_bar_desc")}</div>
    </div>
    <label class="switch">
      <input type="checkbox" bind:checked={commandBlockBar} onchange={saveCommandBlockBar} />
      <span class="slider"></span>
    </label>
  </div>

  {#if commandBlockBar}
    <div class="tips-card">
      <div class="tips-title">{t("settings.shell.command_block_tips_title")}</div>
      <ul class="tips-list">
        <li>{t("settings.shell.command_block_tip_click")}</li>
        <li>{t("settings.shell.command_block_tip_shift_click")}</li>
        <li>{t("settings.shell.command_block_tip_cmd_click")}</li>
        <li>{t("settings.shell.command_block_tip_right_click")}</li>
        <li>{t("settings.shell.command_block_tip_clear")}</li>
      </ul>
    </div>
  {/if}

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

  .timeout-row {
    display: flex; align-items: center; gap: 10px;
  }
  .timeout-row input[type="number"] {
    width: 80px;
  }
  .timeout-hint {
    font-size: 11px; color: var(--text-dim);
  }

  /* Tips 卡：开启侧栏后展开，把交互快捷方式列清楚（单击/shift/cmd/右键/Esc）。
     视觉上比 switch-card 更"轻"——边框替代背景填充，让用户一看就知道是辅助信息。 */
  .tips-card {
    border: 1px solid var(--divider);
    border-radius: 6px;
    padding: 10px 14px;
    background: color-mix(in srgb, var(--accent) 4%, transparent);
  }
  .tips-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-sub);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 6px;
  }
  .tips-list {
    margin: 0;
    padding-left: 18px;
    font-size: 12px;
    color: var(--text);
    line-height: 1.6;
  }
  .tips-list li {
    margin: 2px 0;
  }
</style>
