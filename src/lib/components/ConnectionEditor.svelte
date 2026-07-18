<script lang="ts">
  import * as app from "../stores/app.svelte.ts";
  import type { ConnectionKind } from "../stores/app.svelte.ts";
  import { t } from "../i18n/index.svelte.ts";
  import { availableConnectionKinds } from "./connection-editor.ts";
  import ProfileEditor from "./ProfileEditor.svelte";
  import ForwardEditor from "./ForwardEditor.svelte";
  import SerialProfileEditor from "./SerialProfileEditor.svelte";
  import TelnetProfileEditor from "./TelnetProfileEditor.svelte";
  import AppIcon from "./AppIcon.svelte";
  import { connectionIconName } from "./app-icon";

  const intent = app.connectionEditorIntent();
  const locked = app.connectionTypeLocked();
  const updateId = app.connectionUpdateId();
  const copyFromId = intent.mode === "copy" ? intent.sourceId : null;
  let selectedKind = $state<ConnectionKind>(intent.kind);

  let typeOptions = $derived(availableConnectionKinds(app.isMobile).map((kind) => ({
    kind,
    label: kindLabel(kind),
    description: kindDescription(kind),
    icon: connectionIconName(kind),
  })));

  function kindLabel(kind: ConnectionKind): string {
    switch (kind) {
      case "ssh": return t("connection.type.ssh");
      case "forward": return t("connection.type.forward");
      case "serial": return t("connection.type.serial");
      case "telnet": return t("connection.type.telnet");
    }
  }

  function kindDescription(kind: ConnectionKind): string {
    switch (kind) {
      case "ssh": return t("connection.type.ssh_description");
      case "forward": return t("connection.type.forward_description");
      case "serial": return t("connection.type.serial_description");
      case "telnet": return t("connection.type.telnet_description");
    }
  }

</script>

<div class="page">
  <div class="card editor-card">
    <fieldset
      class="type-picker"
      disabled={locked}
      aria-describedby={locked ? "connection-type-locked" : undefined}
    >
      <legend class="type-legend">{t("connection.type")}</legend>
      <div class="type-grid">
        {#each typeOptions as option (option.kind)}
          <label class="type-option">
            <input type="radio" name="connection-type" value={option.kind} bind:group={selectedKind} />
            <span class="type-card">
              <span class="type-icon"><AppIcon name={option.icon} size={17} /></span>
              <span class="type-copy">
                <span class="type-title">{option.label}</span>
                <span class="type-sub">{option.description}</span>
              </span>
            </span>
          </label>
        {/each}
      </div>
      {#if locked}
        <p id="connection-type-locked" class="type-help">{t("connection.type_locked")}</p>
      {/if}
    </fieldset>

    {#key selectedKind}
      {#if selectedKind === "ssh"}
        <ProfileEditor id={updateId} {copyFromId} />
      {:else if selectedKind === "forward"}
        <ForwardEditor id={updateId} {copyFromId} />
      {:else if selectedKind === "serial"}
        <SerialProfileEditor id={updateId} {copyFromId} />
      {:else}
        <TelnetProfileEditor id={updateId} {copyFromId} />
      {/if}
    {/key}
  </div>
</div>

<style>
  .page { padding: 24px; }
  .editor-card {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px;
    margin-bottom: 12px;
  }
  .type-picker { min-width: 0; margin: 0; padding: 0; border: 0; }
  .type-legend {
    margin-bottom: 8px;
    color: var(--text-sub);
    font-size: 12px;
    font-weight: 600;
  }
  .type-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 10px;
  }
  .type-option {
    position: relative;
    min-width: 0;
    color: inherit;
    cursor: pointer;
    text-transform: none;
  }
  .type-option > input {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: 0;
    opacity: 0;
    clip-path: inset(50%);
    overflow: hidden;
  }
  .type-card {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    padding: 10px;
    border: 1px solid var(--divider);
    border-radius: var(--radius-sm);
    background: var(--bg);
    color: var(--text);
    transition: border-color 0.15s, background 0.15s, color 0.15s;
  }
  .type-option:hover > input:not(:disabled) + .type-card { background: var(--surface); }
  .type-option > input:checked + .type-card {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, var(--bg));
    color: var(--accent);
  }
  .type-option > input:focus-visible + .type-card {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .type-option > input:disabled + .type-card { cursor: default; opacity: 0.45; }
  .type-option > input:disabled:checked + .type-card { opacity: 1; }
  .type-icon {
    display: grid;
    place-items: center;
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    border-radius: 6px;
    background: var(--accent-soft);
    font-size: 12px;
    font-weight: 700;
  }
  .type-copy { display: flex; min-width: 0; flex-direction: column; gap: 2px; }
  .type-title { overflow: hidden; font-size: 13px; font-weight: 650; text-overflow: ellipsis; white-space: nowrap; }
  .type-sub { overflow: hidden; color: var(--text-sub); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
  .type-help { margin: 8px 0 0; color: var(--text-dim); font-size: 11px; }

  @media (max-width: 760px) {
    .type-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
  @media (max-width: 480px) {
    .type-grid { grid-template-columns: 1fr; }
  }
</style>
