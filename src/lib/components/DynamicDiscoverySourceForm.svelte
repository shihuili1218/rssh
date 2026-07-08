<script lang="ts">
  import type {
    DynamicDiscoveryContext,
    DynamicDiscoverySource,
    DynamicDiscoveryToolStatus,
    DynamicPlatform,
  } from "../stores/app.svelte.ts";
  import { t } from "../i18n/index.svelte.ts";
  import SearchSelect from "./SearchSelect.svelte";

  let {
    source,
    contexts,
    statuses,
    onRefreshContexts,
    onSave,
    onCancel,
  }: {
    source: DynamicDiscoverySource;
    contexts: DynamicDiscoveryContext[];
    statuses: Partial<Record<DynamicPlatform, DynamicDiscoveryToolStatus>>;
    onRefreshContexts: (platform: DynamicPlatform) => void | Promise<void>;
    onSave: (source: DynamicDiscoverySource) => void;
    onCancel: () => void;
  } = $props();

  let formId = $state("");
  let formPlatform = $state<DynamicPlatform>("docker");
  let formName = $state("");
  let formEnabled = $state(true);
  let formContext = $state("");
  let formNamespace = $state("");
  let formShell = $state("sh");
  let refreshingContexts = $state(false);
  let loadedSourceId = $state("");

  function loadFromSource(s: DynamicDiscoverySource) {
    formId = s.id;
    formPlatform = s.platform;
    formName = s.name ?? "";
    formEnabled = s.enabled ?? true;
    formContext = s.context ?? "";
    formShell = s.shell || "sh";
    formNamespace = s.platform === "k8s" ? (s.namespace ?? "") : "";
  }

  $effect(() => {
    if (source.id === loadedSourceId) return;
    loadedSourceId = source.id;
    loadFromSource(source);
  });

  const cliOptions: { platform: DynamicPlatform; label: string; command: string; icon: string }[] = [
    { platform: "docker", label: "Docker CLI", command: "docker", icon: "D" },
    { platform: "k8s", label: "kubectl CLI", command: "kubectl", icon: "K" },
  ];

  const contextSelectId = $derived(`dynamic-context-select-${formId || "new"}`);
  const contextOptions = $derived(
    contexts.filter((c) => c.platform === formPlatform),
  );
  const contextSelectOptions = $derived(
    contextOptions.map((ctx) => ({
      value: ctx.name,
      label: ctx.current ? `${ctx.name} (${t("dynamic_discovery.current")})` : ctx.name,
    })),
  );
  const formError = $derived(
    !cliReady(formPlatform)
      ? t("common.loading")
      : !cliAvailable(formPlatform)
      ? t("dynamic_discovery.error.cli_unavailable", { cli: cliCommand(formPlatform) })
      : !formName.trim()
      ? t("dynamic_discovery.error.name_required")
      : !formContext.trim()
        ? t("dynamic_discovery.error.context_required")
        : !formShell.trim()
          ? t("dynamic_discovery.error.shell_required")
          : "",
  );

  const finalSource = $derived<DynamicDiscoverySource>(
    formPlatform === "docker"
      ? {
          id: formId,
          name: formName.trim(),
          enabled: formEnabled,
          platform: "docker",
          context: formContext.trim(),
          shell: formShell.trim(),
        }
      : {
          id: formId,
          name: formName.trim(),
          enabled: formEnabled,
          platform: "k8s",
          context: formContext.trim(),
          namespace: formNamespace.trim() || null,
          shell: formShell.trim(),
        },
  );

  $effect(() => {
    const options = contextOptions;
    if (!formContext.trim() && options.length > 0) {
      formContext = defaultContext(formPlatform);
    }
  });

  function cliAvailable(platform: DynamicPlatform): boolean {
    return statuses[platform]?.available === true;
  }

  function cliReady(platform: DynamicPlatform): boolean {
    return !!statuses[platform];
  }

  function cliCommand(platform: DynamicPlatform): string {
    return platform === "docker" ? "docker" : "kubectl";
  }

  function cliStatus(platform: DynamicPlatform): string {
    const st = statuses[platform];
    if (!st) return t("common.loading");
    return st.available ? t("dynamic_discovery.cli_available") : t("dynamic_discovery.cli_missing");
  }

  function defaultContext(platform: DynamicPlatform): string {
    return contexts.find((c) => c.platform === platform && c.current)?.name
      ?? contexts.find((c) => c.platform === platform)?.name
      ?? "";
  }

  function defaultName(platform: DynamicPlatform): string {
    return platform === "docker" ? "Docker" : "K8S";
  }

  function choosePlatform(platform: DynamicPlatform) {
    if (!cliAvailable(platform) || platform === formPlatform) return;
    const previousDefaultName = defaultName(formPlatform);
    formPlatform = platform;
    formContext = defaultContext(platform);
    formNamespace = "";
    if (!formName.trim() || formName === previousDefaultName || formName === "Docker" || formName === "K8S") {
      formName = defaultName(platform);
    }
  }

  async function refreshContexts() {
    if (refreshingContexts || !cliAvailable(formPlatform)) return;
    refreshingContexts = true;
    try {
      await onRefreshContexts(formPlatform);
    } finally {
      refreshingContexts = false;
    }
  }

  function handleSave() {
    if (formError) return;
    onSave(finalSource);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") handleSave();
  }
