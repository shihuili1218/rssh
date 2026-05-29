<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Credential, Profile, Group } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import Select from "./Select.svelte";

  let { id = null }: { id: string | null } = $props();

  let name = $state(""); let host = $state(""); let port = $state(22);
  // credential_id 必填：空串 placeholder 仅用于 select 初始值，Save 按钮在
  // 未选时 disabled。后端 add/edit 入口也校验，前端的禁用只是少一次往返。
  let credentialId = $state("");
  let bastionId = $state<string | null>(null);
  let shellCommand = $state("");
  let credentials = $state<Credential[]>([]);
  let profiles = $state<Profile[]>([]);
  let groups = $state<Group[]>([]);
  let groupId = $state<string | null>(null);
  let saving = $state(false);

  let bastionProfiles = $derived(profiles.filter(p => p.id !== id));

  /** 下拉选项 —— 列表动态，跟随 onMount 拉到的数据。 */
  let credentialOptions = $derived(
    credentials.map((c) => ({ value: c.id, label: `${c.name} (${c.username})` })),
  );
  let bastionOptions = $derived([
    { value: null, label: t("profile.none") },
    ...bastionProfiles.map((p) => ({ value: p.id, label: `${p.name} (${p.host}:${p.port})` })),
  ]);
  let groupOptions = $derived([
    { value: null, label: t("profile.none") },
    ...groups.map((g) => ({ value: g.id, label: g.name })),
  ]);

  onMount(async () => {
    [credentials, profiles, groups] = await Promise.all([app.loadCredentials(), app.loadProfiles(), app.loadGroups()]);
    // Edit loads by `id` (Save updates that row). Copy loads from the store's
    // copy source while `id` stays null (Save creates a new row) and appends
    // " copy" to the name. Same fill path, one branch — the only differences
    // are the source id and the name suffix.
    const sourceId = id ?? app.copyFromProfileId();
    if (sourceId) {
      const p = await invoke<any>("get_profile", { id: sourceId });
      name = id ? p.name : `${p.name}_copy`;
      host = p.host; port = p.port;
      credentialId = p.credential_id ?? ""; bastionId = p.bastion_profile_id;
      shellCommand = p.init_command ?? "";
      groupId = p.group_id ?? null;
    }
    // Consume once: a later "+ New" must start blank.
    app.clearCopyFromProfile();
  });

  async function save() {
    saving = true;
    try {
      const profile = {
        id: id ?? crypto.randomUUID(),
        name, host, port,
        credential_id: credentialId,
        bastion_profile_id: bastionId || null,
        init_command: shellCommand || null,
        group_id: groupId || null,
      };
      if (id) await invoke("update_profile", { profile });
      else await invoke("create_profile", { profile });
      app.navigate("profiles");
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
    finally { saving = false; }
  }
</script>

<div class="page">
  <div class="form">
    <label>{t("profile.name")}</label>
    <input type="text" bind:value={name} placeholder={t("profile.placeholder.name")} />
    <label>{t("profile.host")}</label>
    <input type="text" bind:value={host} placeholder={t("profile.placeholder.host")} />
    <label>{t("profile.port")}</label>
    <input type="number" bind:value={port} min="1" max="65535" />
    <label>{t("profile.credential")}</label>
    <Select bind:value={credentialId} options={credentialOptions} placeholder={t("profile.select_credential")} />
    <label>{t("profile.bastion")} {t("common.optional")}</label>
    <Select bind:value={bastionId} options={bastionOptions} />
    <label>{t("profile.group")} {t("common.optional")}</label>
    <Select bind:value={groupId} options={groupOptions} />
    <label>{t("profile.init_command")} {t("common.optional")}</label>
    <input type="text" bind:value={shellCommand} placeholder={t("profile.placeholder.init")} />
    <button class="btn btn-accent" onclick={save} disabled={saving || !name || !host || !credentialId}>
      {saving ? t("common.saving") : t("common.save")}
    </button>
  </div>
</div>

<style>
  .page { padding: 24px; }
  .form { display: flex; flex-direction: column; gap: 10px; }
  .form .btn-accent { margin-top: 8px; }
</style>
