<!--
  Scene 4 — CLI-First.
  Split stage: left is a mock OS terminal where someone types
  `rssh open prod`; right is the rssh GUI. After Enter, a beam shoots
  from the terminal into the GUI's tab strip, and a new "prod" tab
  slides in and becomes active. Then the terminal prints the
  confirmation line.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../i18n/index.svelte.ts";
  import NextButton from "./NextButton.svelte";

  let { onNext }: { onNext: () => void } = $props();

  let typed = $state("");
  let sent = $state(false);
  let beamOn = $state(false);
  let newTabOpen = $state(false);
  let confirmed = $state(false);
  let captionShown = $state(false);
  let ready = $state(false);

  const CMD = "rssh open prod";

  let timers: number[] = [];
  function at(ms: number, fn: () => void) { timers.push(window.setTimeout(fn, ms)); }

  onMount(() => {
    const reduced =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

    if (reduced) {
      typed = CMD; sent = true; beamOn = false; newTabOpen = true;
      confirmed = true; captionShown = true; ready = true;
      return;
    }

    for (let i = 1; i <= CMD.length; i++) {
      at(600 + i * 70, () => { typed = CMD.slice(0, i); });
    }
    const typeEnd = 600 + CMD.length * 70;
    at(typeEnd + 240, () => { sent = true; beamOn = true; });
    at(typeEnd + 700, () => { newTabOpen = true; });
    at(typeEnd + 1200, () => { beamOn = false; });
    at(typeEnd + 1300, () => { confirmed = true; });
    at(typeEnd + 1800, () => { captionShown = true; });
    at(typeEnd + 2600, () => { ready = true; });

    return () => { timers.forEach(window.clearTimeout); };
  });
</script>

