<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Group } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import Select from "./Select.svelte";

  let { id = null }: { id: string | null } = $props();

  // Endpoint
  let name = $state("");
  let host = $state("");
  let port = $state(23);
  // Terminal-layer (shared with serial; crlf is the telnet NVT end-of-line)
  let inputNewline = $state("crlf");
  let outputNewline = $state("raw");
  let localEcho = $state(false);
  let backspace = $state("del");
  // Automation
  let loginScript = $state("");

  let saving = $state(false);
  let groups = $state<Group[]>([]);
  let groupId = $state<string | null>(null);
  let groupOptions = $derived([
    { value: null, label: t("profile.none") },
    ...groups.map((g) => ({ value: g.id, label: g.name })),
  ]);

  const newlineInOptions = [
    { value: "crlf", label: "CRLF (\\r\\n)" },
    { value: "cr", label: "CR (\\r)" },
    { value: "lf", label: "LF (\\n)" },
  ];
  let newlineOutOptions = $derived([
    { value: "raw", label: t("serial.nl.raw") },
    { value: "cr", label: "CR → CRLF" },
    { value: "lf", label: "LF → CRLF" },
    { value: "crlf", label: "CRLF" },
  ]);
  let backspaceOptions = $derived([
    { value: "del", label: t("serial.bs.del") },
    { value: "bs", label: t("serial.bs.bs") },
    { value: "csi3", label: t("serial.bs.csi3") },
  ]);

  onMount(async () => {
    groups = await app.loadGroups();
    if (id) {
      const s = await invoke<any>("get_telnet_profile", { id }).catch(() => null);
      if (s) {
        name = s.name; host = s.host; port = s.port;
        inputNewline = s.input_newline; outputNewline = s.output_newline;
        localEcho = s.local_echo; backspace = s.backspace;
        loginScript = s.login_script;
        groupId = s.group_id ?? null;
      }
    }
  });

  async function save() {
    saving = true;
    try {
      const profile = {
        id: id ?? crypto.randomUUID(),
        name, host, port: Number(port) || 23,
        input_newline: inputNewline, output_newline: outputNewline,
        local_echo: localEcho, backspace, login_script: loginScript,
        group_id: groupId || null,
      };
      if (id) await invoke("update_telnet_profile", { profile });
      else await invoke("create_telnet_profile", { profile });
      app.navigate("telnet-profiles");
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
    finally { saving = false; }
  }
</script>

<div class="page">
  <div class="form">
    <label>{t("common.name")}</label>
    <input type="text" bind:value={name} placeholder={t("telnet.name_placeholder")} />
    <label>{t("profile.group")} {t("common.optional")}</label>
    <Select bind:value={groupId} options={groupOptions} />

    <div class="section-label">{t("telnet.sec.endpoint")}</div>
    <div class="row-hostport">
      <div class="field"><label>{t("telnet.host")}</label><input type="text" bind:value={host} placeholder="192.168.1.1" /></div>
      <div class="field"><label>{t("telnet.port")}</label><input type="number" bind:value={port} min="1" max="65535" /></div>
    </div>

    <div class="section-label">{t("serial.sec.term")}</div>
    <div class="row2">
      <div class="field"><label>{t("serial.nl.in")}</label><Select bind:value={inputNewline} options={newlineInOptions} /></div>
      <div class="field"><label>{t("serial.nl.out")}</label><Select bind:value={outputNewline} options={newlineOutOptions} /></div>
    </div>
    <label>{t("serial.backspace")}</label>
    <Select bind:value={backspace} options={backspaceOptions} />
    <label class="check"><input type="checkbox" bind:checked={localEcho} /> {t("serial.local_echo")}</label>

    <div class="section-label">{t("serial.sec.script")}</div>
    <label>{t("serial.login_script")}</label>
    <textarea bind:value={loginScript} rows="4" placeholder={t("serial.login_script.ph")}></textarea>

    <button class="btn btn-accent" onclick={save} disabled={saving || !name || !host}>
      {saving ? t("common.saving") : t("common.save")}
    </button>
  </div>
</div>

<style>
  .page { padding: 24px; }
  .form { display: flex; flex-direction: column; gap: 10px; }
  .row-hostport { display: grid; grid-template-columns: 2fr 1fr; gap: 8px; }
  .row2 { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .check { display: flex; align-items: center; gap: 8px; }
  textarea { font-family: monospace; resize: vertical; }
  .form :global(.section-label) { margin-top: 10px; }
</style>
