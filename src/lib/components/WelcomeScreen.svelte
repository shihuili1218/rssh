<!--
  WelcomeScreen — cinematic intro overlay.
  Owns the scene state machine, the global keyboard shortcuts, the
  scene indicator, and the temporary window-decorations toggle. Each
  scene is a self-contained component that calls onNext when its
  Next → button is clicked.
-->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { t } from "../i18n/index.svelte.ts";
  import { isMobile } from "../stores/app.svelte.ts";

  import SceneIntro from "./welcome/SceneIntro.svelte";
  import SceneAi from "./welcome/SceneAi.svelte";
  import SceneBlocks from "./welcome/SceneBlocks.svelte";
  import SceneSync from "./welcome/SceneSync.svelte";
  import SceneCli from "./welcome/SceneCli.svelte";
  import SceneCta from "./welcome/SceneCta.svelte";

  let { onDismiss }: { onDismiss: () => void } = $props();

  // Linear scene flow. CTA is terminal — its "Next" is dismiss.
  const FLOW = ["intro", "ai", "blocks", "sync", "cli", "cta"] as const;
  type Scene = (typeof FLOW)[number];

  let scene = $state<Scene>("intro");
  // Bumping this remounts the active scene to replay it from the start.
  let replayKey = $state(0);

  // Demo-scene indicator only shows for the 4 "feature" scenes.
  const FEATURE_SCENES: Scene[] = ["ai", "blocks", "sync", "cli"];
  let featureIdx = $derived(FEATURE_SCENES.indexOf(scene));
  let showIndicator = $derived(featureIdx >= 0);

  function advance() {
    const idx = FLOW.indexOf(scene);
    if (idx < 0 || idx >= FLOW.length - 1) {
      onDismiss();
      return;
    }
    scene = FLOW[idx + 1];
  }

  function jumpTo(target: Scene) {
    if (scene !== target) { scene = target; }
    else { replayKey++; }
  }

  function replayCurrent() {
    replayKey++;
  }

  function handleKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      // CTA's Enter dismisses; intro's Enter advances; feature scenes'
      // Enter advances to next scene (mirrors clicking Next).
      if (scene === "cta") onDismiss();
      else advance();
    } else if (e.key === "Escape") {
      e.preventDefault();
      if (scene === "cta") onDismiss();
      else scene = "cta";
    } else if (e.key === " " || e.code === "Space") {
      e.preventDefault();
      replayCurrent();
    }
  }

  // ── Window decorations: temporarily strip the title bar so the
  //    welcome occupies the whole window for maximum immersion. We
  //    restore the original state on unmount, even if the user closes
  //    via Esc or the parent unmount (e.g. preview button toggled off
  //    from About). Mobile platforms don't have window chrome to toggle.
  let originalDecorated = true;
  let decorChanged = false;

  onMount(async () => {
    if (isMobile) return;
    try {
      const win = getCurrentWindow();
      originalDecorated = await win.isDecorated();
      if (originalDecorated) {
        await win.setDecorations(false);
        decorChanged = true;
      }
    } catch (e) {
      // Tauri API not available (e.g. running purely in a browser tab
      // during a vite dev preview). Fall back gracefully — the CSS
      // overlay alone still works.
      console.debug("welcome: setDecorations skipped:", e);
    }
  });

  onDestroy(async () => {
    if (!decorChanged) return;
    try {
      await getCurrentWindow().setDecorations(originalDecorated);
    } catch (e) {
      console.debug("welcome: restore decorations failed:", e);
    }
  });
</script>

<svelte:window onkeydown={handleKey} />