<section class="scene">
  <div class="chip">
    <span class="chip-dot"></span>
    {t("welcome.scene.cli.chip")}
  </div>

  <div class="stage">
    <!-- LEFT: OS terminal where the user runs the CLI -->
    <div class="mock-app term-side">
      <div class="app-header">
        <div class="dots"><span class="dot r"></span><span class="dot y"></span><span class="dot g"></span></div>
        <div class="app-title">~ — zsh</div>
        <span class="header-spacer"></span>
      </div>
      <div class="term-body">
        <div class="ln dim">Last login on ttys004</div>
        <div class="ln"><span class="ps1">~ ❯</span> <span class="typed">{typed}</span>{#if !sent}<span class="caret">_</span>{/if}{#if sent}<span class="enter-key lit">⏎</span>{/if}</div>
        {#if confirmed}
          <div class="ln out">
            → {t("welcome.scene.cli.confirm")} <span class="hl">ssh:prod</span>
          </div>
          <div class="ln"><span class="ps1">~ ❯</span> <span class="caret">_</span></div>
        {/if}
      </div>
    </div>

    <!-- Beam: a glowing line from terminal → GUI sidebar. Drawn as a
         single div positioned across the gutter, opacity-pulsed. -->
    <div class="beam" class:on={beamOn} aria-hidden="true"></div>

    <!-- RIGHT: the rssh GUI receiving the new tab -->
    <div class="mock-app gui-side">
      <div class="app-header">
        <div class="dots"><span class="dot r"></span><span class="dot y"></span><span class="dot g"></span></div>
        <div class="app-title">RSSH</div>
        <span class="header-spacer"></span>
      </div>
      <div class="gui-body">
        <nav class="gui-sidebar">
          <div class="gui-tab tab-home" title="Home">⌂</div>
          <div class="gui-tab tab-add" title="New">+</div>
          <div class="gui-sep"></div>
          <div class="gui-tab tab-local">L</div>
          <div class="gui-tab tab-prod" class:appear={newTabOpen} class:active={newTabOpen} title="prod">
            <span class="tab-letter">P</span>
            <span class="tab-label-ext">prod</span>
          </div>
          <div class="gui-spacer"></div>
          <div class="gui-tab tab-settings" title="Settings">⚙</div>
        </nav>
        <div class="gui-content">
          <div class="gui-content-inner" class:active={newTabOpen}>
            {#if !newTabOpen}
              <div class="gui-empty">
                <div class="gui-empty-title">{t("welcome.scene.cli.gui_waiting")}</div>
                <div class="gui-empty-hint">{t("welcome.scene.cli.gui_hint")}</div>
              </div>
            {:else}
              <div class="gui-term-mock">
                <div class="gln"><span class="ps1g">prod ❯</span> uptime</div>
                <div class="gln out">up 14 days · load 1.42</div>
                <div class="gln"><span class="ps1g">prod ❯</span> <span class="caret">_</span></div>
              </div>
            {/if}
          </div>
        </div>
      </div>
    </div>
  </div>

  <div class="caption" class:show={captionShown}>
    <span class="kw">{t("welcome.scene.cli.caption_kw1")}</span>
    {t("welcome.scene.cli.caption_join")}
    <span class="kw">{t("welcome.scene.cli.caption_kw2")}</span>
  </div>

  <NextButton {ready} onClick={onNext} />
</section>

<style>
  .scene {
    position: relative;
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: clamp(18px, 3vh, 32px);
    padding: clamp(20px, 4vh, 48px) clamp(16px, 3vw, 32px);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border: 1px solid color-mix(in srgb, var(--success) 55%, transparent);
    border-radius: 999px;
    background: color-mix(in srgb, var(--success) 10%, transparent);
    color: var(--success);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 1.4px;
    text-transform: uppercase;
    opacity: 0;
    animation: chip-in 500ms 200ms forwards;
  }
  @keyframes chip-in { to { opacity: 1; } }
  .chip-dot {
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--success);
    box-shadow: 0 0 8px var(--success);
    animation: chip-pulse 1.6s ease-in-out infinite;
  }
  @keyframes chip-pulse { 50% { opacity: 0.45; } }

  .stage {
    position: relative;
    width: min(86vw, 1100px);
    aspect-ratio: 16 / 10;
    max-height: 64vh;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 24px;
    opacity: 0;
    transform: translateY(14px);
    animation: stage-in 600ms 100ms cubic-bezier(0.22, 1, 0.36, 1) forwards;
  }
  @keyframes stage-in { to { opacity: 1; transform: translateY(0); } }

  .mock-app {
    background: #1c1d24;
    border-radius: 14px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    box-shadow:
      0 30px 80px rgba(0, 0, 0, 0.6),
      0 0 0 1px rgba(255, 255, 255, 0.05);
  }
  .app-header {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 10px 14px;
    background: linear-gradient(180deg, #2a2c36 0%, #232530 100%);
    border-bottom: 1px solid rgba(0, 0, 0, 0.4);
  }
  .dots { display: flex; gap: 7px; justify-self: start; }
  .dot { width: 12px; height: 12px; border-radius: 50%; }
  .dot.r { background: #ff5f57; } .dot.y { background: #febc2e; } .dot.g { background: #28c840; }
  .app-title { font-family: "SF Mono", Menlo, Consolas, monospace; font-size: 11px; color: rgba(255,255,255,0.55); letter-spacing: 0.6px; }
  .header-spacer { justify-self: end; }

  .term-body {
    flex: 1;
    padding: 16px 18px;
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 13px;
    line-height: 1.7;
    color: #d4d8e2;
  }
  .ln { white-space: pre-wrap; }
  .ln.dim { color: var(--text-dim); }
  .ln.out { color: var(--success); }
  .ln.out .hl {
    background: color-mix(in srgb, var(--success) 18%, transparent);
    color: var(--success);
    padding: 1px 6px;
    border-radius: 4px;
    font-weight: 700;
  }
  .ps1 { color: var(--success); margin-right: 6px; font-weight: 700; }
  .ps1g { color: var(--accent); margin-right: 6px; font-weight: 700; }
  .typed { color: var(--text); }
  .caret { color: var(--accent); animation: blink 1s steps(1, start) infinite; font-weight: 700; }
  @keyframes blink { 50% { opacity: 0; } }
  .enter-key {
    margin-left: 8px;
    width: 22px;
    height: 18px;
    border-radius: 4px;
    background: var(--accent);
    color: var(--white);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 10px;
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent) 20%, transparent);
    animation: enter-flash 360ms ease-out;
    vertical-align: middle;
  }
  @keyframes enter-flash {
    0%   { transform: scale(1.4); box-shadow: 0 0 0 10px color-mix(in srgb, var(--accent) 30%, transparent); }
    100% { transform: scale(1); box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent) 20%, transparent); }
  }

  /* Beam — sits in the column gutter, full height. */
  .beam {
    position: absolute;
    top: 12%;
    bottom: 12%;
    /* Center of the 24px gap between the two grid columns. The grid
       has `grid-template-columns: 1fr 1fr` with 24px gap; the gutter
       starts at 50% - 12px and ends at 50% + 12px. Center is 50%. */
    left: 50%;
    width: 4px;
    transform: translateX(-50%);
    background: linear-gradient(180deg, transparent 0%, var(--accent) 30%, var(--accent) 70%, transparent 100%);
    opacity: 0;
    z-index: 5;
    border-radius: 2px;
    transition: opacity 200ms ease;
    box-shadow: 0 0 18px var(--accent), 0 0 40px color-mix(in srgb, var(--accent) 50%, transparent);
  }
  .beam.on {
    opacity: 0.9;
    animation: beam-pulse 1200ms ease-in-out;
  }
  @keyframes beam-pulse {
    0%, 100% { opacity: 0.2; }
    50%      { opacity: 1; }
  }

  /* GUI side */
  .gui-body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .gui-sidebar {
    width: 44px;
    padding: 6px;
    background: #181920;
    border-right: 1px solid rgba(255, 255, 255, 0.04);
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: center;
  }
  .gui-tab {
    width: 32px;
    height: 32px;
    border-radius: 7px;
    background: rgba(255, 255, 255, 0.04);
    color: var(--text-sub);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 13px;
    font-weight: 700;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    position: relative;
    transition: background 0.18s ease, color 0.18s ease, box-shadow 0.2s ease, width 0.3s ease;
  }
  .gui-sep {
    width: 22px;
    height: 1px;
    background: rgba(255, 255, 255, 0.06);
    margin: 4px 0;
  }
  .gui-spacer { flex: 1; }

  .tab-home { background: var(--bg); color: var(--text); }
  .tab-add { color: var(--text-dim); }
  .tab-local { color: var(--text); }

  /* The new prod tab — initially scaled to 0, animates in at 'appear'. */
  .tab-prod {
    transform: scale(0.4);
    opacity: 0;
    width: 0;
    overflow: hidden;
    border: 1px solid transparent;
  }
  .tab-prod.appear {
    transform: scale(1);
    opacity: 1;
    width: 32px;
    animation: tab-in 520ms cubic-bezier(0.22, 1, 0.36, 1) forwards;
  }
  @keyframes tab-in {
    0%   { transform: scale(0.4) rotate(-8deg); opacity: 0; }
    60%  { transform: scale(1.08) rotate(2deg); opacity: 1; }
    100% { transform: scale(1) rotate(0deg); opacity: 1; }
  }
  .tab-prod.active {
    background: color-mix(in srgb, var(--accent) 28%, transparent);
    color: var(--accent);
    box-shadow:
      inset 0 0 0 1px color-mix(in srgb, var(--accent) 50%, transparent),
      0 0 14px color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .tab-letter { z-index: 1; }
  .tab-label-ext {
    display: none;
  }

  .tab-settings {
    margin-top: auto;
    color: var(--text-dim);
  }

  .gui-content {
    flex: 1;
    padding: 14px;
    background: var(--bg, #2B2D3A);
    position: relative;
    overflow: hidden;
  }
  .gui-content-inner {
    height: 100%;
    display: flex;
    flex-direction: column;
    /* Mirror the left zsh pane — content starts at the top. The
       empty-state "waiting…" copy is small enough that top-alignment
       reads as "ready to receive output" rather than "lost in space". */
    justify-content: flex-start;
  }
  .gui-empty {
    color: var(--text-dim);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 12px;
    text-align: center;
  }
  .gui-empty-title { color: var(--text-sub); margin-bottom: 4px; }
  .gui-empty-hint { font-size: 10.5px; letter-spacing: 0.4px; }

  .gui-term-mock {
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 12px;
    line-height: 1.7;
    color: #d4d8e2;
    opacity: 0;
    animation: gui-fade 480ms ease forwards;
  }
  @keyframes gui-fade { to { opacity: 1; } }
  .gln.out { color: var(--text-sub); }

  .caption {
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: clamp(12px, 1.3vw, 14px);
    color: var(--text-sub);
    letter-spacing: 0.6px;
    text-align: center;
    opacity: 0;
    transform: translateY(6px);
    transition: opacity 400ms ease, transform 400ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .caption.show { opacity: 1; transform: translateY(0); }
  .caption .kw { color: var(--success); font-weight: 700; }

  @media (max-width: 640px) {
    .stage { grid-template-columns: 1fr; grid-template-rows: 1fr 1fr; }
    .beam { display: none; }
  }

  @media (prefers-reduced-motion: reduce) {
    .chip, .stage, .mock-app, .beam, .tab-prod, .gui-term-mock,
    .caption, .caret, .chip-dot {
      animation: none !important;
      transition: none !important;
      opacity: 1 !important;
      transform: none !important;
    }
    .beam { opacity: 0 !important; }
  }
</style>
