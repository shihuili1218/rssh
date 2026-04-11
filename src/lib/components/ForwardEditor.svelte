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
    <label>名称</label>
    <input type="text" bind:value={name} placeholder="Web Forward" />
    <label>关联 Profile</label>
    <select bind:value={profileId}>
      <option value="">-- 选择 --</option>
      {#each profiles as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
    </select>
    <label>类型</label>
    <select bind:value={forwardType}>
      <option value="local">本地转发 (Local)</option>
      <option value="remote">远程转发 (Remote)</option>
    </select>
    <div class="row3">
      <div class="field"><label>本地端口</label><input type="number" bind:value={localPort} /></div>
      <div class="field"><label>远程主机</label><input type="text" bind:value={remoteHost} /></div>
      <div class="field"><label>远程端口</label><input type="number" bind:value={remotePort} /></div>
    </div>
    <button class="btn btn-accent" onclick={save} disabled={saving || !name || !profileId}>
      {saving ? "保存中..." : "保存"}
    </button>
  </div>
</div>

<style>
  .page { padding: 24px; }
  .form { display: flex; flex-direction: column; gap: 10px; }
  .row3 { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
</style>
