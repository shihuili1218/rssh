<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Profile, Group } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import Select from "./Select.svelte";
  import { connectionCopyName } from "./connection-editor.ts";

  let { id = null, copyFromId = null }: { id?: string | null; copyFromId?: string | null } = $props();

  let name = $state(""); let forwardType = $state("local");
  let localPort = $state(8080); let remoteHost = $state("127.0.0.1");
  let remotePort = $state(80); let profileId = $state("");
  let profiles = $state<Profile[]>([]);
  let groups = $state<Group[]>([]);
  let groupId = $state<string | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let saving = $state(false);

  let profileOptions = $derived(profiles.map((p) => ({ value: p.id, label: p.name })));
  let forwardTypeOptions = $derived([
    { value: "local",   label: t("forward.type.local") },
    { value: "remote",  label: t("forward.type.remote") },
    { value: "dynamic", label: t("forward.type.dynamic") },
  ]);
  let groupOptions = $derived([
    { value: null, label: t("profile.none") },
    ...groups.map((g) => ({ value: g.id, label: g.name })),
  ]);

  onMount(async () => {
    try {
      [profiles, groups] = await Promise.all([app.loadProfiles(), app.loadGroups()]);
      const sourceId = id ?? copyFromId;
      if (sourceId) {
        const f = await invoke<any>("get_forward", { id: sourceId });
        name = copyFromId ? connectionCopyName(f.name) : f.name; forwardType = f.type;
        localPort = f.local_port; remoteHost = f.remote_host;
        remotePort = f.remote_port; profileId = f.profile_id;
        groupId = f.group_id ?? null;
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
      const forward = {
        id: id ?? crypto.randomUUID(),
        name,
        type: forwardType,
        local_port: localPort,
        remote_host: remoteHost,
        remote_port: remotePort,
        profile_id: profileId,
        group_id: groupId || null,
      };
      if (id) await invoke("update_forward", { forward });
      else await invoke("create_forward", { forward });
      app.navigate("connections");
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
    finally { saving = false; }
  }
</script>

<div class="form" aria-busy={loading}>
    {#if loadError}
      <div class="form-error" role="alert">{loadError}</div>
    {/if}
    <label for="forward-name">{t("common.name")}</label>
    <input id="forward-name" type="text" bind:value={name} placeholder={t("forward.name_placeholder")} />
    <label for="forward-profile">{t("forward.profile")}</label>
    <Select id="forward-profile" bind:value={profileId} options={profileOptions} placeholder={t("forward.select")} />
    <label for="forward-type">{t("forward.type")}</label>
    <Select id="forward-type" bind:value={forwardType} options={forwardTypeOptions} />
    {#if forwardType === "dynamic"}
      <div class="field"><label for="forward-local-port">{t("forward.local_port_socks5")}</label><input id="forward-local-port" type="number" bind:value={localPort} /></div>
    {:else}
      <div class="row3">
        <div class="field"><label for="forward-local-port">{t("forward.local_port")}</label><input id="forward-local-port" type="number" bind:value={localPort} /></div>
        <div class="field"><label for="forward-remote-host">{t("forward.remote_host")}</label><input id="forward-remote-host" type="text" bind:value={remoteHost} /></div>
        <div class="field"><label for="forward-remote-port">{t("forward.remote_port")}</label><input id="forward-remote-port" type="number" bind:value={remotePort} /></div>
      </div>
    {/if}
    <label for="forward-group">{t("profile.group")} {t("common.optional")}</label>
    <Select id="forward-group" bind:value={groupId} options={groupOptions} />
    <div class="form-actions">
      <button type="button" class="btn btn-accent btn-sm" onclick={save} disabled={loading || !!loadError || saving || !name || !profileId}>
        {loading ? t("common.loading") : saving ? t("common.saving") : t("common.save")}
      </button>
      <button type="button" class="btn btn-sm" onclick={() => app.navigate("connections")}>{t("common.cancel")}</button>
    </div>
  </div>

<style>
  .form { display: flex; flex-direction: column; gap: 10px; }
  .row3 { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .form-error {
    padding: 6px 10px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--error) 8%, transparent);
    color: var(--error);
    font-size: 12px;
  }
  .form-actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 8px; }
  @media (max-width: 640px) { .row3 { grid-template-columns: 1fr; } }
</style>
