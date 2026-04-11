<script lang="ts">
  import { onMount } from "svelte";
  import * as app from "../stores/app.svelte.ts";
  import type { Snippet } from "../stores/app.svelte.ts";

  let snippets = $state<Snippet[]>([]);
  let focusIdx = $state(0);
  let inputEl: HTMLInputElement;
  let filter = $state("");

  let filtered = $derived(
    filter
      ? snippets.filter(s => s.name.toLowerCase().includes(filter.toLowerCase()) || s.command.toLowerCase().includes(filter.toLowerCase()))
      : snippets
  );

  onMount(async () => {
    snippets = await app.loadSnippets();
    requestAnimationFrame(() => inputEl?.focus());
  });

  function select(s: Snippet) {
    app.sendToTerminal(s.command);
    app.closeSnippetPicker();
  }

  function handleKey(e: KeyboardEvent) {
    if (e.key === "Escape") { app.closeSnippetPicker(); e.preventDefault(); }
    else if (e.key === "ArrowDown") { focusIdx = Math.min(focusIdx + 1, filtered.length - 1); e.preventDefault(); }
    else if (e.key === "ArrowUp") { focusIdx = Math.max(focusIdx - 1, 0); e.preventDefault(); }
    else if (e.key === "Enter" && filtered[focusIdx]) { select(filtered[focusIdx]); e.preventDefault(); }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="picker-backdrop" onclick={() => app.closeSnippetPicker()}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="picker" onclick={(e) => e.stopPropagation()}>
    <input bind:this={inputEl} type="text" bind:value={filter} placeholder="搜索命令片段..."
      onkeydown={handleKey} />
    <div class="picker-list">
      {#each filtered as s, i (s.name)}
        <button class="picker-item" class:focused={focusIdx === i} onclick={() => select(s)}>
          <span class="picker-name">{s.name}</span>
          <span class="picker-cmd">{s.command}</span>
        </button>
      {:else}
        <div class="picker-empty">暂无命令片段</div>
      {/each}
    </div>
  </div>
</div>

<style>
  .picker-backdrop {
    position: fixed; inset: 0; z-index: 400;
    background: rgba(0,0,0,0.4);
    display: flex; align-items: flex-start; justify-content: center;
    padding-top: 80px;
  }
  .picker {
    background: var(--bg);
    box-shadow: var(--raised);
    border-radius: var(--radius);
    width: 400px; max-height: 360px;
    display: flex; flex-direction: column;
    overflow: hidden;
  }
  .picker input {
    margin: 12px;
    flex-shrink: 0;
  }
  .picker-list {
    flex: 1; overflow-y: auto;
    padding: 0 8px 8px;
  }
  .picker-item {
    display: flex; flex-direction: column; gap: 2px;
    width: 100%; padding: 8px 12px; border: none;
    border-radius: var(--radius-sm);
    background: transparent; text-align: left;
    font-family: inherit; cursor: pointer;
    transition: background 0.1s;
  }
  .picker-item:hover, .picker-item.focused { background: var(--surface); }
  .picker-item.focused { outline: 1px solid var(--accent); outline-offset: -1px; }
  .picker-name { font-size: 13px; font-weight: 600; color: var(--text); }
  .picker-cmd { font-size: 11px; font-family: monospace; color: var(--text-sub); }
  .picker-empty { padding: 16px; text-align: center; color: var(--text-dim); font-size: 13px; }
</style>
