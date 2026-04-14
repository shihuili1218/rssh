<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Profile } from "../stores/app.svelte.ts";

  let { id = null }: { id: string | null } = $props();

  let name = $state(""); let forwardType = $state("local");
  let localPort = $state(8080); let remoteHost = $state("127.0.0.1");
  let remotePort = $state(80); let profileId = $state("");
  let profiles = $state<Profile[]>([]);
  let saving = $state(false);

  onMount(async () => {
    profiles = await app.loadProfiles();
    if (id) {
      const f = await invoke<any>("get_forward", { id }).catch(() => null);
      if (f) {
        name = f.name; forwardType = f.type;
        localPort = f.local_port; remoteHost = f.remote_host;
        remotePort = f.remote_port; profileId = f.profile_id;
      }
    }
  });

  async function save() {
    saving = true;
    try {
      const forward = {
        id: id ?? crypto.randomUUID(),
        name,
        type: forwardType,
        local_port: localPort,
        remote_host: remoteHost,
        remote_port: remotePort,
        profile_id: profileId,
      };
      if (id) await invoke("update_forward", { forward });
      else await invoke("create_forward", { forward });
      app.navigate("forwards");
    } catch (e: any) { alert(String(e)); }
    finally { saving = false; }
  }
</script>

<div class="page">
  <div class="form">
    <label>Name</label>
    <input type="text" bind:value={name} placeholder="Web Forward" />
    <label>Profile</label>
    <select bind:value={profileId}>
      <option value="">-- Select --</option>
      {#each profiles as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
    </select>
    <label>Type</label>
    <select bind:value={forwardType}>
      <option value="local">Local Forward</option>
      <option value="remote">Remote Forward</option>
      <option value="dynamic">Dynamic (SOCKS5)</option>
    </select>
    {#if forwardType === "dynamic"}
      <div class="field"><label>Local Port (SOCKS5 proxy)</label><input type="number" bind:value={localPort} /></div>
    {:else}
      <div class="row3">
        <div class="field"><label>Local Port</label><input type="number" bind:value={localPort} /></div>
        <div class="field"><label>Remote Host</label><input type="text" bind:value={remoteHost} /></div>
        <div class="field"><label>Remote Port</label><input type="number" bind:value={remotePort} /></div>
      </div>
    {/if}
    <button class="btn btn-accent" onclick={save} disabled={saving || !name || !profileId}>
      {saving ? "Saving..." : "Save"}
    </button>
  </div>
</div>

<style>
  .page { padding: 24px; }
  .form { display: flex; flex-direction: column; gap: 10px; }
  .row3 { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
</style>
