<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";

  let { id = null }: { id: string | null } = $props();

  let name = $state(""); let username = $state("");
  let credentialType = $state("password"); let secret = $state("");
  let saveToRemote = $state(false);
  let saving = $state(false);

  onMount(async () => {
    if (id) {
      const c = await invoke<any>("get_credential", { id });
      name = c.name; username = c.username;
      credentialType = c.type; secret = c.secret ?? "";
      saveToRemote = c.save_to_remote;
    }
  });

  async function save() {
    saving = true;
    try {
      const credential = {
        id: id ?? crypto.randomUUID(),
        name, username,
        type: credentialType,
        secret: secret || null,
        save_to_remote: saveToRemote,
      };
      if (id) await invoke("update_credential", { credential });
      else await invoke("create_credential", { credential });
      app.navigate("credentials");
    } catch (e: any) { alert(String(e)); }
    finally { saving = false; }
  }
</script>

<div class="page">
  <div class="form">
    <label>Name</label>
    <input type="text" bind:value={name} placeholder="prod-key" />
    <label>Username</label>
    <input type="text" bind:value={username} placeholder="root" />
    <label>Auth Type</label>
    <select bind:value={credentialType}>
      <option value="password">Password</option>
      <option value="key">Private Key (PEM)</option>
      <option value="none">None</option>
      <option value="interactive">Keyboard Interactive</option>
    </select>
    {#if credentialType === "password"}
      <label>Password</label>
      <input type="password" bind:value={secret} />
    {:else if credentialType === "key"}
      <label>Private Key</label>
      <textarea bind:value={secret} rows="6" placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"></textarea>
    {/if}
    <div class="switch-card">
      <div class="switch-card-body">
        <div class="switch-card-title" class:on={saveToRemote} class:off={!saveToRemote}>SYNC TO REMOTE</div>
        <div class="switch-card-desc">Include this credential's secret when pushing to GitHub.</div>
      </div>
      <label class="switch">
        <input type="checkbox" bind:checked={saveToRemote} />
        <span class="slider"></span>
      </label>
    </div>
    <button class="btn btn-accent" onclick={save} disabled={saving || !name || !username}>
      {saving ? "Saving..." : "Save"}
    </button>
  </div>
</div>

<style>
  .page { padding: 24px; }
  .form { display: flex; flex-direction: column; gap: 10px; }
  textarea { font-family: monospace; font-size: 12px; resize: vertical; }
</style>
