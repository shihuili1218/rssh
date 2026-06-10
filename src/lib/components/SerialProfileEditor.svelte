<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import Select from "./Select.svelte";

  let { id = null }: { id: string | null } = $props();

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
    if (id) {
      const s = await invoke<any>("get_serial_profile", { id }).catch(() => null);
      if (s) {
        name = s.name; port = s.port; baudRate = s.baud_rate; dataBits = s.data_bits;
        parity = s.parity; stopBits = s.stop_bits; flowControl = s.flow_control;
        xany = s.xany; inputNewline = s.input_newline; outputNewline = s.output_newline;
        localEcho = s.local_echo; backspace = s.backspace; slowSend = s.slow_send;
        inputMode = s.input_mode; outputMode = s.output_mode; loginScript = s.login_script;
      }
    }
  });

  async function save() {
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
      };
      if (id) await invoke("update_serial_profile", { profile });
      else await invoke("create_serial_profile", { profile });
      app.navigate("serial-profiles");
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
    finally { saving = false; }
  }
</script>

<div class="page">
  <div class="form">
    <label>{t("common.name")}</label>
    <input type="text" bind:value={name} placeholder={t("serial.name_placeholder")} />

    <div class="section-label">{t("serial.sec.line")}</div>
    <label>{t("serial.port")}</label>
    <input type="text" bind:value={port} placeholder="/dev/cu.usbserial-…" list="serial-ports-list" />
    <datalist id="serial-ports-list">{#each ports as p}<option value={p}></option>{/each}</datalist>
    <label>{t("serial.baud")}</label>
    <input type="number" bind:value={baudRate} min="1" />
    <div class="row3">
      <div class="field"><label>{t("serial.data_bits")}</label><Select bind:value={dataBits} options={dataBitsOptions} /></div>
      <div class="field"><label>{t("serial.parity")}</label><Select bind:value={parity} options={parityOptions} /></div>
      <div class="field"><label>{t("serial.stop_bits")}</label><Select bind:value={stopBits} options={stopBitsOptions} /></div>
    </div>
    <label>{t("serial.flow")}</label>
    <Select bind:value={flowControl} options={flowOptions} />
    <label class="check"><input type="checkbox" bind:checked={xany} /> {t("serial.xany")}</label>

    <div class="section-label">{t("serial.sec.term")}</div>
    <div class="row2">
      <div class="field"><label>{t("serial.nl.in")}</label><Select bind:value={inputNewline} options={newlineInOptions} /></div>
      <div class="field"><label>{t("serial.nl.out")}</label><Select bind:value={outputNewline} options={newlineOutOptions} /></div>
    </div>
    <div class="row2">
      <div class="field"><label>{t("serial.backspace")}</label><Select bind:value={backspace} options={backspaceOptions} /></div>
      <div class="field"><label>{t("serial.input_mode")}</label><Select bind:value={inputMode} options={inputModeOptions} /></div>
    </div>
    <label>{t("serial.output_mode")}</label>
    <Select bind:value={outputMode} options={outputModeOptions} />
    <label class="check"><input type="checkbox" bind:checked={localEcho} /> {t("serial.local_echo")}</label>
    <label class="check"><input type="checkbox" bind:checked={slowSend} /> {t("serial.slow_send")}</label>

    <div class="section-label">{t("serial.sec.script")}</div>
    <label>{t("serial.login_script")}</label>
    <textarea bind:value={loginScript} rows="4" placeholder={t("serial.login_script.ph")}></textarea>

    <button class="btn btn-accent" onclick={save} disabled={saving || !name || !port}>
      {saving ? t("common.saving") : t("common.save")}
    </button>
  </div>
</div>

<style>
  .page { padding: 24px; }
  .form { display: flex; flex-direction: column; gap: 10px; }
  .row3 { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; }
  .row2 { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .check { display: flex; align-items: center; gap: 8px; }
  textarea { font-family: monospace; resize: vertical; }
  .form :global(.section-label) { margin-top: 10px; }
</style>
