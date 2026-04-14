<script lang="ts">
  const isMac = navigator.platform?.startsWith("Mac");
  const mod = isMac ? "⌘" : "Ctrl";

  const sections = [
    {
      title: "Global",
      keys: [
        [`${mod}+K`, "Toggle sidebar"],
        ["Esc", "Close sidebar / search bar / SFTP"],
      ],
    },
    {
      title: "Sidebar",
      keys: [
        ["↑ ↓", "Select tab / pinned profile"],
        ["Enter", "Switch to selected item"],
      ],
    },
    {
      title: "Home",
      keys: [
        ["↑ ↓ ← →", "Select Profile / Forward card"],
        ["Enter", "Connect selected Profile or open Forward"],
      ],
    },
    {
      title: "Terminal",
      keys: [
        [`${mod}+F`, "Search"],
        [`${mod}+S`, "Command Snippet (send to terminal)"],
        [`${mod}+O`, "Open SFTP (SSH session)"],
        [`${mod}+W`, "Close Current Tab"],
        ["Any key", "Reconnect after disconnect"],
      ],
    },
    {
      title: "Port Forward",
      keys: [
        [`${mod}+W`, "Close Current Tab"],
        ["Any key", "Reconnect after error/stop"],
      ],
    },
  ];
</script>

<div class="page">
  {#each sections as s}
    <div class="section-label">{s.title}</div>
    <div class="key-list">
      {#each s.keys as [key, desc]}
        <div class="key-row">
          <kbd class="kbd">{key}</kbd>
          <span class="key-desc">{desc}</span>
        </div>
      {/each}
    </div>
  {/each}
</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 4px; }
  .key-list { display: flex; flex-direction: column; gap: 2px; }
  .key-row {
    display: flex; align-items: center; gap: 12px;
    padding: 8px 0;
  }
  .kbd {
    display: inline-block;
    min-width: 80px;
    padding: 3px 8px;
    background: var(--surface);
    box-shadow: var(--pressed);
    border-radius: 6px;
    font-family: monospace;
    font-size: 12px;
    color: var(--text);
    text-align: center;
  }
  .key-desc { font-size: 13px; color: var(--text-sub); }
</style>
