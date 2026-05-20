<!--
  Scene 0 — Intro
  Giant blinking `>_` + welcome line + Start button.
  Caller (WelcomeScreen) owns advance; we just call onNext when user presses the
  Start button or Enter/Space. Esc is handled globally by the parent.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../i18n/index.svelte.ts";

  let { onNext }: { onNext: () => void } = $props();

  // Reveal the Start button once the intro animation has had time to land.
  let ready = $state(false);
  onMount(() => {
    const id = window.setTimeout(() => { ready = true; }, 1800);
    return () => window.clearTimeout(id);
  });
</script>

<section class="intro" aria-label={t("welcome.intro.aria")}>
  <div class="glow" aria-hidden="true"></div>

  <div class="prompt-block">
    <div class="big-prompt">
      <span class="gt">&gt;</span><span class="caret">_</span>
    </div>
    <div class="welcome-line">{t("welcome.intro.welcome")}</div>
    <div class="subline">{t("welcome.intro.sub")}</div>
  </div>

  <button class="start-btn" class:ready onclick={onNext}>
    <span class="bracket">[</span>
    <span class="label">{t("welcome.intro.start")}</span>
    <span class="bracket">]</span>
  </button>
</section>

<style>
  .intro {
    position: relative;
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: clamp(40px, 8vh, 80px);
    isolation: isolate;
    overflow: hidden;
  }

  .glow {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(ellipse 60% 50% at center 42%, color-mix(in srgb, var(--accent) 28%, transparent) 0%, transparent 70%);
    filter: blur(50px);
    z-index: -1;
    animation: glow-breathe 5s ease-in-out infinite;
  }
  @keyframes glow-breathe {
    0%, 100% { opacity: 0.85; }
    50%      { opacity: 0.55; }
  }

  .prompt-block {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 22px;
  }

  .big-prompt {
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: clamp(80px, 18vw, 200px);
    font-weight: 700;
    line-height: 1;
    color: var(--accent);
    text-shadow:
      0 0 12px color-mix(in srgb, var(--accent) 60%, transparent),
      0 0 32px color-mix(in srgb, var(--accent) 35%, transparent),
      0 0 64px color-mix(in srgb, var(--accent) 20%, transparent);
    display: flex;
    align-items: baseline;
    gap: 4px;
    letter-spacing: -0.04em;
    opacity: 0;
    transform: scale(0.86);
    animation: big-in 900ms 100ms cubic-bezier(0.22, 1, 0.36, 1) forwards;
  }
  @keyframes big-in {
    to { opacity: 1; transform: scale(1); }
  }
  .gt { display: inline-block; }
  .caret {
    display: inline-block;
    animation: caret-blink 1.05s steps(1, start) infinite;
  }
  @keyframes caret-blink {
    50% { opacity: 0; }
  }

  .welcome-line {
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: clamp(18px, 2.6vw, 28px);
    color: var(--text);
    letter-spacing: 0.5px;
    clip-path: inset(0 100% 0 0);
    animation: type-in 1000ms 900ms steps(24, end) forwards;
  }
  @keyframes type-in {
    to { clip-path: inset(0 0% 0 0); }
  }

  .subline {
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: clamp(11px, 1.1vw, 13px);
    color: var(--text-dim);
    letter-spacing: 0.8px;
    text-transform: uppercase;
    opacity: 0;
    animation: fade-up 600ms 1800ms forwards;
  }
  @keyframes fade-up {
    from { opacity: 0; transform: translateY(8px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  .start-btn {
    background: transparent;
    border: 1px solid color-mix(in srgb, var(--accent) 60%, transparent);
    color: var(--accent);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: clamp(13px, 1.4vw, 15px);
    font-weight: 700;
    letter-spacing: 1.2px;
    text-transform: uppercase;
    padding: 12px 28px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    opacity: 0;
    transform: translateY(12px);
    transition:
      background 0.18s ease,
      color 0.18s ease,
      box-shadow 0.2s ease,
      transform 0.12s ease,
      opacity 0.5s ease;
    pointer-events: none;
    box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .start-btn.ready {
    opacity: 1;
    transform: translateY(0);
    pointer-events: auto;
    animation: btn-breathe 2.4s 1s ease-in-out infinite;
  }
  @keyframes btn-breathe {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 30%, transparent); }
    50%      { box-shadow: 0 0 0 10px color-mix(in srgb, var(--accent) 0%, transparent); }
  }
  .start-btn:hover {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    box-shadow: 0 0 0 6px color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .start-btn:active { transform: scale(0.98); }
  .bracket { opacity: 0.55; }

  @media (prefers-reduced-motion: reduce) {
    .glow, .big-prompt, .welcome-line, .subline, .start-btn, .caret {
      animation: none !important;
      opacity: 1 !important;
      transform: none !important;
      clip-path: none !important;
    }
    .start-btn.ready { pointer-events: auto; }
  }
</style>
