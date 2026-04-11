<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { HighlightRule } from "../stores/app.svelte.ts";

  const COLORS: { name: string; css: string }[] = [
    { name: "brightRed",     css: "#FF5555" },
    { name: "red",           css: "#CC3333" },
    { name: "brightYellow",  css: "#FFFF44" },
    { name: "yellow",        css: "#CCCC00" },
    { name: "brightGreen",   css: "#00FF41" },
    { name: "green",         css: "#00CC33" },
    { name: "brightCyan",    css: "#44FFFF" },
    { name: "cyan",          css: "#00CCCC" },
    { name: "brightBlue",    css: "#4488FF" },
    { name: "blue",          css: "#0066CC" },
    { name: "brightMagenta", css: "#FF44FF" },
    { name: "magenta",       css: "#CC00CC" },
    { name: "brightWhite",   css: "#FFFFFF" },
    { name: "white",         css: "#CCCCCC" },
  ];

  let items = $state<HighlightRule[]>([]);
  let newKw = $state("");
  let newColor = $state("brightRed");

  onMount(refresh);
  async function refresh() { items = await app.loadHighlights(); }

  function colorCss(name: string): string {
    return COLORS.find(c => c.name === name)?.css ?? "#888";
  }

  async function add() {
    if (!newKw.trim()) return;
    try {
      await invoke("add_highlight", { rule: { keyword: newKw.trim(), color: newColor, enabled: true } });
      newKw = "";
      await refresh();
    } catch (e: any) { alert("添加失败: " + String(e)); }
  }

  async function remove(kw: string) {
    await invoke("remove_highlight", { keyword: kw });
    await refresh();
  }

  async function resetDefaults() {
    if (!confirm("重置为默认高亮规则？现有规则将被清除。")) return;
    await invoke("reset_highlights");
    await refresh();
  }
</script>

<div class="page">
  <div class="add-card neu-raised">
    <input type="text" bind:value={newKw} placeholder="输入关键词..."
      onkeydown={(e) => { if (e.key === "Enter") add(); }} />
    <div class="color-picker">
      {#each COLORS as c}
        <button
          class="color-dot"
          class:selected={newColor === c.name}
          style="background: {c.css};"
          title={c.name}
          onclick={() => newColor = c.name}
        ></button>
      {/each}
    </div>
    <button class="btn btn-accent btn-sm" onclick={add} disabled={!newKw.trim()}>添加</button>
  </div>

  <div class="rules-list">
    {#each items as h (h.keyword)}
      <div class="rule-row">
        <span class="rule-dot" style="background: {colorCss(h.color)};"></span>
        <span class="rule-kw">{h.keyword}</span>
        <span class="rule-color">{h.color}</span>
        <button class="rule-del" onclick={() => remove(h.keyword)}>&times;</button>
      </div>
    {:else}
      <p class="empty">暂无高亮规则</p>
    {/each}
  </div>

  <button class="btn btn-sm reset-btn" onclick={resetDefaults}>重置为默认</button>
</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 16px; }

  .add-card { padding: 16px; display: flex; flex-direction: column; gap: 10px; }
  .color-picker { display: flex; flex-wrap: wrap; gap: 6px; }
  .color-dot {
    width: 24px; height: 24px; border-radius: 50%; border: 2px solid transparent;
    cursor: pointer; transition: border-color 0.15s, transform 0.1s;
  }
  .color-dot:hover { transform: scale(1.15); }
  .color-dot.selected { border-color: var(--text); transform: scale(1.2); }

  .rules-list { display: flex; flex-direction: column; gap: 4px; }
  .rule-row {
    display: flex; align-items: center; gap: 10px;
    padding: 10px 14px;
    background: var(--bg);
    box-shadow: var(--raised-sm);
    border-radius: var(--radius-sm);
  }
  .rule-dot { width: 12px; height: 12px; border-radius: 50%; flex-shrink: 0; }
  .rule-kw { font-weight: 600; font-family: monospace; font-size: 13px; flex: 1; }
  .rule-color { font-size: 11px; color: var(--text-dim); }
  .rule-del {
    background: none; border: none; font-size: 18px; color: var(--text-dim);
    cursor: pointer; padding: 0 4px; transition: color 0.1s;
  }
  .rule-del:hover { color: var(--error); }

  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
  .reset-btn { align-self: flex-start; }
</style>
