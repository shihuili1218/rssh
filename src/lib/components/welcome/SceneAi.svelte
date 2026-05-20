<!--
  Scene 1 — AI Diagnostics demo.
  Mouse glides to the AI button → click → panel slides in with focus
  zoom → typed prompt → user message → tool_use card → ✓ approved →
  caption → Next →.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../i18n/index.svelte.ts";
  import MockCursor from "./MockCursor.svelte";
  import NextButton from "./NextButton.svelte";

  let { onNext }: { onNext: () => void } = $props();

  // Cursor: starts off-stage lower-right, glides up to the AI button.
  let cursorX = $state("108%");
  let cursorY = $state("108%");
  let cursorVisible = $state(false);
  let cursorClicking = $state(false);

  let panelOpen = $state(false);
  let focusOn = $state(false);
  let typed = $state("");
  let sent = $state(false);
  let toolShown = $state(false);
  let approved = $state(false);
  let captionShown = $state(false);
  let ready = $state(false);

  const PROMPT = "why is the disk full?";

  let timers: number[] = [];
  function at(ms: number, fn: () => void) { timers.push(window.setTimeout(fn, ms)); }

  onMount(() => {
    const reduced =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

    if (reduced) {
      cursorVisible = true; cursorX = "94%"; cursorY = "9%";
      panelOpen = true; focusOn = true; typed = PROMPT;
      sent = true; toolShown = true; approved = true;
      captionShown = true; ready = true;
      return;
    }

    at(300,  () => { cursorVisible = true; });
    at(500,  () => { cursorX = "94%"; cursorY = "9%"; });
    at(1500, () => { cursorClicking = true; });
    at(1620, () => { cursorClicking = false; panelOpen = true; });
    at(2050, () => { focusOn = true; });
    for (let i = 1; i <= PROMPT.length; i++) {
      at(2500 + i * 55, () => { typed = PROMPT.slice(0, i); });
    }
    const typeEnd = 2500 + PROMPT.length * 55;
    at(typeEnd + 280, () => { sent = true; });
    at(typeEnd + 900, () => { toolShown = true; });
    at(typeEnd + 1500, () => { approved = true; });
    at(typeEnd + 2100, () => { captionShown = true; });
    at(typeEnd + 2900, () => { ready = true; });

    return () => { timers.forEach(window.clearTimeout); };
  });
</script>

