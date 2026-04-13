<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";

  let containerEl: HTMLDivElement;
  let terminal: Terminal;
  let fitAddon: FitAddon;

  let events = $state<[number, string, string][]>([]);
  let playing = $state(false);
  let currentIdx = $state(0);
  let speed = $state(1);
  let totalDuration = $state(0);
  let elapsed = $state(0);
  let timerId: ReturnType<typeof setTimeout> | null = null;

  const fileName = $derived(app.editingId() ?? "");
  let progress = $derived(totalDuration > 0 ? (elapsed / totalDuration) * 100 : 0);

  onMount(async () => {
    terminal = new Terminal({
      cursorBlink: false,
      fontSize: 13,
      fontFamily: "Menlo, Monaco, 'Courier New', monospace",
      theme: {
        background: "#2B2D3A", foreground: "#E0E5EC", cursor: "#4A6CF7",
        selectionBackground: "rgba(74,108,247,0.3)",
      },
    });
    fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(containerEl);
    fitAddon.fit();

    if (fileName) await loadCast(fileName);
    window.addEventListener("resize", handleResize);
  });

  function handleResize() { fitAddon?.fit(); }

  onDestroy(() => {
    stop();
    window.removeEventListener("resize", handleResize);
    terminal?.dispose();
  });

  async function loadCast(name: string) {
    try {
      const content = await invoke<string>("read_recording", { name });
      const lines = content.trim().split("\n");
      if (lines.length < 2) return;

      // Parse header (first line)
      const header = JSON.parse(lines[0]);
      if (header.width && header.height) {
        terminal.resize(header.width, header.height);
      }

      // Parse events
      events = [];
      for (let i = 1; i < lines.length; i++) {
        try {
          const ev = JSON.parse(lines[i]);
          if (Array.isArray(ev) && ev.length >= 3) {
            events.push(ev as [number, string, string]);
          }
        } catch { /* skip malformed lines */ }
      }
      totalDuration = events.length > 0 ? events[events.length - 1][0] : 0;
    } catch (e: any) {
      terminal.write(`\x1b[31mFailed to load recording: ${e}\x1b[0m\r\n`);
    }
  }

  function play() {
    if (events.length === 0) return;
    if (currentIdx >= events.length) {
      currentIdx = 0;
      terminal.reset();
    }
    playing = true;
    scheduleNext();
  }

  function scheduleNext() {
    if (!playing || currentIdx >= events.length) {
      playing = false;
      return;
    }
    const ev = events[currentIdx];
    const prevTime = currentIdx > 0 ? events[currentIdx - 1][0] : 0;
    const delay = ((ev[0] - prevTime) / speed) * 1000;

    timerId = setTimeout(() => {
      if (!playing) return;
      terminal.write(ev[2]);
      elapsed = ev[0];
      currentIdx++;
      scheduleNext();
    }, Math.max(delay, 1));
  }

  function pause() {
    playing = false;
    if (timerId) { clearTimeout(timerId); timerId = null; }
  }

  function stop() {
    pause();
    currentIdx = 0;
    elapsed = 0;
    terminal?.reset();
  }

  function setSpeed(s: number) {
    speed = s;
    if (playing) {
      pause();
      play();
    }
  }
</script>

<div class="playback">
  <div class="controls">
    <button class="btn btn-sm" onclick={() => app.settingsNavigate("recording-settings")}>← Recordings</button>
    <span class="file-name">{fileName}</span>
    <div class="spacer"></div>
    {#if playing}
      <button class="btn btn-sm" onclick={pause}>Pause</button>
    {:else}
      <button class="btn btn-accent btn-sm" onclick={play}>Play</button>
    {/if}
    <button class="btn btn-sm" onclick={stop}>Reset</button>
    <div class="speed-group">
      {#each [1, 2, 4, 8] as s}
        <button class="speed-btn" class:active={speed === s} onclick={() => setSpeed(s)}>{s}x</button>
      {/each}
    </div>
  </div>

  <div class="progress-bar">
    <div class="progress-fill" style="width: {progress}%;"></div>
  </div>

  <div class="term-container" bind:this={containerEl}></div>
</div>

<style>
  .playback { display: flex; flex-direction: column; height: 100%; }

  .controls {
    display: flex; align-items: center; gap: 8px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--divider);
    flex-shrink: 0;
  }
  .file-name { font-size: 12px; font-family: monospace; color: var(--text-sub); }
  .spacer { flex: 1; }
  .speed-group { display: flex; gap: 2px; }
  .speed-btn {
    padding: 4px 8px; border: none; border-radius: 6px;
    background: transparent; color: var(--text-dim);
    font-family: inherit; font-size: 11px; font-weight: 600;
    cursor: pointer; transition: all 0.1s;
  }
  .speed-btn:hover { background: var(--surface); }
  .speed-btn.active { background: var(--accent-soft); color: var(--accent); }

  .progress-bar {
    height: 3px; background: var(--surface); flex-shrink: 0;
  }
  .progress-fill {
    height: 100%; background: var(--accent);
    transition: width 0.1s linear;
  }

  .term-container { flex: 1; min-height: 0; }
  .term-container :global(.xterm) { height: 100%; padding: 4px; }
</style>
