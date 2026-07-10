<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Group, TelnetProfile } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import Select from "./Select.svelte";
  import { connectionCopyName } from "./connection-editor.ts";

  let { id = null, copyFromId = null }: { id?: string | null; copyFromId?: string | null } = $props();

  // Endpoint
  let name = $state("");
  let host = $state("");
  let port = $state(23);
  // Terminal-layer (shared with serial; crlf is the telnet NVT end-of-line)
  let inputNewline = $state("crlf");
  let outputNewline = $state("raw");
  let echoMode = $state<"auto" | "on" | "off">("auto");
  let backspace = $state("del");
  // Automation
  let loginScript = $state("");
  let saveScriptToRemote = $state(false);

  let loading = $state(true);
  let loadError = $state<string | null>(null);
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
  let echoModeOptions = $derived([
    { value: "auto", label: t("telnet.echo.auto") },
    { value: "on", label: t("telnet.echo.on") },
    { value: "off", label: t("telnet.echo.off") },
  ]);

  onMount(async () => {
    try {
      groups = await app.loadGroups();
    } catch (error) {
      // Grouping is optional. Keep the profile editable if only this auxiliary
      // list fails; the bound group id still round-trips unchanged below.
      console.warn("[telnet] failed to load profile groups:", error);
    }
    const sourceId = id ?? copyFromId;
    if (!sourceId) {
      loading = false;
      return;
    }
    try {
      const s = await invoke<TelnetProfile>("get_telnet_profile", { id: sourceId });
      name = copyFromId ? connectionCopyName(s.name) : s.name;
      host = s.host; port = s.port;
      inputNewline = s.input_newline; outputNewline = s.output_newline;
      echoMode = s.echo_mode ?? (s.local_echo ? "on" : "off");
      backspace = s.backspace;
      loginScript = s.login_script;
      saveScriptToRemote = s.save_script_to_remote ?? false;
      groupId = s.group_id ?? null;
    } catch (error) {
      loadError = errMsg(error);
    } finally {
      loading = false;
    }
  });

  async function save() {
    // A failed hydrate must never turn an existing profile into a blank edit.
    // Keep the form fail-closed until the user reloads or leaves this page.
    if (loading || loadError || saving) return;
    saving = true;
    try {
      const profile = {
        id: id ?? crypto.randomUUID(),
        name, host, port: Number(port) || 23,
        input_newline: inputNewline, output_newline: outputNewline,
        echo_mode: echoMode,
        // Keep old clients/sync payload readers meaningful while echo_mode rolls out.
        local_echo: echoMode === "on",
        backspace, login_script: loginScript,
        save_script_to_remote: saveScriptToRemote,
        group_id: groupId || null,
      };
      if (id) {
        await invoke("update_telnet_profile", {
          profile,
          // The editor owns a hydrated, complete value. Metadata-only callers
          // omit this flag so a scrubbed empty field preserves the secret.
          loginScriptUpdate: "replace",
        });
      } else {
        await invoke("create_telnet_profile", { profile });
      }
      app.navigate("connections");
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
    finally { saving = false; }
  }
</script>

<div class="form" aria-busy={loading}>
    {#if loadError}
      <div class="form-error" role="alert">{loadError}</div>
    {/if}
    <label for="telnet-name">{t("common.name")}</label>
    <input id="telnet-name" type="text" bind:value={name} placeholder={t("telnet.name_placeholder")} />
    <label for="telnet-group">{t("profile.group")} {t("common.optional")}</label>
    <Select id="telnet-group" bind:value={groupId} options={groupOptions} />

    <div class="section-label">{t("telnet.sec.endpoint")}</div>
    <div class="row-hostport">
      <div class="field"><label for="telnet-host">{t("telnet.host")}</label><input id="telnet-host" type="text" bind:value={host} placeholder="192.168.1.1" /></div>
      <div class="field"><label for="telnet-port">{t("telnet.port")}</label><input id="telnet-port" type="number" bind:value={port} min="1" max="65535" /></div>
    </div>

    <div class="section-label">{t("serial.sec.term")}</div>
    <div class="row2">
      <div class="field"><label for="telnet-newline-in">{t("serial.nl.in")}</label><Select id="telnet-newline-in" bind:value={inputNewline} options={newlineInOptions} /></div>
      <div class="field"><label for="telnet-newline-out">{t("serial.nl.out")}</label><Select id="telnet-newline-out" bind:value={outputNewline} options={newlineOutOptions} /></div>
    </div>
    <label for="telnet-backspace">{t("serial.backspace")}</label>
    <Select id="telnet-backspace" bind:value={backspace} options={backspaceOptions} />
    <label for="telnet-echo-mode">{t("telnet.echo_mode")}</label>
    <Select id="telnet-echo-mode" bind:value={echoMode} options={echoModeOptions} />

    <div class="section-label">{t("serial.sec.script")}</div>
    <label for="telnet-login-script">{t("serial.login_script")}</label>
    <textarea id="telnet-login-script" bind:value={loginScript} rows="4" placeholder={t("serial.login_script.ph")}></textarea>
    <label class="check"><input type="checkbox" bind:checked={saveScriptToRemote} /> {t("telnet.save_script_to_remote")}</label>

    <div class="form-actions">
      <button type="button" class="btn btn-accent btn-sm" onclick={save} disabled={loading || !!loadError || saving || !name || !host}>
        {loading ? t("common.loading") : saving ? t("common.saving") : t("common.save")}
      </button>
      <button type="button" class="btn btn-sm" onclick={() => app.navigate("connections")}>{t("common.cancel")}</button>
    </div>
</div>

<style>
  .form { display: flex; flex-direction: column; gap: 10px; }
  .row-hostport { display: grid; grid-template-columns: 2fr 1fr; gap: 8px; }
  .row2 { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  textarea { font-family: monospace; resize: vertical; }
  .check { display: flex; align-items: center; gap: 8px; }
  .form-error { color: var(--error); font-size: 12px; }
  .form :global(.section-label) { margin-top: 10px; }
  .form-actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 8px; }
  @media (max-width: 640px) {
    .row-hostport, .row2 { grid-template-columns: 1fr; }
  }
</style>
