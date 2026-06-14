<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import type { HighlightRule } from "../stores/app.svelte.ts";
  import * as app from "../stores/app.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { t, errMsg } from "../i18n/index.svelte.ts";
  import HighlightRuleForm from "./HighlightRuleForm.svelte";

  const EMPTY_RULE: HighlightRule = {
    keyword: "",
    name: "",
    color: "#FF6B6B",
    enabled: true,
    is_regex: false,
    is_case_sensitive: false,
  };

  let items = $state<HighlightRule[]>([]);
  let adding = $state(false);
  let addKey = $state(0);
  // Edit identity = keyword as currently stored on the backend (rename uses old → new).
  let editKw = $state<string | null>(null);

  onMount(refresh);

  async function refresh() {
    items = await app.loadHighlights();
    // Tell open TerminalPanes their highlight regex is stale. Local-only
    // bump (no backend round-trip) — TerminalPane's $effect re-reads the
    // DB and recompiles its regex. Without this, edits here only take
    // effect after the next terminal reconnect.
    app.bumpHighlights();
  }

  function startAdd() {
    adding = true;
    addKey += 1;
    editKw = null;
  }

  function startEdit(h: HighlightRule) {
    adding = false;
    editKw = h.keyword;
  }

  function cancelForm() {
    adding = false;
    editKw = null;
  }

  async function saveNew(rule: HighlightRule) {
    try {
      await invoke("add_highlight", { rule });
      adding = false;
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.add")}: ${errMsg(e)}`); }
  }

  async function saveEdit(rule: HighlightRule) {
    if (editKw === null) return;
    try {
      await invoke("update_highlight", {
        oldKeyword: editKw,
        rule,
      });
      editKw = null;
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
  }

  // Flip enabled in place — reuses update_highlight so no extra backend command.
  // refresh() runs regardless: it re-syncs the (controlled) checkbox to the DB
  // truth, so a rejected toggle snaps back instead of lying.
  async function toggleEnabled(h: HighlightRule) {
    try {
      await invoke("update_highlight", {
        oldKeyword: h.keyword,
        rule: { ...h, enabled: !h.enabled },
      });
    } catch (e: any) { toast.error(`${t("toast.error.save")}: ${errMsg(e)}`); }
    await refresh();
  }

  async function remove(keyword: string) {
    try {
      await invoke("remove_highlight", { keyword });
      if (editKw === keyword) editKw = null;
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.delete")}: ${errMsg(e)}`); }
  }

  async function resetDefaults() {
    try {
      await invoke("reset_highlights");
      cancelForm();
      await refresh();
    } catch (e: any) { toast.error(`${t("toast.error.reset")}: ${errMsg(e)}`); }
  }

  function displayTitle(h: HighlightRule): string {
    return h.is_regex && h.name ? h.name : h.keyword;
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-sm" onclick={resetDefaults}>{t("highlight.reset_defaults")}</button>
    <button class="btn btn-accent btn-sm" onclick={startAdd}>{t("highlight.new")}</button>
  </div>

  {#if adding}
    {#key addKey}
      <HighlightRuleForm rule={EMPTY_RULE} onSave={saveNew} onCancel={cancelForm} />
    {/key}
  {/if}

  {#each items as h (h.keyword)}
    {#if editKw === h.keyword}
      <HighlightRuleForm rule={h} onSave={saveEdit} onCancel={cancelForm} />
    {:else}
      <div class="card item-row">
        <div class="item-info" class:dimmed={!h.enabled}>
          <span class="color-swatch" style="background: {h.color}"></span>
          <div class="item-text">
            <div class="item-name" title={h.keyword}>{displayTitle(h)}</div>
            <div class="item-meta">
              <div class="item-sub">{h.color}</div>
              {#if h.is_regex || h.is_case_sensitive}
                <div class="item-tags">
                  {#if h.is_regex}<span class="tag">{t("highlight.tag_regex")}</span>{/if}
                  {#if h.is_case_sensitive}<span class="tag">{t("highlight.tag_case_sensitive")}</span>{/if}
                </div>
              {/if}
            </div>
          </div>
        </div>
        <div class="item-actions">
          <label class="switch" title={t("highlight.enabled")}>
            <input type="checkbox" checked={h.enabled} onchange={() => toggleEnabled(h)} />
            <span class="slider"></span>
          </label>
          <button class="btn btn-sm" onclick={() => startEdit(h)}>{t("common.edit")}</button>
          <button class="btn btn-sm btn-danger" onclick={() => remove(h.keyword)}>{t("common.delete")}</button>
        </div>
      </div>
    {/if}
  {:else}
    {#if !adding}
      <p class="empty">{t("highlight.empty")}</p>
    {/if}
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; gap: 8px; margin-bottom: 16px; }
  .item-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 16px;
    gap: 12px;
  }
  .item-info { display: flex; align-items: center; gap: 10px; min-width: 0; flex: 1; }
  .item-info.dimmed { opacity: 0.45; }
  .item-text { min-width: 0; }
  .item-name {
    font-weight: 600; font-size: 14px; font-family: monospace;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .item-meta { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .item-sub { font-size: 12px; color: var(--text-sub); font-family: monospace; }
  .item-actions { display: flex; align-items: center; gap: 10px; flex-shrink: 0; }
  .color-swatch {
    width: 20px; height: 20px; border-radius: 4px; flex-shrink: 0;
    border: 1px solid var(--divider);
  }
  .item-tags { display: flex; gap: 6px; }
  .tag {
    font-size: 10px; font-weight: 600; color: var(--text-dim);
    border: 1px solid var(--divider); border-radius: 3px;
    padding: 1px 4px; font-family: monospace;
  }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
