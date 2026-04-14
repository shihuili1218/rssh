<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Credential, Profile } from "../stores/app.svelte.ts";

  let { id = null }: { id: string | null } = $props();

  let name = $state(""); let host = $state(""); let port = $state(22);
  let credentialId = $state<string | null>(null);
  let bastionId = $state<string | null>(null);
  let shellCommand = $state("");
  let credentials = $state<Credential[]>([]);
  let profiles = $state<Profile[]>([]);
  let saving = $state(false);

  let bastionProfiles = $derived(profiles.filter(p => p.id !== id));

  onMount(async () => {
    [credentials, profiles] = await Promise.all([app.loadCredentials(), app.loadProfiles()]);
    if (id) {
      const p = await invoke<any>("get_profile", { id });
      name = p.name; host = p.host; port = p.port;
      credentialId = p.credential_id; bastionId = p.bastion_profile_id;
      shellCommand = p.init_command ?? "";
    }
  });

  async function save() {
    saving = true;
    try {
      const profile = {
        id: id ?? crypto.randomUUID(),
        name, host, port,
        credential_id: credentialId || null,
        bastion_profile_id: bastionId || null,
        init_command: shellCommand || null,
      };
      if (id) await invoke("update_profile", { profile });
      else await invoke("create_profile", { profile });
      app.navigate("profiles");
    } catch (e: any) { alert(String(e)); }
    finally { saving = false; }
  }
</script>

<div class="page">
  <div class="form">
    <label>Name</label>
    <input type="text" bind:value={name} placeholder="My Server" />
    <label>Host</label>
    <input type="text" bind:value={host} placeholder="192.168.1.1" />
    <label>Port</label>
    <input type="number" bind:value={port} min="1" max="65535" />
    <label>Credential</label>
    <select bind:value={credentialId}>
      <option value={null}>-- Select Credential --</option>
      {#each credentials as c (c.id)}
        <option value={c.id}>{c.name} ({c.username})</option>
      {/each}
    </select>
    <label>Bastion Host (optional)</label>
    <select bind:value={bastionId}>
      <option value={null}>-- None --</option>
      {#each bastionProfiles as p (p.id)}
        <option value={p.id}>{p.name} ({p.host}:{p.port})</option>
      {/each}
    </select>
    <label>Init Command (optional)</label>
    <input type="text" bind:value={shellCommand} placeholder="cd /app && ls" />
    <button class="btn btn-accent" onclick={save} disabled={saving || !name || !host}>
      {saving ? "Saving..." : "Save"}
    </button>
  </div>
</div>

<style>
  .page { padding: 24px; }
  .form { display: flex; flex-direction: column; gap: 10px; }
  .form .btn-accent { margin-top: 8px; }
</style>
