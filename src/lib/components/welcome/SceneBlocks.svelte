<!--
  Scene 2 — Command Block colors.
  Bars light up in sequence → mouse clicks middle bar → menu opens →
  mouse clicks "Copy as image" → thumbnails burst out from cursor →
  mouse clicks top bar → menu opens → mouse clicks "Fold" → block
  collapses to a single "N lines hidden" line. Caption → Next →.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../i18n/index.svelte.ts";
  import MockCursor from "./MockCursor.svelte";
  import NextButton from "./NextButton.svelte";

  let { onNext }: { onNext: () => void } = $props();

  // Cursor coordinates carry "px" so they slot directly into the
  // MockCursor `left:` / `top:` props. Hand-tuned percentages drift
  // off-target whenever the stage resizes; we instead measure target
  // elements via getBoundingClientRect and emit pixel offsets.
  let cursorX = $state("0px");
  let cursorY = $state("0px");
  let cursorVisible = $state(false);
  let cursorClicking = $state(false);

  // Stage ref for relative geometry; bound via bind:this below.
  let stageEl: HTMLElement | undefined = $state();

  let barStage = $state(0);
  let selectedIdx = $state<number | null>(null);
  let menuOpenIdx = $state<number | null>(null);
  // Which menu item the cursor is hovering — drives the highlight bg
  // so the demo "previews" the click target before the click ripple.
  let hoverItem = $state<"text" | "image" | "fold" | null>(null);
  let foldedIdx = $state<number | null>(null);
  // Toggling false→true remounts the burst thumbnails so their CSS
  // entrance keyframes fire from zero.
  let imageBurst = $state(false);
  // Origin is captured at trigger time so later cursor moves don't drag
  // the burst across the screen.
  let burstX = $state("0px");
  let burstY = $state("0px");
  let focusOn = $state(false);
  let captionShown = $state(false);
  let ready = $state(false);

  let timers: number[] = [];
  function at(ms: number, fn: () => void) { timers.push(window.setTimeout(fn, ms)); }

  /** Move cursor to the centre of the element matched by `selector`,
   *  measured at runtime. No-op if the element isn't in the DOM yet
   *  (e.g. menu hasn't opened); callers space their timing so this
   *  doesn't happen in practice. */
  function moveCursorTo(selector: string) {
    if (!stageEl) return;
    const target = stageEl.querySelector<HTMLElement>(selector);
    if (!target) return;
    const s = stageEl.getBoundingClientRect();
    const t = target.getBoundingClientRect();
    cursorX = `${(t.left - s.left + t.width / 2).toFixed(1)}px`;
    cursorY = `${(t.top - s.top + t.height / 2).toFixed(1)}px`;
  }

  /** Park the cursor just outside the bottom-right of the stage so the
   *  first reveal glides in from off-screen. */
  function moveCursorOffStage() {
    if (!stageEl) return;
    const r = stageEl.getBoundingClientRect();
    cursorX = `${r.width + 24}px`;
    cursorY = `${r.height + 24}px`;
  }

  onMount(() => {
    const reduced =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

    if (reduced) {
      barStage = 3;
      selectedIdx = null;
      menuOpenIdx = null;
      foldedIdx = 0;
      focusOn = false;
      captionShown = true;
      ready = true;
      // park cursor somewhere reasonable (we don't animate, just static).
      moveCursorOffStage();
      return;
    }

    // Cursor starts just off the bottom-right of the stage so its
    // first move feels like an entrance.
    moveCursorOffStage();

    // Bars light up
    at(400,  () => { barStage = 1; });
    at(700,  () => { barStage = 2; });
    at(1000, () => { barStage = 3; });

    // ── Beat 1: click middle bar → copy as image → burst ──
    at(1300, () => { cursorVisible = true; });
    at(1500, () => moveCursorTo('[data-target="bar-1"]'));
    at(2500, () => { cursorClicking = true; });
    at(2620, () => { cursorClicking = false; selectedIdx = 1; });
    at(2900, () => { menuOpenIdx = 1; focusOn = true; });
    // Menu mounted; wait long enough for layout to settle before aiming.
    at(3600, () => moveCursorTo('[data-menu-item="image"]'));
    at(4100, () => { hoverItem = "image"; });
    at(4400, () => { cursorClicking = true; });
    at(4520, () => {
      cursorClicking = false;
      menuOpenIdx = null;
      selectedIdx = null;
      hoverItem = null;
      focusOn = false;
      burstX = cursorX;
      burstY = cursorY;
      imageBurst = true;
    });
    at(5400, () => { imageBurst = false; });

    // ── Beat 2: click top bar → fold → block collapses ──
    at(5500, () => moveCursorTo('[data-target="bar-0"]'));
    at(6400, () => { cursorClicking = true; });
    at(6520, () => {
      cursorClicking = false;
      selectedIdx = 0;
      menuOpenIdx = 0;
      focusOn = true;
    });
    at(7200, () => moveCursorTo('[data-menu-item="fold"]'));
    at(7700, () => { hoverItem = "fold"; });
    at(8000, () => { cursorClicking = true; });
    at(8120, () => {
      cursorClicking = false;
      menuOpenIdx = null;
      selectedIdx = null;
      hoverItem = null;
      focusOn = false;
      foldedIdx = 0;
    });

    at(8900, () => { captionShown = true; });
    at(9700, () => { ready = true; });

    return () => { timers.forEach(window.clearTimeout); };
  });

  // Three mock command blocks. Each defines its bar color + lines.
  const blocks = [
    {
      color: "var(--success)",
      cmd: "ls /var/log",
      out: ["auth.log    52K", "nginx/      4.0K", "syslog      18M", "kern.log    9.2M"],
    },
    {
      color: "var(--accent)",
      cmd: "df -h /",
      out: [
        "Filesystem  Size  Used  Avail  Use%",
        "/dev/sda1   480G  478G    2G   100%   /",
      ],
    },
    {
      color: "var(--warning)",
      cmd: "ps aux | head -3",
      out: [
        "USER  PID  %CPU  COMMAND",
        "root    1   0.0  /sbin/init",
        "rssh   42   1.8  node server.js",
      ],
    },
  ];

  // Menu sits at fixed left, top depends on which block is selected
  // (top block lower offset, bottom block higher offset).
  let menuTop = $derived(
    menuOpenIdx === 0 ? "22%" : menuOpenIdx === 1 ? "38%" : "55%"
  );
