<!--
  Virtual mouse cursor for welcome demos. Position-driven via left/top
  (accepting CSS strings like "50%" or "300px") so scenes can target
  buttons by % of their own frame. Click ripple is triggered by toggling
  `clicking` true for one frame — caller manages the timing.
-->
<script lang="ts">
  let {
    x = "0%",
    y = "0%",
    visible = true,
    clicking = false,
    duration = 600,
  }: {
    x?: string;
    y?: string;
    visible?: boolean;
    clicking?: boolean;
    duration?: number;
  } = $props();
</script>

<div
  class="mock-cursor"
  class:visible
  class:clicking
  style="left: {x}; top: {y}; transition-duration: {duration}ms;"
  aria-hidden="true"
>
  <svg viewBox="0 0 16 18" width="22" height="24">
    <path
      d="M1 1 L1 14 L4.5 10.5 L7 16 L9 15 L6.5 9.5 L11.5 9.5 Z"
      fill="white"
      stroke="black"
      stroke-width="1"
      stroke-linejoin="round"
    />
  </svg>
  {#if clicking}
    <span class="ripple"></span>
  {/if}
</div>

<style>
  .mock-cursor {
    position: absolute;
    width: 22px;
    height: 24px;
    pointer-events: none;
    z-index: 50;
    opacity: 0;
    /* Arrow tip is at (1,1) of the 16x18 viewBox; nudge so the visible tip
       lands on the left/top coordinate the caller specifies. */
    margin-left: -2px;
    margin-top: -2px;
    transition-property: left, top, opacity;
    transition-timing-function: cubic-bezier(0.22, 1, 0.36, 1);
    filter: drop-shadow(0 2px 4px rgba(0, 0, 0, 0.4));
    will-change: left, top;
  }
  .mock-cursor.visible { opacity: 1; }

  .ripple {
    position: absolute;
    left: 2px;
    top: 2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent) 70%, transparent);
    transform: scale(0);
    animation: ripple 520ms ease-out forwards;
    pointer-events: none;
  }
  @keyframes ripple {
    0%   { transform: scale(0.4); opacity: 0.9; }
    100% { transform: scale(3.5); opacity: 0; }
  }

  @media (prefers-reduced-motion: reduce) {
    .mock-cursor { transition-duration: 0ms !important; }
    .ripple { animation-duration: 0ms !important; opacity: 0 !important; }
  }
</style>
