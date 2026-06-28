<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import Select from "./Select.svelte";
  import { pickTextFile } from "../pick-file.ts";

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

  /** Read the key file in the webview — one path for desktop, mobile and
   *  browser. null = user cancelled, so leave the textarea alone. */
  async function pickKeyFile() {
    picking = true;
    try {
      const f = await pickTextFile({ maxBytes: 1024 * 1024 });
      if (f) secret = f.text;
    } catch (e: any) { toast.error(errMsg(e)); }
    finally { picking = false; }
  }

  /** The usual default private keys. One click reads ~/.ssh/<name> on the host
   *  (where the keys live) and drops it into the textarea — saves hunting for
   *  the file in a picker. Hidden on mobile: there is no ~/.ssh there. */
  const DEFAULT_KEY_NAMES = ["id_rsa", "id_ed25519"];
  async function fillDefaultKey(keyName: string) {
    picking = true;
    try {
      secret = await invoke<string>("read_default_key_file", { name: keyName });
    } catch (e: any) { toast.error(errMsg(e)); }
    finally { picking = false; }
  }

  async function save() {
    saving = true;
    try {
      // Trim paste artifacts off name/username, and off the secret only for
      // keys (PEM is whitespace-tolerant); passwords may legitimately start
      // or end with spaces. Empty string still collapses to null (= no secret).
      const credential = {
        id: id ?? crypto.randomUUID(),
        name: name.trim(), username: username.trim(),
        type: credentialType,
        secret: (credentialType === "key" ? secret.trim() : secret) || null,
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
      <div class="key-quick">
        <button class="btn btn-sm" onclick={pickKeyFile} disabled={picking}>
          {t("credential.pick_key_file")}
        </button>
        {#if !app.isMobile}
          {#each DEFAULT_KEY_NAMES as keyName}
            <button class="chip" onclick={() => fillDefaultKey(keyName)} disabled={picking}>~/.ssh/{keyName}</button>
          {/each}
        {/if}
      </div>
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
  .key-quick { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .chip {
    font-family: monospace; font-size: 12px;
    padding: 3px 8px; border-radius: 6px;
    border: 1px solid var(--border); background: var(--surface);
    color: var(--text); cursor: pointer;
  }
  .chip:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
  .chip:disabled { opacity: 0.5; cursor: default; }
  textarea { font-family: monospace; font-size: 12px; resize: vertical; }
  .hint { font-size: 13px; color: var(--text-dim); margin: 4px 0; line-height: 1.55; }
  .agent-hint { white-space: pre-line; }
</style>
