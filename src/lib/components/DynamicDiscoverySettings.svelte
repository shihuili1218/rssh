<script lang="ts">
  import { onMount } from "svelte";
  import type {
    DynamicDiscoveryContext,
    DynamicDiscoverySource,
    DynamicDiscoveryToolStatus,
    DynamicPlatform,
  } from "../stores/app.svelte.ts";
  import * as app from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import DynamicDiscoverySourceForm from "./DynamicDiscoverySourceForm.svelte";

  let items = $state<DynamicDiscoverySource[]>([]);
  let contexts = $state<DynamicDiscoveryContext[]>([]);
  let statuses = $state<Partial<Record<DynamicPlatform, DynamicDiscoveryToolStatus>>>({});
  let adding = $state<DynamicDiscoverySource | null>(null);
  let addKey = $state(0);
  let editId = $state<string | null>(null);

  onMount(() => {
    void refresh();
    void refreshPlatformInfo();
  });

  async function refresh() {
    try {
      items = await app.loadDynamicDiscoverySources();
    } catch (e: any) {
      items = [];
      toast.error(errMsg(e));
    }
  }

  async function loadToolStatus(platform: DynamicPlatform): Promise<DynamicDiscoveryToolStatus> {
    try {
      return await app.dynamicDiscoveryToolStatus(platform);
    } catch (e: any) {
      toast.error(errMsg(e));
      return {
        platform,
        available: false,
        version: null,
        error: errMsg(e),
      };
    }
  }

  async function refreshPlatformInfo() {
    const [dockerStatus, k8sStatus, dockerContexts, k8sContexts] = await Promise.all([
      loadToolStatus("docker"),
      loadToolStatus("k8s"),
      app.loadDynamicDiscoveryContexts("docker").catch(() => []),
      app.loadDynamicDiscoveryContexts("k8s").catch(() => []),
    ]);
    statuses = { docker: dockerStatus, k8s: k8sStatus };
    contexts = [...dockerContexts, ...k8sContexts];
  }

  async function refreshContextInfo(platform: DynamicPlatform) {
    const [status, nextContexts] = await Promise.all([
      loadToolStatus(platform),
      app.loadDynamicDiscoveryContexts(platform).catch(() => []),
    ]);
    statuses = { ...statuses, [platform]: status };
    contexts = [
      ...contexts.filter((c) => c.platform !== platform),
      ...nextContexts,
    ];
  }

  function defaultContext(platform: DynamicPlatform): string {
    return contexts.find((c) => c.platform === platform && c.current)?.name
      ?? contexts.find((c) => c.platform === platform)?.name
      ?? "";
  }

  function defaultPlatform(): DynamicPlatform {
    if (statuses.docker?.available) return "docker";
    if (statuses.k8s?.available) return "k8s";
    return "docker";
  }

  function blankSource(platform: DynamicPlatform): DynamicDiscoverySource {
    return platform === "docker"
      ? {
          id: crypto.randomUUID(),
          name: "Docker",
          enabled: true,
          platform: "docker",
          context: defaultContext("docker"),
          shell: "sh",
        }
      : {
          id: crypto.randomUUID(),
          name: "K8S",
          enabled: true,
          platform: "k8s",
          context: defaultContext("k8s"),
          namespace: null,
          shell: "sh",
        };
  }

  function startAdd() {
    editId = null;
    addKey += 1;
    adding = blankSource(defaultPlatform());
  }

  function startEdit(source: DynamicDiscoverySource) {
    adding = null;
    editId = source.id;
  }

  function cancelForm() {
    adding = null;
    editId = null;
  }

  async function saveAll(next: DynamicDiscoverySource[]): Promise<boolean> {
    try {
      await app.saveDynamicDiscoverySources(next);
      items = next;
      await refresh();
      return true;
    } catch (e: any) {
      toast.error(`${t("toast.error.save")}: ${errMsg(e)}`);
      return false;
    }
  }

  async function saveNew(source: DynamicDiscoverySource) {
    if (await saveAll([...items, source])) adding = null;
  }

  async function saveEdit(source: DynamicDiscoverySource) {
    if (await saveAll(items.map((it) => it.id === source.id ? source : it))) editId = null;
  }

  async function toggleEnabled(source: DynamicDiscoverySource) {
    const previous = items;
    const next = items.map((it) => it.id === source.id ? { ...it, enabled: !it.enabled } : it);
    items = next;
    if (!(await saveAll(next))) items = previous;
  }

  async function remove(id: string) {
    await saveAll(items.filter((it) => it.id !== id));
    if (editId === id) editId = null;
  }

  async function testDiscovery() {
    try {
      const snapshot = await app.discoverDynamicTargets();
      if (snapshot.errors.length) {
        toast.error(t("dynamic_discovery.probe_partial", { targets: snapshot.targets.length, errors: snapshot.errors.length }));
      } else {
        toast.success(t("dynamic_discovery.probe_ok", { targets: snapshot.targets.length }));
      }
    } catch (e: any) {
      toast.error(errMsg(e));
    }
  }

  function sourceMeta(source: DynamicDiscoverySource): string {
    if (source.platform === "docker") return `Docker · ${source.context}`;
    return `K8S · ${source.context}${source.namespace ? ` · ${source.namespace}` : ` · ${t("dynamic_discovery.all_namespaces")}`}`;
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-sm" onclick={testDiscovery}>{t("dynamic_discovery.test")}</button>
    <button class="btn btn-accent btn-sm" onclick={startAdd}>{t("dynamic_discovery.new")}</button>
  </div>

  {#if adding}
    {#key addKey}
      <DynamicDiscoverySourceForm
        source={adding}
        {contexts}
        {statuses}
        onRefreshContexts={refreshContextInfo}
        onSave={saveNew}
        onCancel={cancelForm}
      />
    {/key}
  {/if}

  {#each items as source (source.id)}
    {#if editId === source.id}
      <DynamicDiscoverySourceForm
        {source}
        {contexts}
        {statuses}
        onRefreshContexts={refreshContextInfo}
        onSave={saveEdit}
        onCancel={cancelForm}
      />
    {:else}
      <div class="card item-row">
        <div class="item-info" class:dimmed={!source.enabled}>
          <div class="item-icon" class:k8s={source.platform === "k8s"}>{source.platform === "docker" ? "D" : "K"}</div>
          <div class="item-text">
            <div class="item-name">{source.name}</div>
            <div class="item-sub">{sourceMeta(source)}</div>
          </div>
        </div>
        <div class="item-actions">
          <label class="switch" title={t("dynamic_discovery.enabled")}>
            <input type="checkbox" checked={source.enabled} onchange={() => toggleEnabled(source)} />
            <span class="slider"></span>
          </label>
          <button class="btn btn-sm" onclick={() => startEdit(source)}>{t("common.edit")}</button>
          <button class="btn btn-sm btn-danger" onclick={() => remove(source.id)}>{t("common.delete")}</button>
        </div>
      </div>
    {/if}
  {:else}
    {#if !adding}
      <p class="empty">{t("dynamic_discovery.empty")}</p>
    {/if}
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 8px;
    margin-bottom: 16px;
  }
  .item-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 16px;
    gap: 12px;
  }
  .item-info {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    flex: 0 1 auto;
    width: fit-content;
  }
  .item-info.dimmed { opacity: 0.45; }
  .item-icon {
    width: 28px;
    height: 28px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    font-weight: 700;
    font-size: 12px;
    color: var(--accent);
    background: var(--accent-soft);
  }
  .item-icon.k8s {
    color: var(--purple);
    background: color-mix(in srgb, var(--purple) 15%, transparent);
  }
  .item-text { min-width: 0; }
  .item-name {
    font-weight: 600;
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item-sub {
    font-size: 12px;
    color: var(--text-sub);
    font-family: monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item-actions { display: flex; align-items: center; gap: 10px; flex-shrink: 0; }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
  @media (max-width: 640px) {
    .toolbar { justify-content: stretch; flex-wrap: wrap; }
    .item-row { align-items: flex-start; flex-direction: column; }
    .item-actions { align-self: flex-end; }
  }
</style>
