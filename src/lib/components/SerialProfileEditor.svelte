<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Group } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import Select from "./Select.svelte";
  import { connectionCopyName } from "./connection-editor.ts";

  let { id = null, copyFromId = null }: { id?: string | null; copyFromId?: string | null } = $props();

  // Line settings
  let name = $state("");
  let port = $state("");
  let baudRate = $state(115200);
  let dataBits = $state(8);
  let parity = $state("none");
  let stopBits = $state(1);
  let flowControl = $state("none");
  let xany = $state(false);
  // Terminal-layer
  let inputNewline = $state("cr");
  let outputNewline = $state("raw");
  let localEcho = $state(false);
  let backspace = $state("del");
  let slowSend = $state(false);
  let inputMode = $state("normal");
  let outputMode = $state("text");
  // Automation
  let loginScript = $state("");

  let saving = $state(false);
  // Detected ports populate a <datalist> for the free-form port input.
  let ports = $state<string[]>([]);
  let groups = $state<Group[]>([]);
  let groupId = $state<string | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let groupOptions = $derived([
    { value: null, label: t("profile.none") },
    ...groups.map((g) => ({ value: g.id, label: g.name })),
  ]);

  const dataBitsOptions = [
    { value: 8, label: "8" }, { value: 7, label: "7" },
    { value: 6, label: "6" }, { value: 5, label: "5" },
  ];
  const stopBitsOptions = [{ value: 1, label: "1" }, { value: 2, label: "2" }];
  const newlineInOptions = [
    { value: "cr", label: "CR (\\r)" },
    { value: "lf", label: "LF (\\n)" },
    { value: "crlf", label: "CRLF (\\r\\n)" },
  ];
  let parityOptions = $derived([
    { value: "none", label: t("serial.parity.none") },
    { value: "odd", label: t("serial.parity.odd") },
    { value: "even", label: t("serial.parity.even") },
  ]);
  let flowOptions = $derived([
    { value: "none", label: t("serial.flow.none") },
    { value: "software", label: t("serial.flow.software") },
    { value: "hardware", label: t("serial.flow.hardware") },
  ]);
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
  let inputModeOptions = $derived([
    { value: "normal", label: t("serial.mode.normal") },
    { value: "line", label: t("serial.mode.line") },
    { value: "hex", label: t("serial.mode.hex") },
  ]);
  let outputModeOptions = $derived([
    { value: "text", label: t("serial.mode.text") },
    { value: "hex", label: t("serial.mode.hex") },
  ]);

  onMount(async () => {
    invoke<string[]>("serial_list_ports").then((p) => (ports = p)).catch(() => {});
    try {
      groups = await app.loadGroups();
    } catch (error) {
      console.warn("[serial] failed to load profile groups:", error);
    }
    const sourceId = id ?? copyFromId;
    try {
      if (sourceId) {
        const s = await invoke<any>("get_serial_profile", { id: sourceId });
        name = copyFromId ? connectionCopyName(s.name) : s.name;
        port = s.port; baudRate = s.baud_rate; dataBits = s.data_bits;
        parity = s.parity; stopBits = s.stop_bits; flowControl = s.flow_control;
        xany = s.xany; inputNewline = s.input_newline; outputNewline = s.output_newline;
        localEcho = s.local_echo; backspace = s.backspace; slowSend = s.slow_send;
        inputMode = s.input_mode; outputMode = s.output_mode; loginScript = s.login_script;
        groupId = s.group_id ?? null;
      }
    } catch (error) {
      loadError = errMsg(error);
    } finally {
      loading = false;
    }
  });

  async function save() {
    if (loading || loadError || saving) return;
    saving = true;
    try {
      const profile = {
        id: id ?? crypto.randomUUID(),
        name, port,
        baud_rate: Number(baudRate), data_bits: Number(dataBits), parity,
        stop_bits: Number(stopBits), flow_control: flowControl, xany,
        input_newline: inputNewline, output_newline: outputNewline,
        local_echo: localEcho, backspace, slow_send: slowSend,
        input_mode: inputMode, output_mode: outputMode, login_script: loginScript,
        group_id: groupId || null,
      };
      if (id) await invoke("update_serial_profile", { profile });
      else await invoke("create_serial_profile", { profile });
      app.navigate("connections");
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
    finally { saving = false; }
  }
</script>

<div class="form" aria-busy={loading}>
    {#if loadError}
      <div class="form-error" role="alert">{loadError}</div>
    {/if}
    <label for="serial-name">{t("common.name")}</label>
    <input id="serial-name" type="text" bind:value={name} placeholder={t("serial.name_placeholder")} />
    <label for="serial-group">{t("profile.group")} {t("common.optional")}</label>
    <Select id="serial-group" bind:value={groupId} options={groupOptions} />

    <div class="section-label">{t("serial.sec.line")}</div>
    <label for="serial-port">{t("serial.port")}</label>
    <input id="serial-port" type="text" bind:value={port} placeholder="/dev/cu.usbserial-…" list="serial-ports-list" aria-describedby="serial-port-hint" />
    <datalist id="serial-ports-list">{#each ports as p}<option value={p}></option>{/each}</datalist>
    <p id="serial-port-hint" class="port-hint">{t("serial.port_device_hint")}</p>
    <label for="serial-baud">{t("serial.baud")}</label>
    <input id="serial-baud" type="number" bind:value={baudRate} min="1" />
    <div class="row3">
      <div class="field"><label for="serial-data-bits">{t("serial.data_bits")}</label><Select id="serial-data-bits" bind:value={dataBits} options={dataBitsOptions} /></div>
      <div class="field"><label for="serial-parity">{t("serial.parity")}</label><Select id="serial-parity" bind:value={parity} options={parityOptions} /></div>
      <div class="field"><label for="serial-stop-bits">{t("serial.stop_bits")}</label><Select id="serial-stop-bits" bind:value={stopBits} options={stopBitsOptions} /></div>
    </div>
    <label for="serial-flow">{t("serial.flow")}</label>
    <Select id="serial-flow" bind:value={flowControl} options={flowOptions} />
    <label class="check"><input type="checkbox" bind:checked={xany} /> {t("serial.xany")}</label>

    <div class="section-label">{t("serial.sec.term")}</div>
    <div class="row2">
      <div class="field"><label for="serial-newline-in">{t("serial.nl.in")}</label><Select id="serial-newline-in" bind:value={inputNewline} options={newlineInOptions} /></div>
      <div class="field"><label for="serial-newline-out">{t("serial.nl.out")}</label><Select id="serial-newline-out" bind:value={outputNewline} options={newlineOutOptions} /></div>
    </div>
    <div class="row2">
      <div class="field"><label for="serial-backspace">{t("serial.backspace")}</label><Select id="serial-backspace" bind:value={backspace} options={backspaceOptions} /></div>
      <div class="field"><label for="serial-input-mode">{t("serial.input_mode")}</label><Select id="serial-input-mode" bind:value={inputMode} options={inputModeOptions} /></div>
    </div>
    <label for="serial-output-mode">{t("serial.output_mode")}</label>
    <Select id="serial-output-mode" bind:value={outputMode} options={outputModeOptions} />
    <label class="check"><input type="checkbox" bind:checked={localEcho} /> {t("serial.local_echo")}</label>
    <label class="check"><input type="checkbox" bind:checked={slowSend} /> {t("serial.slow_send")}</label>

    <div class="section-label">{t("serial.sec.script")}</div>
    <label for="serial-login-script">{t("serial.login_script")}</label>
    <textarea id="serial-login-script" bind:value={loginScript} rows="4" placeholder={t("serial.login_script.ph")}></textarea>

    <div class="form-actions">
      <button type="button" class="btn btn-accent btn-sm" onclick={save} disabled={loading || !!loadError || saving || !name || !port}>
        {loading ? t("common.loading") : saving ? t("common.saving") : t("common.save")}
      </button>
      <button type="button" class="btn btn-sm" onclick={() => app.navigate("connections")}>{t("common.cancel")}</button>
    </div>
</div>

<style>
  .form { display: flex; flex-direction: column; gap: 10px; }
  .row3 { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; }
  .row2 { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .check { display: flex; align-items: center; gap: 8px; }
  textarea { font-family: monospace; resize: vertical; }
  .form :global(.section-label) { margin-top: 10px; }
  .port-hint { margin: -4px 0 2px; font-size: 11px; color: var(--text-dim); line-height: 1.4; }
  .form-error {
    padding: 6px 10px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--error) 8%, transparent);
    color: var(--error);
    font-size: 12px;
  }
  .form-actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 8px; }
  @media (max-width: 640px) {
    .row2, .row3 { grid-template-columns: 1fr; }
  }
</style>