<section class="scene">
  <div class="chip">
    <span class="chip-dot"></span>
    {t("welcome.scene.ai.chip")}
  </div>

  <div class="stage" class:focus-on={focusOn}>
    <div class="mock-app" class:open={panelOpen}>
      <div class="app-header">
        <div class="dots">
          <span class="dot r"></span><span class="dot y"></span><span class="dot g"></span>
        </div>
        <div class="app-title">rssh — prod-web-01</div>
        <button class="ai-btn" class:active={panelOpen} tabindex="-1">
          <span class="ai-glyph">✦</span> AI
        </button>
      </div>

      <div class="app-body">
        <div class="term-pane">
          <div class="ln"><span class="prompt">$</span> <span>uptime</span></div>
          <div class="ln out">up 14 days · load avg 1.42</div>
          <div class="ln"><span class="prompt">$</span> <span>df -h /</span></div>
          <div class="ln out warn">/dev/sda1   480G  478G    2G   <span class="hot">100%</span>  /</div>
          <div class="ln"><span class="prompt">$</span> <span class="cur-blink">_</span></div>
        </div>

        <aside class="ai-pane">
          <div class="ai-head">
            <span class="ai-dot"></span>
            <span class="ai-name">{t("welcome.scene.ai.panel_title")}</span>
            <span class="ai-model">claude-opus-4-7</span>
          </div>

          <div class="ai-thread">
            {#if sent}
              <div class="bubble user">{PROMPT}</div>
              <div class="bubble asst">
                {#if !toolShown}
                  <span class="dots-anim"><span></span><span></span><span></span></span>
                {:else}
                  <div class="asst-line">
                    {t("welcome.scene.ai.asst_intro")}
                  </div>
                  <div class="tool-card" class:approved>
                    <div class="tool-head">
                      <span class="tool-tag">tool_use</span>
                      <span class="tool-name">list_dir</span>
                      {#if approved}
                        <span class="tool-status">
                          <span class="ok">✓</span> {t("welcome.scene.ai.tool_approved")}
                        </span>
                      {/if}
                    </div>
                    <div class="tool-args">{`{ "path": "/var/log", "depth": 1 }`}</div>
                    <div class="tool-guard">
                      <span>shape ✓</span><span>redact ✓</span><span>approve {approved ? "✓" : "…"}</span>
                    </div>
                  </div>
                {/if}
              </div>
            {/if}
          </div>

          <div class="ai-input" class:focused={focusOn && !sent}>
            <span class="caret-bar">›</span>
            <span class="typed">{typed}</span>
            {#if !sent}<span class="caret-blink">_</span>{/if}
            <span class="enter-key" class:lit={sent}>⏎</span>
          </div>
        </aside>
      </div>
    </div>

    <MockCursor
      x={cursorX}
      y={cursorY}
      visible={cursorVisible}
      clicking={cursorClicking}
      duration={1000}
    />
  </div>

  <div class="caption" class:show={captionShown}>
    <span class="kw">{t("welcome.scene.ai.caption_kw1")}</span>
    {t("welcome.scene.ai.caption_join")}
    <span class="kw">{t("welcome.scene.ai.caption_kw2")}</span>
    {t("welcome.scene.ai.caption_join")}
    <span class="kw">{t("welcome.scene.ai.caption_kw3")}</span>
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
    align-self: center;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border: 1px solid color-mix(in srgb, var(--purple) 55%, transparent);
    border-radius: 999px;
    background: color-mix(in srgb, var(--purple) 10%, transparent);
    color: var(--purple);
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
    background: var(--purple);
    box-shadow: 0 0 8px var(--purple);
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

  /* The "focus zoom" — emphasise the AI panel + cursor by dimming the
     terminal and scaling/glowing the panel. This is the "放大聚焦"
     beat the user asked for. */
  .stage.focus-on .term-pane {
    filter: brightness(0.38) saturate(0.7) blur(1.5px);
    transition: filter 500ms ease;
  }

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
    user-select: none;
  }
  .dots { display: flex; gap: 7px; justify-self: start; }
  .dot { width: 12px; height: 12px; border-radius: 50%; }
  .dot.r { background: #ff5f57; } .dot.y { background: #febc2e; } .dot.g { background: #28c840; }
  .app-title {
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 11px;
    color: rgba(255, 255, 255, 0.55);
    letter-spacing: 0.6px;
  }

  .ai-btn {
    justify-self: end;
    background: rgba(168, 85, 247, 0.12);
    border: 1px solid color-mix(in srgb, var(--purple) 50%, transparent);
    color: var(--purple);
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.8px;
    padding: 5px 10px;
    border-radius: 6px;
    cursor: default;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    transition: background 0.2s ease, box-shadow 0.2s ease, transform 0.12s ease;
  }
  .ai-btn.active {
    background: color-mix(in srgb, var(--purple) 30%, transparent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--purple) 20%, transparent);
  }
  .ai-glyph { color: var(--purple); }

  .app-body {
    flex: 1;
    display: flex;
    position: relative;
    overflow: hidden;
  }

  .term-pane {
    flex: 1;
    padding: 16px 18px;
    font-family: "SF Mono", Menlo, Consolas, "Courier New", monospace;
    font-size: 13px;
    line-height: 1.65;
    color: #d4d8e2;
    min-width: 0;
  }
  .ln { white-space: pre-wrap; }
  .ln.out { color: #b8bcc8; }
  .ln.out.warn { color: #f0c674; }
  .ln .hot { color: #ff6b6b; font-weight: 700; }
  .prompt { color: var(--success); margin-right: 6px; }
  .cur-blink { color: var(--accent); animation: blink 1s steps(1, start) infinite; }
  @keyframes blink { 50% { opacity: 0; } }

  .ai-pane {
    position: absolute;
    top: 0; bottom: 0; right: 0;
    width: 42%;
    background: linear-gradient(180deg, #20212a 0%, #1c1d24 100%);
    border-left: 1px solid color-mix(in srgb, var(--purple) 25%, transparent);
    display: flex;
    flex-direction: column;
    transform: translateX(100%);
    transition: transform 650ms cubic-bezier(0.22, 1, 0.36, 1),
                box-shadow 500ms ease;
    z-index: 2;
  }
  .mock-app.open .ai-pane {
    transform: translateX(0);
    box-shadow: -20px 0 60px rgba(0, 0, 0, 0.5);
  }
  .stage.focus-on .ai-pane {
    box-shadow:
      -28px 0 80px rgba(0, 0, 0, 0.65),
      0 0 0 1px color-mix(in srgb, var(--purple) 35%, transparent);
  }

  .ai-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 14px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 11px;
  }
  .ai-dot {
    width: 7px; height: 7px; border-radius: 50%;
    background: var(--purple);
    box-shadow: 0 0 8px var(--purple);
  }
  .ai-name { color: var(--text); font-weight: 700; letter-spacing: 0.5px; }
  .ai-model {
    margin-left: auto;
    color: var(--text-dim);
    font-size: 10px;
    letter-spacing: 0.5px;
  }

  .ai-thread {
    flex: 1;
    padding: 14px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .bubble {
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 12px;
    padding: 8px 12px;
    border-radius: 12px;
    max-width: 100%;
    line-height: 1.45;
    opacity: 0;
    transform: translateY(6px);
    animation: bubble-in 360ms cubic-bezier(0.22, 1, 0.36, 1) forwards;
  }
  .bubble.user {
    align-self: flex-end;
    background: color-mix(in srgb, var(--accent) 25%, transparent);
    color: var(--text);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .bubble.asst {
    align-self: flex-start;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.06);
    color: var(--text);
    animation-delay: 200ms;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  @keyframes bubble-in { to { opacity: 1; transform: translateY(0); } }

  .asst-line { color: var(--text-sub); font-size: 11px; line-height: 1.5; }

  .dots-anim {
    display: inline-flex;
    gap: 4px;
    padding: 4px 0;
  }
  .dots-anim span {
    width: 5px; height: 5px;
    border-radius: 50%;
    background: var(--text-dim);
    animation: dotbounce 1.1s ease-in-out infinite;
  }
  .dots-anim span:nth-child(2) { animation-delay: 0.18s; }
  .dots-anim span:nth-child(3) { animation-delay: 0.36s; }
  @keyframes dotbounce {
    0%, 80%, 100% { transform: translateY(0); opacity: 0.4; }
    40%           { transform: translateY(-4px); opacity: 1; }
  }

  .tool-card {
    background: rgba(168, 85, 247, 0.08);
    border: 1px solid color-mix(in srgb, var(--purple) 35%, transparent);
    border-radius: 8px;
    padding: 8px 10px;
    font-size: 11px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    opacity: 0;
    transform: translateY(4px) scale(0.97);
    animation: tool-in 380ms cubic-bezier(0.22, 1, 0.36, 1) forwards;
  }
  @keyframes tool-in { to { opacity: 1; transform: translateY(0) scale(1); } }
  .tool-card.approved {
    background: color-mix(in srgb, var(--success) 8%, rgba(168, 85, 247, 0.06));
    border-color: color-mix(in srgb, var(--success) 40%, transparent);
  }
  .tool-head { display: flex; align-items: center; gap: 8px; }
  .tool-tag {
    background: var(--purple);
    color: var(--white);
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 9px;
    letter-spacing: 0.8px;
    font-weight: 700;
    text-transform: uppercase;
  }
  .tool-name { color: var(--text); font-weight: 700; }
  .tool-status {
    margin-left: auto;
    color: var(--success);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.5px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .ok { font-size: 12px; }
  .tool-args {
    color: var(--text-sub);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 10px;
    background: rgba(0,0,0,0.25);
    padding: 4px 8px;
    border-radius: 4px;
  }
  .tool-guard {
    display: flex;
    gap: 10px;
    font-size: 9.5px;
    color: var(--text-dim);
    letter-spacing: 0.4px;
  }

  .ai-input {
    margin: 0 12px 12px;
    padding: 10px 12px;
    border-radius: 10px;
    background: rgba(0, 0, 0, 0.35);
    border: 1px solid rgba(255, 255, 255, 0.07);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 12px;
    display: flex;
    align-items: center;
    gap: 6px;
    transition: transform 360ms cubic-bezier(0.22, 1, 0.36, 1),
                box-shadow 360ms ease,
                border-color 360ms ease;
    transform-origin: bottom center;
  }
  .ai-input.focused {
    transform: scale(1.08);
    border-color: color-mix(in srgb, var(--purple) 60%, transparent);
    box-shadow:
      0 0 0 4px color-mix(in srgb, var(--purple) 20%, transparent),
      0 14px 36px rgba(0, 0, 0, 0.5);
  }
  .caret-bar { color: var(--purple); font-weight: 700; }
  .typed { color: var(--text); }
  .caret-blink {
    color: var(--purple);
    animation: blink 1s steps(1, start) infinite;
    font-weight: 700;
  }
  .enter-key {
    margin-left: auto;
    width: 24px;
    height: 20px;
    border-radius: 4px;
    background: rgba(255,255,255,0.06);
    color: var(--text-dim);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    border: 1px solid rgba(255,255,255,0.08);
    transition: background 0.18s ease, color 0.18s ease, box-shadow 0.18s ease;
  }
  .enter-key.lit {
    background: var(--accent);
    color: var(--white);
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent) 20%, transparent);
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
  .caption .kw { color: var(--purple); font-weight: 700; }

  @media (prefers-reduced-motion: reduce) {
    .stage, .chip, .caption, .bubble, .tool-card, .ai-pane, .ai-input,
    .dots-anim span, .cur-blink, .caret-blink, .chip-dot {
      animation: none !important;
      transition: none !important;
      opacity: 1 !important;
      transform: none !important;
    }
  }
</style>
