<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import Select from "./Select.svelte";

  let { id = null }: { id: string | null } = $props();

  let name = $state(""); let username = $state("");
  let credentialType = $state("password"); let secret = $state("");
  let saveToRemote = $state(false);
  let saving = $state(false);
  let picking = $state(false);

  /** 翻译跟着 locale 变，必须 $derived。 */
  let credentialTypeOptions = $derived([
    { value: "password",    label: t("credential.type.password") },
    { value: "key",         label: t("credential.type.key") },
    { value: "agent",       label: t("credential.type.agent") },
    { value: "none",        label: t("credential.type.none") },
    { value: "interactive", label: t("credential.type.interactive") },
  ]);

  onMount(async () => {
    if (id) {
      const c = await invoke<any>("get_credential", { id });
      name = c.name; username = c.username;
      credentialType = c.type; secret = c.secret ?? "";
      saveToRemote = c.save_to_remote;
    }
  });

  /** Desktop: native dialog at ~/.ssh; browser: <input type=file> via the
   *  ipc-shim. null = user cancelled — leave the textarea alone. */
  async function pickKeyFile() {
    picking = true;
    try {
      const content = await invoke<string | null>("pick_private_key_file");
      if (content != null) secret = content;
    } catch (e: any) { toast.error(errMsg(e)); }
    finally { picking = false; }
  }

  async function save() {
    saving = true;
    try {
      // Trim paste artifacts off every text field; a whitespace-only secret
      // collapses to null (= no secret), same as before.
      const credential = {
        id: id ?? crypto.randomUUID(),
        name: name.trim(), username: username.trim(),
        type: credentialType,
        secret: secret.trim() || null,
        save_to_remote: saveToRemote,
      };
      if (id) await invoke("update_credential", { credential });
      else await invoke("create_credential", { credential });
      app.navigate("credentials");
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
    finally { saving = false; }
  }
</script>

<div class="page">
  <div class="form">
    <label>{t("credential.name")}</label>
    <input type="text" bind:value={name} placeholder="prod-key" />
    <label>{t("credential.username")}</label>
    <input type="text" bind:value={username} placeholder="root" />
    <label>{t("credential.auth_type")}</label>
    <Select bind:value={credentialType} options={credentialTypeOptions} />
    {#if credentialType === "password"}
      <label>{t("credential.password")}</label>
      <input type="password" bind:value={secret} />
    {:else if credentialType === "key"}
      <label>{t("credential.private_key")}</label>
      <textarea bind:value={secret} rows="6" placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"></textarea>
      {#if !app.isMobile}
        <button class="btn btn-sm pick-key" onclick={pickKeyFile} disabled={picking}>
          {t("credential.pick_key_file")}
        </button>
      {/if}
      <p class="hint">{t("credential.encrypted_key_hint")}</p>
    {:else if credentialType === "agent"}
      <p class="hint agent-hint">{t("credential.agent_hint")}</p>
    {/if}
    <div class="switch-card">
      <div class="switch-card-body">
        <div class="switch-card-title" class:on={saveToRemote} class:off={!saveToRemote}>{t("credential.sync_to_remote")}</div>
        <div class="switch-card-desc">{t("credential.sync_to_remote_desc")}</div>
      </div>
      <label class="switch">
        <input type="checkbox" bind:checked={saveToRemote} />
        <span class="slider"></span>
      </label>
    </div>
    <button class="btn btn-accent" onclick={save} disabled={saving || !name.trim() || !username.trim()}>
      {saving ? t("common.saving") : t("common.save")}
    </button>
  </div>
</div>

<style>
  .page { padding: 24px; }
  .form { display: flex; flex-direction: column; gap: 10px; }
  .pick-key { align-self: flex-start; }
  textarea { font-family: monospace; font-size: 12px; resize: vertical; }
  .hint { font-size: 13px; color: var(--text-dim); margin: 4px 0; line-height: 1.55; }
  .agent-hint { white-space: pre-line; }
</style>