</script>

<div class="card inline-form">
  <div class="form-head">
    <div class="cli-grid" aria-label={t("dynamic_discovery.cli")} role="group">
      {#each cliOptions as cli (cli.platform)}
        {@const available = cliAvailable(cli.platform)}
        <button
          type="button"
          class="cli-card"
          class:active={formPlatform === cli.platform}
          class:k8s={cli.platform === "k8s"}
          disabled={!available}
          aria-pressed={formPlatform === cli.platform}
          onclick={() => choosePlatform(cli.platform)}
        >
          <span class="cli-icon">{cli.icon}</span>
          <span class="cli-text">
            <span class="cli-title">{cli.label}</span>
            <span class="cli-sub">{cli.command} · {cliStatus(cli.platform)}</span>
          </span>
        </button>
      {/each}
    </div>
    <label class="switch" title={t("dynamic_discovery.enabled")}>
      <input type="checkbox" bind:checked={formEnabled} />
      <span class="slider"></span>
    </label>
  </div>

  <label class="field">
    <span class="label-text">{t("common.name")}</span>
    <input type="text" bind:value={formName} placeholder={t("dynamic_discovery.name_placeholder")} onkeydown={handleKeydown} />
  </label>

  <div class="field">
    <label class="label-text" for={contextSelectId}>{t("dynamic_discovery.context")}</label>
    <div class="context-row">
      <SearchSelect
        id={contextSelectId}
        bind:value={formContext}
        options={contextSelectOptions}
        allowCustom
        ariaLabel={t("dynamic_discovery.context")}
        placeholder={t("dynamic_discovery.context_placeholder")}
        searchPlaceholder={t("dynamic_discovery.context_placeholder")}
        emptyText={t("dynamic_discovery.context_empty")}
      />
      <button
        type="button"
        class="btn btn-sm"
        onclick={refreshContexts}
        disabled={refreshingContexts || !cliAvailable(formPlatform)}
      >
        {t("dynamic_discovery.refresh_contexts")}
      </button>
    </div>
  </div>

  {#if formPlatform === "k8s"}
    <label class="field">
      <span class="label-text">{t("dynamic_discovery.namespace")}</span>
      <input type="text" bind:value={formNamespace} placeholder={t("dynamic_discovery.namespace_placeholder")} onkeydown={handleKeydown} />
    </label>
  {/if}

  <label class="field">
    <span class="label-text">{t("dynamic_discovery.shell")}</span>
    <input type="text" bind:value={formShell} placeholder="sh" onkeydown={handleKeydown} />
  </label>

  <div class="form-actions">
    <button class="btn btn-accent btn-sm" onclick={handleSave} disabled={!!formError}>{t("common.save")}</button>
    <button class="btn btn-sm" onclick={onCancel}>{t("common.cancel")}</button>
  </div>

  {#if formError}
    <div class="form-error">{formError}</div>
  {/if}
</div>

<style>
  .inline-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    margin-bottom: 12px;
  }
  .form-head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 12px;
  }
  .cli-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    flex: 0 1 auto;
    width: fit-content;
    max-width: 100%;
    min-width: 0;
  }
  .cli-card {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 0 0 auto;
    width: max-content;
    max-width: 100%;
    padding: 10px;
    border: 1px solid var(--divider);
    border-radius: var(--radius-sm);
    background: var(--bg);
    color: var(--text);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
    transition: border-color 0.15s, background 0.15s, color 0.15s;
  }
  .cli-card:hover:not(:disabled):not(.active) {
    background: var(--surface);
  }
  .cli-card.active {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, var(--bg));
    color: var(--accent);
  }
  .cli-card.k8s.active {
    border-color: var(--purple);
    background: color-mix(in srgb, var(--purple) 12%, var(--bg));
    color: var(--purple);
  }
  .cli-card:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .cli-icon {
    width: 28px;
    height: 28px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    color: var(--accent);
    background: var(--accent-soft);
    font-size: 12px;
    font-weight: 700;
  }
  .cli-card.k8s .cli-icon {
    color: var(--purple);
    background: color-mix(in srgb, var(--purple) 15%, transparent);
  }
  .cli-text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .cli-title {
    font-size: 13px;
    font-weight: 650;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .cli-sub {
    font-size: 11px;
    color: var(--text-sub);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .label-text { font-size: 12px; color: var(--text-sub); }
  .context-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .context-row :global(.search-select) {
    flex: 1;
    min-width: 0;
  }
  .context-row .btn {
    flex-shrink: 0;
  }
  .inline-form input[type="text"] {
    width: 100%;
    box-sizing: border-box;
  }
  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    align-items: center;
  }
  .form-error {
    font-size: 12px;
    color: var(--error, #ff6b6b);
    background: rgba(255, 107, 107, 0.08);
    padding: 6px 10px;
    border-radius: 4px;
  }
  @media (max-width: 640px) {
    .form-head {
      flex-direction: column;
    }
    .cli-grid {
      width: 100%;
    }
    .form-head .switch {
      align-self: flex-end;
    }
    .context-row {
      align-items: stretch;
      flex-direction: column;
    }
  }
</style>
