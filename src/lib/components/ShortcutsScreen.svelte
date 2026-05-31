<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { t } from "../i18n/index.svelte.ts";
  import {
    ACTIONS,
    collidingAction,
    defaultBinding,
    eventToBinding,
    isModifierKey,
    reservedConflict,
    validateBinding,
    type ActionId,
    type KeyBinding,
    type Surface,
  } from "../keyboard/keymap.ts";
  import * as keymap from "../stores/keymap.svelte.ts";

  onMount(() => { keymap.init(); });

  // $derived so headings re-translate when the locale changes while mounted.
  const groups = $derived<{ surface: Surface; title: string }[]>([
    { surface: "global", title: t("shortcuts.section.global") },
    { surface: "terminal", title: t("shortcuts.section.terminal") },
  ]);
  const actionsBySurface = (s: Surface) => ACTIONS.filter((a) => a.surface === s);

  // Fixed, non-customizable shortcuts — shown read-only for discoverability.
  // (Stateful interactions / "any key" handlers; they aren't key→action data.)
  const fixed = $derived<[string, string][]>([
    ["Ctrl+Tab", t("shortcuts.next_tab")],
    ["Ctrl+Shift+Tab", t("shortcuts.prev_tab")],
    ["Esc", t("shortcuts.close_overlay")],
    ["↑ ↓ ← →", t("shortcuts.select_card")],
    ["Enter", t("shortcuts.connect_card")],
    ["Any key", t("shortcuts.reconnect_disconnect")],
    ["Any key", t("shortcuts.reconnect_error")],
  ]);

  let recordingId = $state<ActionId | null>(null);
  let recordError = $state<string | null>(null);
  let rowError = $state<{ id: ActionId; msg: string } | null>(null);
  let detach: (() => void) | null = null;

  const conflictGroups = $derived(keymap.conflicts());
  function conflictPartner(id: ActionId): ActionId | null {
    const g = conflictGroups.find((grp) => grp.includes(id));
    return g ? (g.find((x) => x !== id) ?? null) : null;
  }
  function labelOf(id: ActionId): string {
    const a = ACTIONS.find((x) => x.id === id);
    return a ? t(a.labelKey) : id;
  }

  /** Label of whatever already owns this combo (a fixed shortcut or another
   *  action, excluding `id`), or null if the combo is free. */
  function recordClashLabel(id: ActionId, b: KeyBinding): string | null {
    const reserved = reservedConflict(b);
    if (reserved) return t(reserved);
    const other = collidingAction(id, b, keymap.effective());
    return other ? labelOf(other) : null;
  }

  function startRecord(id: ActionId) {
    if (recordingId) stopRecord();
    recordError = null;
    rowError = null;
    recordingId = id;
    keymap.setRecording(true);
    const onKey = (e: KeyboardEvent) => {
      // Wait for a real key — ignore lone modifier presses.
      if (isModifierKey(e.key)) return;
      e.preventDefault();
      e.stopPropagation();
      e.stopImmediatePropagation();
      const bare = !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey;
      if (e.key === "Escape" && bare) { stopRecord(); return; }
      const b = eventToBinding(e);
      if (!validateBinding(b).ok) { recordError = t("shortcuts.customize.need_modifier"); return; }
      const clash = recordClashLabel(id, b);
      if (clash) { recordError = t("shortcuts.customize.conflict", { name: clash }); return; }
      keymap.setOverride(id, b);
      stopRecord();
    };
    window.addEventListener("keydown", onKey, { capture: true });
    detach = () => window.removeEventListener("keydown", onKey, { capture: true });
  }

  function stopRecord() {
    detach?.();
    detach = null;
    recordingId = null;
    keymap.setRecording(false);
  }

  // Reset reverts to the default — guard it with the SAME collision check as
  // recording, so reverting can never silently duplicate another action's combo.
  function tryReset(id: ActionId) {
    const other = collidingAction(id, defaultBinding(id, keymap.isMac), keymap.effective());
    if (other) {
      rowError = { id, msg: t("shortcuts.customize.conflict", { name: labelOf(other) }) };
      return;
    }
    rowError = null;
    keymap.reset(id);
  }

  onDestroy(stopRecord);
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-sm" onclick={() => { rowError = null; keymap.resetAll(); }}>{t("shortcuts.customize.reset_all")}</button>
  </div>

  {#each groups as g}
    <div class="section-label">{g.title}</div>
    <div class="key-list">
      {#each actionsBySurface(g.surface) as a (a.id)}
        {@const partner = conflictPartner(a.id)}
        <div class="key-row">
          <div class="row-main">
            <kbd class="kbd surface-pressed" class:conflict={!!partner}>{keymap.format(a.id)}</kbd>
            <span class="key-desc">{t(a.labelKey)}</span>
          </div>
          <div class="row-actions">
            {#if recordingId === a.id}
              <span class="recording">{recordError ?? t("shortcuts.customize.recording")}</span>
            {:else}
              {#if rowError?.id === a.id}
                <span class="conflict-text">{rowError.msg}</span>
              {:else if partner}
                <span class="conflict-text">{t("shortcuts.customize.conflict", { name: labelOf(partner) })}</span>
              {/if}
              {#if keymap.isOverridden(a.id)}
                <button class="btn btn-sm" onclick={() => tryReset(a.id)}>{t("shortcuts.customize.reset")}</button>
              {/if}
              <button class="btn btn-sm btn-accent" onclick={() => startRecord(a.id)}>{t("shortcuts.customize.record")}</button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/each}

  <div class="section-label">{t("shortcuts.customize.fixed")}</div>
  <div class="key-list">
    {#each fixed as [key, desc]}
      <div class="key-row">
        <div class="row-main">
          <kbd class="kbd surface-pressed">{key}</kbd>
          <span class="key-desc">{desc}</span>
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 4px; }
  .section-label {
    font-size: 12px; font-weight: 600; color: var(--text-sub);
    text-transform: uppercase; letter-spacing: 0.04em;
    margin-top: 18px; margin-bottom: 4px;
  }
  .key-list { display: flex; flex-direction: column; gap: 2px; }
  .key-row {
    display: flex; align-items: center; justify-content: space-between;
    gap: 12px; padding: 8px 0; min-height: 34px;
  }
  .row-main { display: flex; align-items: center; gap: 12px; min-width: 0; }
  .row-actions { display: flex; align-items: center; gap: 10px; flex-shrink: 0; }
  .kbd {
    display: inline-block;
    min-width: 80px;
    padding: calc(3px * var(--density)) calc(8px * var(--density));
    background: var(--surface);
    box-shadow: var(--pressed);
    border-radius: 6px;
    font-family: monospace;
    font-size: 12px;
    color: var(--text);
    text-align: center;
  }
  .kbd.conflict { color: var(--danger, #e05252); box-shadow: inset 0 0 0 1px var(--danger, #e05252); }
  .key-desc { font-size: 13px; color: var(--text-sub); }
  .recording { font-size: 12px; color: var(--accent, var(--text)); font-style: italic; }
  .conflict-text { font-size: 12px; color: var(--danger, #e05252); }
</style>
