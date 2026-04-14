<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { EditorView, basicSetup } from "codemirror";
  import { keymap } from "@codemirror/view";
  import { EditorState } from "@codemirror/state";
  import { indentWithTab } from "@codemirror/commands";
  import { StreamLanguage } from "@codemirror/language";
  import { shell } from "@codemirror/legacy-modes/mode/shell";
  import { oneDark } from "@codemirror/theme-one-dark";
  import * as app from "../stores/app.svelte.ts";

  let { tabId }: { tabId: string } = $props();

  let editorEl: HTMLDivElement;
  let view: EditorView;

  let sessions = $derived(app.connectedSessions());
  let selectedTabIds = $state<Set<string>>(new Set());

  $effect(() => {
    const activeIds = new Set(sessions.map(s => s.tabId));
    const pruned = [...selectedTabIds].filter(id => activeIds.has(id));
    if (pruned.length !== selectedTabIds.size) selectedTabIds = new Set(pruned);
  });

  function toggle(tid: string) {
    const next = new Set(selectedTabIds);
    if (next.has(tid)) next.delete(tid);
    else next.add(tid);
    selectedTabIds = next;
  }

  function selectAll() { selectedTabIds = new Set(sessions.map(s => s.tabId)); }
  function selectNone() { selectedTabIds = new Set(); }

  function broadcast() {
    const text = view.state.doc.toString();
    if (!text.trim() || selectedTabIds.size === 0) return;
    app.broadcastToSessions([...selectedTabIds], text + "\n");
  }

  const appTheme = EditorView.theme({
    "&": { height: "100%", backgroundColor: "var(--bg)" },
    ".cm-scroller": { overflow: "auto" },
    ".cm-gutters": { backgroundColor: "var(--bg)", borderRight: "1px solid var(--divider)" },
    ".cm-activeLineGutter": { backgroundColor: "var(--surface)" },
    ".cm-activeLine": { backgroundColor: "var(--surface)" },
    "&.cm-focused .cm-cursor": { borderLeftColor: "var(--accent)" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": {
      backgroundColor: "rgba(74,108,247,0.3)",
    },
  }, { dark: true });

  onMount(() => {
    view = new EditorView({
      state: EditorState.create({
        doc: "",
        extensions: [
          basicSetup,
          keymap.of([indentWithTab]),
          StreamLanguage.define(shell),
          oneDark,
          appTheme,
          EditorView.lineWrapping,
        ],
      }),
      parent: editorEl,
    });
    view.focus();
  });

  onDestroy(() => { view?.destroy(); });
</script>

<div class="edit-pane">
  <div class="editor-area" bind:this={editorEl}></div>

  <div class="session-panel">
    <div class="panel-header">Target Sessions</div>

    {#if sessions.length === 0}
      <div class="empty-hint">No connected sessions</div>
    {:else}
      <div class="session-list">
        {#each sessions as s (s.tabId)}
          <label class="session-item">
            <input type="checkbox" checked={selectedTabIds.has(s.tabId)} onchange={() => toggle(s.tabId)} />
            <span class="session-type">{s.type === "local" ? "$" : "SSH"}</span>
            <span class="session-label">{s.label}</span>
          </label>
        {/each}
      </div>
      <div class="select-actions">
        <button class="link-btn" onclick={selectAll}>All</button>
        <button class="link-btn" onclick={selectNone}>None</button>
      </div>
    {/if}

    <button
      class="broadcast-btn"
      disabled={selectedTabIds.size === 0}
      onclick={broadcast}
    >
      Broadcast ({selectedTabIds.size})
    </button>
  </div>
</div>

<style>
  .edit-pane {
    display: flex;
    height: 100%;
    width: 100%;
  }

  .editor-area {
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }

  .editor-area :global(.cm-editor) {
    height: 100%;
  }

  .session-panel {
    width: 200px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--divider);
    background: var(--bg);
    padding: 12px;
    gap: 8px;
  }

  .panel-header {
    font-size: 13px;
    font-weight: 700;
    color: var(--text);
    padding-bottom: 4px;
    border-bottom: 1px solid var(--divider);
  }

  .empty-hint {
    font-size: 12px;
    color: var(--text-dim);
    padding: 8px 0;
  }

  .session-list {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .session-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-size: 12px;
    color: var(--text-sub);
    transition: background 0.1s;
  }
  .session-item:hover { background: var(--surface); color: var(--text); }

  .session-item input[type="checkbox"] {
    accent-color: var(--accent);
  }

  .session-type {
    font-size: 10px;
    font-weight: 700;
    color: var(--accent);
    min-width: 28px;
  }

  .session-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .select-actions {
    display: flex;
    gap: 8px;
    padding: 4px 0;
  }

  .link-btn {
    background: none;
    border: none;
    color: var(--accent);
    font-size: 12px;
    font-family: inherit;
    cursor: pointer;
    padding: 0;
  }
  .link-btn:hover { text-decoration: underline; }

  .broadcast-btn {
    margin-top: auto;
    padding: 10px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: #fff;
    font-family: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: opacity 0.15s;
  }
  .broadcast-btn:hover { opacity: 0.9; }
  .broadcast-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
