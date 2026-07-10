<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Credential, Profile, Group, SshAlgorithmCatalog, SshAlgorithms } from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import type { MessageKey } from "../i18n/locales/en.ts";
  import Select from "./Select.svelte";

  let { id = null }: { id: string | null } = $props();
  type AlgorithmCategory = keyof SshAlgorithms;

  const emptyAlgorithms = (): SshAlgorithms => ({
    kex: [],
    key: [],
    cipher: [],
    mac: [],
    compression: [],
  });
  const algorithmCategories: Array<{ id: AlgorithmCategory; labelKey: MessageKey }> = [
    { id: "kex", labelKey: "profile.algorithms.kex" },
    { id: "key", labelKey: "profile.algorithms.key" },
    { id: "cipher", labelKey: "profile.algorithms.cipher" },
    { id: "mac", labelKey: "profile.algorithms.mac" },
    { id: "compression", labelKey: "profile.algorithms.compression" },
  ];

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
  let algorithmCatalog = $state<SshAlgorithmCatalog | null>(null);
  let algorithms = $state<SshAlgorithms>(emptyAlgorithms());
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
  let algorithmSelectionComplete = $derived(
    algorithmCategories.every((category) => {
      const supported = algorithmCatalog?.supported[category.id] ?? [];
      return algorithms[category.id].some((name) => supported.includes(name));
    }),
  );

  onMount(async () => {
    [credentials, profiles, groups, algorithmCatalog] = await Promise.all([
      app.loadCredentials(),
      app.loadProfiles(),
      app.loadGroups(),
      invoke<SshAlgorithmCatalog>("ssh_algorithm_catalog"),
    ]);
    algorithms = cloneAlgorithms(algorithmCatalog.defaults);
    // Edit loads by `id` (Save updates that row). Copy loads from the store's
    // copy source while `id` stays null (Save creates a new row) and appends
    // "_copy" to the name. Same fill path, one branch — the only differences
    // are the source id and the name suffix.
    const sourceId = id ?? app.copyFromProfileId();
    if (sourceId) {
      const p = await invoke<any>("get_profile", { id: sourceId });
      name = id ? p.name : `${p.name}_copy`;
      host = p.host; port = p.port;
      credentialId = p.credential_id ?? ""; bastionId = p.bastion_profile_id;
      shellCommand = p.init_command ?? "";
      groupId = p.group_id ?? null;
      algorithms = cloneAlgorithms(p.algorithms ?? algorithmCatalog.defaults);
    }
    // Consume once: a later "+ New" must start blank.
    app.clearCopyFromProfile();
  });

  function cloneAlgorithms(value: SshAlgorithms): SshAlgorithms {
    return {
      kex: [...(value.kex ?? [])],
      key: [...(value.key ?? [])],
      cipher: [...(value.cipher ?? [])],
      mac: [...(value.mac ?? [])],
      compression: [...(value.compression ?? [])],
    };
  }

  function checked(category: AlgorithmCategory, name: string) {
    return algorithms[category].includes(name);
  }

  function toggleAlgorithm(category: AlgorithmCategory, name: string) {
    const current = algorithms[category];
    const next = current.includes(name)
      ? current.filter((item) => item !== name)
      : [...current, name];
    algorithms = { ...algorithms, [category]: next };
  }

  function resetAlgorithms() {
    if (!algorithmCatalog) return;
    algorithms = cloneAlgorithms(algorithmCatalog.defaults);
  }

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
        algorithms,
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
    <label for="profile-name">{t("profile.name")}</label>
    <input id="profile-name" type="text" bind:value={name} placeholder={t("profile.placeholder.name")} />
    <label for="profile-host">{t("profile.host")}</label>
    <input id="profile-host" type="text" bind:value={host} placeholder={t("profile.placeholder.host")} />
    <label for="profile-port">{t("profile.port")}</label>
    <input id="profile-port" type="number" bind:value={port} min="1" max="65535" />
    <label for="profile-credential">{t("profile.credential")}</label>
    <Select id="profile-credential" bind:value={credentialId} options={credentialOptions} placeholder={t("profile.select_credential")} />
    <label for="profile-bastion">{t("profile.bastion")} {t("common.optional")}</label>
    <Select id="profile-bastion" bind:value={bastionId} options={bastionOptions} />
    <label for="profile-group">{t("profile.group")} {t("common.optional")}</label>
    <Select id="profile-group" bind:value={groupId} options={groupOptions} />
    <label for="profile-init-command">{t("profile.init_command")} {t("common.optional")}</label>
    <input id="profile-init-command" type="text" bind:value={shellCommand} placeholder={t("profile.placeholder.init")} />
    <details class="algorithm-panel">
      <summary>{t("profile.algorithms")}</summary>
      {#if algorithmCatalog}
        <div class="algorithm-actions">
          <button type="button" class="btn btn-sm" onclick={resetAlgorithms}>
            {t("profile.algorithms.reset")}
          </button>
        </div>
        {#each algorithmCategories as category}
          <fieldset class="algorithm-category">
            <legend>{t(category.labelKey)}</legend>
            <div class="algorithm-list">
              {#each algorithmCatalog.supported[category.id] as algorithm}
                <label class="algorithm-option">
                  <input
                    type="checkbox"
                    checked={checked(category.id, algorithm)}
                    onchange={() => toggleAlgorithm(category.id, algorithm)}
                  />
                  <span>{algorithm}</span>
                </label>
              {/each}
            </div>
          </fieldset>
        {/each}
        {#if !algorithmSelectionComplete}
          <p class="form-error">{t("profile.algorithms.empty")}</p>
        {/if}
      {/if}
    </details>
    <button class="btn btn-accent" onclick={save} disabled={saving || !name || !host || !credentialId || !algorithmSelectionComplete}>
      {saving ? t("common.saving") : t("common.save")}
    </button>
  </div>
</div>

<style>
  .page { padding: 24px; }
  .form { display: flex; flex-direction: column; gap: 10px; }
  .form .btn-accent { margin-top: 8px; }
  .algorithm-panel {
    border: 1px solid var(--divider);
    border-radius: 8px;
    padding: 10px 12px;
  }
  .algorithm-panel summary {
    cursor: pointer;
    font-weight: 600;
  }
  .algorithm-actions {
    display: flex;
    justify-content: flex-end;
    margin: 10px 0;
  }
  .algorithm-category {
    border: 0;
    margin: 0 0 12px;
    padding: 0;
  }
  .algorithm-category legend {
    color: var(--text-sub);
    font-size: 12px;
    font-weight: 600;
    margin-bottom: 6px;
  }
  .algorithm-list {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 6px 12px;
  }
  .algorithm-option {
    align-items: flex-start;
    display: flex;
    gap: 8px;
    min-width: 0;
  }
  .algorithm-option input {
    flex: 0 0 auto;
    margin-top: 2px;
  }
  .algorithm-option span {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace;
    font-size: 12px;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }
  .form-error {
    color: var(--error);
    font-size: 12px;
    margin: 0;
  }
</style>
