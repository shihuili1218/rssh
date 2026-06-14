<script lang="ts">
  import type { HighlightRule } from "../stores/app.svelte.ts";
  import { t } from "../i18n/index.svelte.ts";
  import { validateHighlightRule } from "../terminal/highlight.ts";

  let {
    rule,
    onSave,
    onCancel,
  }: {
    rule: HighlightRule;
    onSave: (rule: HighlightRule) => void;
    onCancel: () => void;
  } = $props();

  let formKw = $state("");
  let formName = $state("");
  let formColor = $state("#FF6B6B");
  let formEnabled = $state(true);
  let formIsRegex = $state(false);
  let formIsCaseSensitive = $state(false);

  function loadFromRule(r: HighlightRule) {
    formKw = r.keyword ?? "";
    formName = r.is_regex ? (r.name ?? "") : "";
    formColor = r.color || "#FF6B6B";
    formEnabled = r.enabled ?? true;
    formIsRegex = r.is_regex ?? false;
    formIsCaseSensitive = r.is_case_sensitive ?? false;
  }

  $effect(() => {
    loadFromRule(rule);
  });

  const finalRule = $derived<HighlightRule>({
    keyword: formKw.trim(),
    name: formName.trim(),
    color: formColor,
    enabled: formEnabled,
    is_regex: formIsRegex,
    is_case_sensitive: formIsCaseSensitive,
  });

  const formError = $derived(validateHighlightRule(finalRule));

  function toggleRegex() {
    formIsRegex = !formIsRegex;
    if (!formIsRegex) {
      formName = "";
    }
  }

  function toggleCaseSensitive() {
    formIsCaseSensitive = !formIsCaseSensitive;
  }

  function handleSave() {
    if (!finalRule.keyword || formError) return;
    onSave(finalRule);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") handleSave();
  }
</script>

<div class="card inline-form">
  <div class="field">
    <span class="label-text">{t("highlight.keyword")}</span>
    <div class="keyword-row">
      <input type="text" bind:value={formKw} placeholder={t("highlight.keyword_placeholder")}
        onkeydown={handleKeydown} />
      <button type="button" class="btn btn-sm" class:btn-accent={formIsRegex}
        aria-pressed={formIsRegex}
        onclick={toggleRegex}>
        {t("highlight.regex")}
      </button>
      <button type="button" class="btn btn-sm" class:btn-accent={formIsCaseSensitive}
        aria-pressed={formIsCaseSensitive}
        onclick={toggleCaseSensitive}>
        {t("highlight.case_sensitive")}
      </button>
    </div>
  </div>

  {#if formIsRegex}
    <label class="field">
      <span class="label-text">{t("highlight.name")}</span>
      <input type="text" bind:value={formName} placeholder={t("highlight.name_placeholder")}
        onkeydown={handleKeydown} />
    </label>
    <p class="hint">{t("highlight.regex_hint")}</p>
  {/if}

  <div class="color-actions-row">
    <label class="color-picker">
      <span class="label-text">{t("common.color")}</span>
      <div class="color-row">
        <input type="color" bind:value={formColor} />
        <span class="color-hex">{formColor}</span>
      </div>
    </label>

    <div class="form-actions">
      <button class="btn btn-accent btn-sm" onclick={handleSave} disabled={!formKw.trim() || !!formError}>
        {t("common.save")}
      </button>
      <button class="btn btn-sm" onclick={onCancel}>{t("common.cancel")}</button>
    </div>
  </div>

  {#if formError}
    <div class="form-error">
      {#if formError.kind === "zero_width"}
        {t("error.highlight_regex_zero_width")}
      {:else if formError.kind === "name_too_long"}
        {t("error.highlight_name_too_long", { max: 100 })}
      {:else}
        {t("error.highlight_invalid_regex", { err: formError.message })}
      {/if}
    </div>
  {/if}
</div>

<style>
  .inline-form {
    display: flex; flex-direction: column; gap: 12px;
    padding: 16px; margin-bottom: 12px;
  }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .label-text { font-size: 12px; color: var(--text-sub); }

  .keyword-row {
    display: flex; align-items: center; gap: 8px; flex-wrap: wrap;
  }
  .keyword-row input[type="text"] {
    flex: 1; min-width: 160px;
    font: inherit; font-size: 13px;
  }
  .keyword-row .btn {
    white-space: nowrap; flex-shrink: 0;
  }

  .inline-form input[type="text"] {
    width: 100%; box-sizing: border-box;
  }

  .color-actions-row {
    display: flex; align-items: flex-end; justify-content: space-between; gap: 12px; flex-wrap: wrap;
  }
  .color-picker { display: flex; flex-direction: column; gap: 4px; }
  .color-row { display: flex; align-items: center; gap: 10px; }
  .color-row input[type="color"] {
    width: 48px; height: 32px; padding: 2px;
    border: 1px solid var(--divider); border-radius: 4px;
    cursor: pointer; box-shadow: none;
  }
  .color-hex { font-size: 12px; font-family: monospace; color: var(--text-dim); }

  .form-actions {
    display: flex; gap: 10px; margin-left: auto; align-items: center;
  }

  .hint {
    font-size: 11px; color: var(--text-dim); margin: 0;
  }

  .form-error {
    font-size: 12px; color: var(--error, #ff6b6b);
    background: rgba(255, 107, 107, 0.08);
    padding: 6px 10px; border-radius: 4px;
  }
</style>