<div class="welcome-root" role="dialog" aria-modal="true" aria-label={t("welcome.aria")}>
  <div class="bg-layer" aria-hidden="true"></div>
  <div class="scanlines" aria-hidden="true"></div>
  <div class="vignette" aria-hidden="true"></div>

  <!-- Skip / Close in top-right. In CTA scene it becomes Close. -->
  <button
    class="skip-btn"
    onclick={() => (scene === "cta" ? onDismiss() : (scene = "cta"))}
  >
    {scene === "cta" ? t("welcome.close") : t("welcome.skip")}
    <span class="arrow">→</span>
  </button>

  <!-- Per-scene mount. {#key} ensures Space (replay) actually
       remounts the active scene's internal timers from zero. -->
  <div class="scene-host">
    {#key `${scene}:${replayKey}`}
      {#if scene === "intro"}
        <SceneIntro onNext={advance} />
      {:else if scene === "ai"}
        <SceneAi onNext={advance} />
      {:else if scene === "blocks"}
        <SceneBlocks onNext={advance} />
      {:else if scene === "sync"}
        <SceneSync onNext={advance} />
      {:else if scene === "cli"}
        <SceneCli onNext={advance} />
      {:else if scene === "cta"}
        <SceneCta onNext={onDismiss} onReplay={() => jumpTo("intro")} />
      {/if}
    {/key}
  </div>

  {#if showIndicator}
    <div class="indicator" aria-label={t("welcome.controls.indicator_aria")}>
      {#each FEATURE_SCENES as s, i}
        <button
          class="dot"
          class:active={i === featureIdx}
          class:done={i < featureIdx}
          onclick={() => jumpTo(s)}
          onkeydown={(e) => {
            // svelte:window 全局 handler 把 Enter/Space 解释成 advance/replay。
            // 焦点在 dot 上时按 Enter/Space 应该触发本按钮的 click → jumpTo，
            // 而不是上层的 advance。让 button 默认行为（Enter→click）继续，
            // 仅阻止冒泡到 window handler。
            if (e.key === "Enter" || e.key === " " || e.code === "Space") {
              e.stopPropagation();
            }
          }}
          aria-label={t(`welcome.scene.${s}.chip` as `welcome.scene.${"ai"|"blocks"|"sync"|"cli"}.chip`)}
        ></button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .welcome-root {
    position: fixed;
    inset: 0;
    z-index: 9999;
    overflow: hidden;
    isolation: isolate;
    user-select: text;
    -webkit-user-select: text;
    color: var(--text);
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
    opacity: 0;
    animation: root-in 360ms ease-out forwards;
  }
  @keyframes root-in {
    to { opacity: 1; }
  }

  /* Dark theatrical backdrop — radial gradient anchored on accent,
     deep enough to make scene content pop. */
  .bg-layer {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(ellipse at 50% 30%, color-mix(in srgb, var(--accent) 18%, transparent) 0%, transparent 65%),
      radial-gradient(ellipse at center, var(--surface) 0%, var(--bg) 50%, var(--shadow-dark) 100%);
    z-index: 0;
  }

  /* CRT-style scanlines. Multiply blend keeps highlight colors saturated. */
  .scanlines {
    position: absolute;
    inset: 0;
    background: repeating-linear-gradient(
      0deg,
      rgba(0, 0, 0, 0.10) 0px,
      rgba(0, 0, 0, 0.10) 1px,
      transparent 1px,
      transparent 3px
    );
    pointer-events: none;
    mix-blend-mode: multiply;
    z-index: 1;
  }

  .vignette {
    position: absolute;
    inset: 0;
    background: radial-gradient(ellipse at center, transparent 50%, rgba(0,0,0,0.55) 100%);
    pointer-events: none;
    z-index: 1;
  }

  .skip-btn {
    position: absolute;
    top: calc(20px + env(safe-area-inset-top, 0px));
    right: 24px;
    z-index: 10;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.12);
    color: var(--text-sub);
    padding: 6px 14px;
    border-radius: var(--radius-sm);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 11px;
    cursor: pointer;
    letter-spacing: 0.8px;
    text-transform: uppercase;
    font-weight: 700;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    transition: border-color 0.18s, color 0.18s, background 0.18s, transform 0.1s;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
  }
  .skip-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .skip-btn:active { transform: scale(0.97); }
  .skip-btn .arrow { transition: transform 0.18s ease; }
  .skip-btn:hover .arrow { transform: translateX(2px); }

  .scene-host {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: flex;
  }
  .scene-host > :global(*) {
    flex: 1;
    min-height: 0;
  }

  .indicator {
    position: absolute;
    left: 50%;
    bottom: calc(18px + env(safe-area-inset-bottom, 0px));
    transform: translateX(-50%);
    display: inline-flex;
    gap: 10px;
    padding: 8px 14px;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.08);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    z-index: 10;
    opacity: 0;
    animation: ind-in 500ms 200ms forwards;
  }
  @keyframes ind-in { to { opacity: 1; } }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.15);
    border: none;
    padding: 0;
    cursor: pointer;
    transition: background 0.2s ease, transform 0.2s ease, box-shadow 0.2s ease;
  }
  .dot:hover { background: rgba(255, 255, 255, 0.4); }
  .dot.done {
    background: color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .dot.active {
    background: var(--accent);
    width: 22px;
    border-radius: 999px;
    box-shadow: 0 0 12px var(--accent);
  }

  @media (prefers-reduced-motion: reduce) {
    .welcome-root, .indicator { animation: none !important; opacity: 1 !important; }
  }
</style>
