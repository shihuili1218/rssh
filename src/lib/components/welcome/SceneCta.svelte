<!--
  Scene 5 — Final CTA.
  Calm, no animation overhead. Big prompt line + Get Started button +
  control hints (Enter / Esc / Space). Calling onNext dismisses the
  welcome.
-->
<script lang="ts">
  import { t } from "../../i18n/index.svelte.ts";

  let { onNext, onReplay }: { onNext: () => void; onReplay?: () => void } = $props();
</script>

<section class="scene">
  <div class="glow" aria-hidden="true"></div>

  <div class="block">
    <div class="prompt-line">
      <span class="gt">&gt;</span>
      <span class="msg">{t("welcome.cta_ready.line")}</span>
      <span class="caret">_</span>
    </div>
    <div class="sub">{t("welcome.cta_ready.sub")}</div>

    <button class="primary" onclick={onNext}>
      <span class="lbl">{t("welcome.cta")}</span>
      <span class="arrow">→</span>
    </button>

    <div class="controls">
      <kbd>Enter</kbd> <span class="muted">{t("welcome.controls.hint_enter")}</span>
      <span class="dot-sep">·</span>
      <kbd>Esc</kbd> <span class="muted">{t("welcome.controls.hint_esc")}</span>
      {#if onReplay}
        <span class="dot-sep">·</span>
        <button class="text-btn" onclick={onReplay}>
          <kbd>Space</kbd> <span class="muted">{t("welcome.controls.hint_replay")}</span>
        </button>
      {/if}
    </div>
  </div>
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
    isolation: isolate;
    overflow: hidden;
  }
  .glow {
    position: absolute;
    inset: 0;
    background: radial-gradient(ellipse 55% 45% at center, color-mix(in srgb, var(--accent) 25%, transparent) 0%, transparent 70%);
    filter: blur(40px);
    z-index: -1;
    animation: breathe 4.5s ease-in-out infinite;
  }
  @keyframes breathe {
    0%, 100% { opacity: 0.8; }
    50%      { opacity: 0.5; }
  }

  .block {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 28px;
    text-align: center;
  }

  .prompt-line {
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: clamp(22px, 3.6vw, 40px);
    font-weight: 700;
    color: var(--text);
    letter-spacing: 0.3px;
    display: inline-flex;
    align-items: baseline;
    gap: 12px;
    opacity: 0;
    animation: line-in 600ms 100ms forwards;
  }
  @keyframes line-in {
    from { opacity: 0; transform: translateY(8px); }
    to   { opacity: 1; transform: translateY(0); }
  }
  .gt { color: var(--accent); }
  .msg {
    clip-path: inset(0 100% 0 0);
    animation: type-in 700ms 400ms steps(20, end) forwards;
  }
  @keyframes type-in { to { clip-path: inset(0 0% 0 0); } }
  .caret {
    color: var(--accent);
    animation: blink 1s steps(1, start) infinite;
  }
  @keyframes blink { 50% { opacity: 0; } }

  .sub {
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: clamp(12px, 1.3vw, 14px);
    color: var(--text-dim);
    letter-spacing: 0.6px;
    text-transform: uppercase;
    opacity: 0;
    animation: fade-up 500ms 1000ms forwards;
  }
  @keyframes fade-up {
    from { opacity: 0; transform: translateY(6px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  .primary {
    background: var(--accent);
    color: var(--white);
    border: none;
    padding: 16px 36px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: clamp(14px, 1.6vw, 16px);
    font-weight: 700;
    letter-spacing: 1.2px;
    text-transform: uppercase;
    display: inline-flex;
    align-items: center;
    gap: 10px;
    opacity: 0;
    transform: translateY(8px);
    animation: btn-in 600ms 1300ms cubic-bezier(0.22, 1, 0.36, 1) forwards,
               pulse 2.8s 2000ms ease-in-out infinite;
    box-shadow:
      0 0 0 0 color-mix(in srgb, var(--accent) 35%, transparent),
      0 14px 36px color-mix(in srgb, var(--accent) 40%, transparent);
    transition: background 0.18s ease, transform 0.12s ease;
  }
  @keyframes btn-in {
    to { opacity: 1; transform: translateY(0); }
  }
  @keyframes pulse {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 35%, transparent), 0 14px 36px color-mix(in srgb, var(--accent) 40%, transparent); }
    50%      { box-shadow: 0 0 0 14px color-mix(in srgb, var(--accent) 0%, transparent), 0 14px 36px color-mix(in srgb, var(--accent) 40%, transparent); }
  }
  .primary:hover { background: color-mix(in srgb, var(--accent) 90%, var(--white)); transform: translateY(-1px); }
  .primary:active { transform: translateY(0) scale(0.98); }
  .arrow { display: inline-block; transition: transform 0.18s ease; }
  .primary:hover .arrow { transform: translateX(3px); }

  .controls {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 11px;
    color: var(--text-dim);
    letter-spacing: 0.4px;
    opacity: 0;
    animation: fade-up 500ms 1700ms forwards;
    flex-wrap: wrap;
    justify-content: center;
  }
  kbd {
    font-family: inherit;
    background: rgba(255, 255, 255, 0.05);
    color: var(--text-sub);
    padding: 2px 7px;
    border-radius: 4px;
    border: 1px solid rgba(255, 255, 255, 0.08);
    font-size: 10px;
    letter-spacing: 0.3px;
  }
  .muted { color: var(--text-dim); }
  .dot-sep { color: var(--text-dim); opacity: 0.5; }
  .text-btn {
    background: none;
    border: none;
    color: inherit;
    font: inherit;
    padding: 0;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .text-btn:hover .muted { color: var(--accent); }

  @media (prefers-reduced-motion: reduce) {
    .glow, .prompt-line, .msg, .sub, .primary, .controls, .caret {
      animation: none !important;
      opacity: 1 !important;
      transform: none !important;
      clip-path: none !important;
    }
  }
</style>
