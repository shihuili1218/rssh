<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import * as app from "../stores/app.svelte.ts";

  let { tabId, anchor }: { tabId: string; anchor: DOMRect } = $props();

  let lines = $state<string[]>([]);
  let timer: ReturnType<typeof setInterval> | undefined;

  const REFRESH_MS = 300;
  const GAP = 8;

  // Popover height is fixed by CSS (min/max-height). Measure the REAL height and
  // place it with that — not a guess. This is what keeps the bottom of the box
  // on-screen for thumbnails low in the list: top is clamped so the whole
  // popover fits, however tall the CSS makes it.
  let el: HTMLDivElement;
  let measuredH = $state(0);

  // The broadcast panel hugs the window's right edge, so the popover opens to
  // the LEFT of the hovered thumbnail (towards the editor). Read-only pull of
  // the tab's existing buffer — no new connection.
  let right = $derived(Math.max(GAP, window.innerWidth - anchor.left + GAP));
  let top = $derived(Math.max(GAP, Math.min(anchor.top, window.innerHeight - measuredH - GAP)));

  function pull() {
    lines = app.readTerminalViewportText(tabId) ?? [];
  }

  onMount(() => {
    pull();
    measuredH = el.offsetHeight;
    timer = setInterval(pull, REFRESH_MS);
  });
  onDestroy(() => clearInterval(timer));
</script>

<div bind:this={el} class="preview-popover" style="right: {right}px; top: {top}px;">
  <pre>{lines.join("\n")}</pre>
</div>

<style>
  .preview-popover {
    position: fixed;
    z-index: 1000;
    /* Preview is a glance, not a mirror: clamp width to a stable band and just
       clip anything wider (overflow: hidden). min() keeps it on-screen. */
    min-width: min(560px, calc(100vw - 220px));
    max-width: min(560px, calc(100vw - 220px));
    max-height: calc(100vh - 200px);
    min-height: calc(100vh - 200px);
    overflow: hidden;
    pointer-events: none;
    background: var(--term-bg);
    color: var(--term-fg);
    border: 1px solid var(--divider);
    border-radius: var(--radius-sm);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
    padding: 8px 10px;
  }

  .preview-popover pre {
    margin: 0;
    font-family: monospace;
    font-size: 11px;
    line-height: 1.25;
    white-space: pre;
    color: inherit;
  }
</style>
