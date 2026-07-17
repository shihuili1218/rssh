<script lang="ts">
  import type { AppIconName } from "./app-icon";

  let {
    name,
    size = 16,
    strokeWidth = 1.8,
    filled = false,
  }: {
    name: AppIconName;
    size?: number;
    strokeWidth?: number;
    filled?: boolean;
  } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={filled ? "currentColor" : "none"}
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  {#if name === "add"}
    <path d="M12 5v14M5 12h14" />
  {:else if name === "ai"}
    <path d="M12 3.5c.8 4.4 3.1 6.7 7.5 7.5-4.4.8-6.7 3.1-7.5 7.5-.8-4.4-3.1-6.7-7.5-7.5 4.4-.8 6.7-3.1 7.5-7.5Z" />
    <path d="M19 3v3M20.5 4.5h-3" />
  {:else if name === "check"}
    <path d="m5 12 4 4 10-10" />
  {:else if name === "cloud-download" || name === "cloud-upload"}
    <path d="M7 18h-1a4 4 0 0 1-.5-8A6.5 6.5 0 0 1 18 9.2 4.5 4.5 0 0 1 18.5 18H17" />
    {#if name === "cloud-upload"}
      <path d="M12 20V11m-3 3 3-3 3 3" />
    {:else}
      <path d="M12 11v9m-3-3 3 3 3-3" />
    {/if}
  {:else if name === "docker"}
    <path d="M4 10h3v3H4zm4 0h3v3H8zm4 0h3v3h-3zM8 6h3v3H8zm4 0h3v3h-3zm4 4h3v3h-3z" />
    <path d="M3 14h16c-.6 3.8-3.2 6-7.6 6H9c-3.1 0-5.2-2-6-6Zm16-2c1.4 0 2.2.6 2.7 1.5-1 .7-2 .9-3 .7" />
  {:else if name === "edit"}
    <path d="M4 20h4l11-11a2.8 2.8 0 0 0-4-4L4 16v4Z" />
    <path d="m13.5 6.5 4 4M4 20l4-1" />
  {:else if name === "file"}
    <path d="M6 3h8l4 4v14H6z" />
    <path d="M14 3v5h4M9 13h6M9 17h6" />
  {:else if name === "folder"}
    <path d="M3 6.5h7l2 2h9v10.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />
    <path d="M3 9h18" />
  {:else if name === "forward"}
    <circle cx="5" cy="7" r="2" />
    <circle cx="19" cy="17" r="2" />
    <path d="M7 7h8l-2.5-2.5M15 7l-2.5 2.5M17 17H9l2.5-2.5M9 17l2.5 2.5" />
  {:else if name === "home"}
    <path d="m3 11 9-8 9 8" />
    <path d="M5.5 9.5V21h13V9.5M9.5 21v-7h5v7" />
  {:else if name === "kubernetes"}
    <path d="m12 2.8 7.2 4.1v8.2L12 19.2l-7.2-4.1V6.9Z" />
    <circle cx="12" cy="11" r="2.5" />
    <path d="M12 5.5V8m0 6v2.5M6.8 8l2.1 1.2m6.2 3.6 2.1 1.2m0-6-2.1 1.2m-6.2 3.6L6.8 14" />
  {:else if name === "link"}
    <path d="m9.5 14.5-1 1a3.5 3.5 0 0 1-5-5l3-3a3.5 3.5 0 0 1 5 0" />
    <path d="m14.5 9.5 1-1a3.5 3.5 0 0 1 5 5l-3 3a3.5 3.5 0 0 1-5 0" />
    <path d="m8.5 15.5 7-7" />
  {:else if name === "pin"}
    <path d="m9 3 6 2-1 5 3 3-4 1-3 7-1-7-4-1 4-3Z" />
  {:else if name === "save"}
    <path d="M4 3h13l3 3v15H4Z" />
    <path d="M8 3v6h8V3M8 21v-7h8v7" />
  {:else if name === "serial"}
    <path d="M5 5h14l2 12H3Z" />
    <circle cx="8" cy="10" r=".7" fill="currentColor" stroke="none" />
    <circle cx="12" cy="10" r=".7" fill="currentColor" stroke="none" />
    <circle cx="16" cy="10" r=".7" fill="currentColor" stroke="none" />
    <circle cx="10" cy="14" r=".7" fill="currentColor" stroke="none" />
    <circle cx="14" cy="14" r=".7" fill="currentColor" stroke="none" />
    <path d="M8 20h8" />
  {:else if name === "settings"}
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1A1.7 1.7 0 0 0 9 4.6 1.7 1.7 0 0 0 10 3v-.2h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z" />
  {:else if name === "snippet"}
    <path d="M13 2 5 14h6l-1 8 9-13h-6Z" />
  {:else if name === "ssh"}
    <rect x="3" y="4" width="18" height="16" rx="2" />
    <path d="m7 9 3 3-3 3M12 15h5" />
  {:else if name === "star"}
    <path d="m12 3 2.7 5.5 6.1.9-4.4 4.3 1 6.1-5.4-2.9-5.4 2.9 1-6.1-4.4-4.3 6.1-.9Z" />
  {:else if name === "telnet"}
    <circle cx="12" cy="12" r="9" />
    <path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" />
  {:else if name === "terminal"}
    <rect x="3" y="4" width="18" height="16" rx="2" />
    <path d="m7 9 3 3-3 3M13 15h4" />
  {:else if name === "transfer"}
    <path d="M8 4v16m0-16L4.5 7.5M8 4l3.5 3.5M16 20V4m0 16-3.5-3.5M16 20l3.5-3.5" />
  {:else if name === "warning"}
    <path d="M12 3 2.5 20h19Z" />
    <path d="M12 9v5m0 3h.01" />
  {/if}
</svg>

<style>
  svg { display: block; flex: none; }
</style>
