<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import * as app from "../stores/app.svelte.ts";

  let { tabId }: { tabId: string } = $props();

  let canvasEl: HTMLCanvasElement;
  // width:height of the thumbnail. Real terminal cells are ~twice as tall as
  // wide, so the box keeps that shape (cols : rows*2) instead of looking square.
  let aspect = $state(80 / (24 * 2));
  let timer: ReturnType<typeof setInterval> | undefined;

  const REFRESH_MS = 500;

  // Repaint the minimap from the tab's live viewport snapshot. Read-only: it
  // pulls the existing terminal buffer, never a new connection. One canvas
  // pixel per cell; CSS stretches the tiny bitmap up to the thumbnail size.
  function paint() {
    const ctx = canvasEl?.getContext("2d");
    if (!ctx) return;

    const snap = app.readTerminalViewport(tabId);
    if (!snap || snap.cols === 0 || snap.rows === 0) {
      ctx.clearRect(0, 0, canvasEl.width, canvasEl.height);
      return;
    }
    if (canvasEl.width !== snap.cols || canvasEl.height !== snap.rows) {
      canvasEl.width = snap.cols;
      canvasEl.height = snap.rows;
    }
    aspect = snap.cols / (snap.rows * 2);

    const styles = getComputedStyle(canvasEl);
    const ink = styles.getPropertyValue("--term-fg").trim() || "#cccccc";
    const cursorColor = styles.getPropertyValue("--term-cursor").trim() || ink;

    ctx.clearRect(0, 0, snap.cols, snap.rows);
    ctx.fillStyle = ink;
    const { cols, rows, filled } = snap;
    for (let r = 0; r < rows; r++) {
      const rowBase = r * cols;
      for (let c = 0; c < cols; c++) {
        if (filled[rowBase + c]) ctx.fillRect(c, r, 1, 1);
      }
    }
    if (snap.cursor) {
      ctx.fillStyle = cursorColor;
      ctx.fillRect(snap.cursor.x, snap.cursor.y, 1, 1);
    }
  }

  onMount(() => {
    paint();
    timer = setInterval(paint, REFRESH_MS);
  });
  onDestroy(() => clearInterval(timer));
</script>

<canvas bind:this={canvasEl} class="minimap" style="aspect-ratio: {aspect};" aria-hidden="true"></canvas>

<style>
  .minimap {
    display: block;
    width: 100%;
    height: auto;
    image-rendering: pixelated;
    background: var(--term-bg);
    border-radius: var(--radius-sm);
  }
</style>