</script>

<section class="scene">
  <div class="chip">
    <span class="chip-dot"></span>
    {t("welcome.scene.blocks.chip")}
  </div>

  <div class="stage" class:focus-on={focusOn} bind:this={stageEl}>
    <div class="mock-app">
      <div class="app-header">
        <div class="dots"><span class="dot r"></span><span class="dot y"></span><span class="dot g"></span></div>
        <div class="app-title">rssh — staging</div>
        <span class="header-spacer"></span>
      </div>

      <div class="term-pane">
        {#each blocks as b, i}
          <div
            class="block"
            class:lit={barStage > i}
            class:selected={selectedIdx === i}
            class:dim={focusOn && selectedIdx !== i}
            class:folded={foldedIdx === i}
            style="--bar: {b.color};"
          >
            <span class="bar" data-target="bar-{i}" aria-hidden="true"></span>
            <div class="block-body">
              <div class="ln cmd-ln"><span class="prompt">$</span> {b.cmd}</div>
              {#each b.out as o}
                <div class="ln out">{o}</div>
              {/each}
              {#if foldedIdx === i}
                <div class="ln folded-hint">
                  ▶ {t("welcome.scene.blocks.folded_hint", { n: b.out.length })}
                </div>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    </div>

    {#if menuOpenIdx !== null}
      <div
        class="ctx-menu"
        role="menu"
        aria-hidden="true"
        style="top: {menuTop};"
      >
        <div class="ctx-item" data-menu-item="text" class:hovered={hoverItem === "text"}>
          <span class="ctx-label">{t("welcome.scene.blocks.menu_copy_text")}</span>
          <span class="ctx-key">⌘C</span>
        </div>
        <div class="ctx-item" data-menu-item="image" class:hovered={hoverItem === "image"}>
          <span class="ctx-label">{t("welcome.scene.blocks.menu_copy_image")}</span>
          <span class="ctx-key">⇧⌘C</span>
        </div>
        <div class="ctx-sep"></div>
        <div class="ctx-item" data-menu-item="fold" class:hovered={hoverItem === "fold"}>
          <span class="ctx-label">{t("welcome.scene.blocks.menu_fold")}</span>
        </div>
      </div>
    {/if}

    <!-- Thumbnails burst out from the cursor when "Copy as image" is
         clicked. Four cards fly off in different directions with rotation
         and fade — the "messy clipboard explosion" beat. -->
    {#if imageBurst}
      <div class="burst-host" style="left: {burstX}; top: {burstY};" aria-hidden="true">
        <div class="thumb t1">$ ls /var/log</div>
        <div class="thumb t2">$ df -h /</div>
        <div class="thumb t3">$ ps aux | head</div>
        <div class="thumb t4">$ uptime</div>
      </div>
    {/if}

    <MockCursor
      x={cursorX}
      y={cursorY}
      visible={cursorVisible}
      clicking={cursorClicking}
      duration={1000}
    />
  </div>

  <div class="caption" class:show={captionShown}>
    <span class="kw">{t("welcome.scene.blocks.caption_kw1")}</span>
    {t("welcome.scene.blocks.caption_join")}
    <span class="kw">{t("welcome.scene.blocks.caption_kw2")}</span>
    {t("welcome.scene.blocks.caption_join")}
    <span class="kw">{t("welcome.scene.blocks.caption_kw3")}</span>
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
    border: 1px solid color-mix(in srgb, var(--accent) 55%, transparent);
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    color: var(--accent);
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
    background: var(--accent);
    box-shadow: 0 0 8px var(--accent);
    animation: chip-pulse 1.6s ease-in-out infinite;
  }
  @keyframes chip-pulse { 50% { opacity: 0.45; } }

  .stage {
    position: relative;
    width: min(86vw, 1100px);
    aspect-ratio: 16 / 10;
    max-height: 64vh;
    opacity: 0;
    transform: translateY(14px);
    animation: stage-in 600ms 100ms cubic-bezier(0.22, 1, 0.36, 1) forwards;
  }
  @keyframes stage-in { to { opacity: 1; transform: translateY(0); } }

  .mock-app {
    position: absolute;
    inset: 0;
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

  .term-pane {
    flex: 1;
    padding: 18px 22px;
    font-family: "SF Mono", Menlo, Consolas, "Courier New", monospace;
    font-size: 13px;
    line-height: 1.6;
    color: #d4d8e2;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .block {
    position: relative;
    display: flex;
    gap: 14px;
    padding: 6px 4px 6px 0;
    border-radius: 6px;
    transition:
      background 300ms ease,
      transform 360ms cubic-bezier(0.22, 1, 0.36, 1),
      filter 360ms ease,
      opacity 360ms ease;
  }
  .bar {
    width: 3px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 0 2px 2px 0;
    flex-shrink: 0;
    transition: background 400ms ease, box-shadow 400ms ease;
  }
  .block.lit .bar {
    background: var(--bar);
    box-shadow: 0 0 10px color-mix(in srgb, var(--bar) 50%, transparent);
  }
  .block-body { flex: 1; min-width: 0; }
  .ln {
    white-space: pre-wrap;
    /* max-height enables a smooth collapse when .folded; the value is
       comfortably above the actual line height so it doesn't clip
       normal content. */
    max-height: 36px;
    overflow: hidden;
    transition: max-height 280ms ease, opacity 220ms ease, margin 220ms ease;
  }
  .ln.out { color: #b8bcc8; }
  .ln.cmd-ln { color: var(--text); }
  .ln.folded-hint {
    color: var(--text-dim);
    font-style: italic;
    font-size: 12px;
    opacity: 0;
    animation: fold-hint-in 320ms 240ms forwards;
  }
  @keyframes fold-hint-in { to { opacity: 1; } }
  .prompt { color: var(--success); margin-right: 6px; }

  /* Collapse output lines when block is folded. cmd-ln + folded-hint
     stay visible so the user sees "▶ N lines hidden" in place. */
  .block.folded .ln.out {
    max-height: 0;
    opacity: 0;
    margin: 0;
  }

  /* Selection state: terminal-style highlight on bar + bg tint. */
  .block.selected {
    background: color-mix(in srgb, var(--bar) 14%, transparent);
  }
  .block.selected .bar {
    width: 4px;
    box-shadow: 0 0 14px color-mix(in srgb, var(--bar) 80%, transparent);
  }
  .block.selected .ln {
    background: color-mix(in srgb, var(--bar) 22%, transparent);
    margin: 0 -4px;
    padding: 0 4px;
  }

  /* Focus zoom: dim the un-selected blocks. */
  .block.dim {
    filter: brightness(0.4) saturate(0.6);
    opacity: 0.6;
  }
  .stage.focus-on .block.selected {
    transform: scale(1.04);
    transform-origin: left center;
    z-index: 2;
  }

  /* Menu — left-aligned items (label + shortcut sit together, no
     space-between). top is set inline based on which block hosts it. */
  .ctx-menu {
    position: absolute;
    left: 6%;
    background: #232530;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    padding: 4px;
    min-width: 210px;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.55);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 12px;
    color: var(--text);
    z-index: 40;
    opacity: 0;
    transform: translateY(0);
    animation: menu-in 200ms cubic-bezier(0.22, 1, 0.36, 1) forwards;
  }
  @keyframes menu-in {
    from { opacity: 0; transform: translateY(4px); }
    to   { opacity: 1; transform: translateY(0); }
  }
  .ctx-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-radius: 5px;
    transition: background 0.15s ease;
  }
  .ctx-item.hovered {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .ctx-label { color: var(--text); }
  .ctx-key {
    font-size: 10px;
    color: var(--text-dim);
    letter-spacing: 0.4px;
  }
  .ctx-sep { height: 1px; background: rgba(255,255,255,0.06); margin: 3px 4px; }

  /* ─── Image burst — thumbnails fly out of the cursor in different
     directions, rotated and fading. Mimics "I just copied N images,
     they're scattering". */
  .burst-host {
    position: absolute;
    pointer-events: none;
    z-index: 50;
  }
  .thumb {
    position: absolute;
    top: 0;
    left: 0;
    width: clamp(80px, 8vw, 100px);
    height: clamp(52px, 6vw, 64px);
    background: linear-gradient(180deg, #2a2c36 0%, #1c1d24 100%);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-radius: 5px;
    padding: 6px 8px 7px;
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 9px;
    color: var(--success);
    box-shadow: 0 14px 30px rgba(0, 0, 0, 0.6);
    opacity: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  /* Faint horizontal lines under the command to evoke "output rows". */
  .thumb::after {
    content: "";
    position: absolute;
    left: 8px;
    right: 8px;
    bottom: 7px;
    height: 22px;
    background:
      linear-gradient(180deg,
        rgba(255,255,255,0.16) 0 2px, transparent 2px 7px,
        rgba(255,255,255,0.12) 7px 9px, transparent 9px 14px,
        rgba(255,255,255,0.09) 14px 16px, transparent 16px 22px);
  }
  .thumb.t1 { animation: burst-1 900ms cubic-bezier(0.22, 1, 0.36, 1) forwards; }
  .thumb.t2 { animation: burst-2 950ms 50ms cubic-bezier(0.22, 1, 0.36, 1) forwards; }
  .thumb.t3 { animation: burst-3 1000ms 100ms cubic-bezier(0.22, 1, 0.36, 1) forwards; }
  .thumb.t4 { animation: burst-4 1050ms 150ms cubic-bezier(0.22, 1, 0.36, 1) forwards; }
  @keyframes burst-1 {
    0%   { opacity: 0; transform: translate(-50%, -50%) scale(0.6) rotate(0deg); }
    25%  { opacity: 1; transform: translate(-50%, -50%) scale(1) rotate(-2deg); }
    100% { opacity: 0; transform: translate(-200%, -140%) scale(0.88) rotate(-22deg); }
  }
  @keyframes burst-2 {
    0%   { opacity: 0; transform: translate(-50%, -50%) scale(0.6); }
    25%  { opacity: 1; transform: translate(-50%, -50%) scale(1) rotate(5deg); }
    100% { opacity: 0; transform: translate(80%, -160%) scale(0.94) rotate(18deg); }
  }
  @keyframes burst-3 {
    0%   { opacity: 0; transform: translate(-50%, -50%) scale(0.6); }
    25%  { opacity: 1; transform: translate(-50%, -50%) scale(1) rotate(-1deg); }
    100% { opacity: 0; transform: translate(-230%, 50%) scale(0.96) rotate(-28deg); }
  }
  @keyframes burst-4 {
    0%   { opacity: 0; transform: translate(-50%, -50%) scale(0.6); }
    25%  { opacity: 1; transform: translate(-50%, -50%) scale(1) rotate(3deg); }
    100% { opacity: 0; transform: translate(130%, 70%) scale(0.9) rotate(30deg); }
  }

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
  .caption .kw { color: var(--accent); font-weight: 700; }

  @media (prefers-reduced-motion: reduce) {
    .stage, .chip, .caption, .block, .bar, .ctx-menu, .chip-dot,
    .ln, .ln.folded-hint, .thumb {
      animation: none !important;
      transition: none !important;
      opacity: 1 !important;
      transform: none !important;
    }
    .burst-host { display: none !important; }
  }
</style>
