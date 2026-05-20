<!--
  Shared "Next →" button used at the end of every scene. Fades up
  when the scene's internal animation timeline completes.
-->
<script lang="ts">
  import { t } from "../../i18n/index.svelte.ts";

  let {
    ready = false,
    label,
    onClick,
  }: {
    ready?: boolean;
    label?: string;
    onClick: () => void;
  } = $props();
</script>

<button class="next-btn" class:ready onclick={onClick} disabled={!ready}>
  <span class="lbl">{label ?? t("welcome.controls.next")}</span>
  <span class="arrow">→</span>
</button>

<style>
  .next-btn {
    background: var(--accent);
    color: var(--white);
    border: none;
    padding: 11px 24px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 1px;
    text-transform: uppercase;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    opacity: 0;
    transform: translateY(8px);
    pointer-events: none;
    transition:
      opacity 0.45s ease,
      transform 0.45s cubic-bezier(0.22, 1, 0.36, 1),
      background 0.18s ease,
      box-shadow 0.18s ease;
    box-shadow:
      0 0 0 0 color-mix(in srgb, var(--accent) 30%, transparent),
      0 10px 28px color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .next-btn.ready {
    opacity: 1;
    transform: translateY(0);
    pointer-events: auto;
    animation: pulse-ready 2.6s 600ms ease-in-out infinite;
  }
  @keyframes pulse-ready {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 35%, transparent), 0 10px 28px color-mix(in srgb, var(--accent) 35%, transparent); }
    50%      { box-shadow: 0 0 0 10px color-mix(in srgb, var(--accent) 0%, transparent), 0 10px 28px color-mix(in srgb, var(--accent) 35%, transparent); }
  }
  .next-btn:hover {
    background: color-mix(in srgb, var(--accent) 92%, var(--white));
    transform: translateY(-1px);
  }
  .next-btn:active { transform: translateY(0) scale(0.98); }
  .arrow {
    display: inline-block;
    transition: transform 0.18s ease;
  }
  .next-btn:hover .arrow { transform: translateX(3px); }

  @media (prefers-reduced-motion: reduce) {
    .next-btn, .next-btn.ready { animation: none !important; }
  }
</style>
