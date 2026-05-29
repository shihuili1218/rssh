<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import { t } from "../i18n/index.svelte.ts";

  let enabled = $state(false);
  let saveDir = $state("");
  let recordings = $state<string[]>([]);

  onMount(async () => {
    enabled = (await invoke<string | null>("get_setting", { key: "recording_enabled" })) === "true";
    saveDir = await invoke<string | null>("get_setting", { key: "recording_dir" }) ?? "";
    loadRecordings();
  });

  async function saveSettings() {
    await invoke("set_setting", { key: "recording_enabled", value: String(enabled) });
    await invoke("set_setting", { key: "recording_dir", value: saveDir });
  }

  async function loadRecordings() {
    try {
      recordings = await invoke<string[]>("list_recordings");
    } catch { recordings = []; }
  }

  function playRecording(name: string) {
    app.settingsNavigate("playback", name);
  }
</script>

<div class="page">
  <div class="switch-card">
    <div class="switch-card-body">
      <div class="switch-card-title" class:on={enabled} class:off={!enabled}>{t("recording.enable")}</div>
      <div class="switch-card-desc">{t("recording.enable_desc")}</div>
    </div>
    <label class="switch">
      <input type="checkbox" bind:checked={enabled} onchange={saveSettings} />
      <span class="slider"></span>
    </label>
  </div>

  <div class="field-group">
    <label>{t("recording.save_dir")}</label>
    <input type="text" bind:value={saveDir} placeholder="~/Documents/rssh-recordings" onblur={saveSettings} />
    <p class="hint">{t("recording.save_dir_hint")}</p>
  </div>

  {#if recordings.length > 0}
    <div class="section-label">{t("recording.list_title")}</div>
    {#each recordings as rec}
      <div class="rec-row neu-sm">
        <span class="rec-name">{rec}</span>
        <button class="btn btn-sm" onclick={() => playRecording(rec)}>▶ {t("recording.playback")}</button>
      </div>
    {/each}
  {/if}
</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 16px; }
  .field-group { display: flex; flex-direction: column; gap: 6px; }
  .hint { font-size: 11px; color: var(--text-dim); }
  .rec-row {
    display: flex; align-items: center; justify-content: space-between;
    padding: 10px 14px; margin-bottom: 6px;
  }
  .rec-name { font-size: 12px; font-family: monospace; color: var(--text-sub); }
</style>
