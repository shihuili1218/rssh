<script lang="ts">
  const isMac = navigator.platform?.startsWith("Mac");
  const mod = isMac ? "⌘" : "Ctrl";

  const sections = [
    {
      title: "Global",
      keys: [
        ["Ctrl+Tab", "Switch to next tab"],
        ["Ctrl+Shift+Tab", "Switch to previous tab"],
        [`${mod}+W`, "Close current tab"],
        [`${mod}+Shift+D`, "Clone current tab"],
        [`${mod}+Shift+N`, "Open current tab in new window"],
        ["Esc", "Close sidebar / search bar / SFTP"],
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
        ["Any key", "Reconnect after disconnect"],
      ],
    },
    {
      title: "Port Forward",
      keys: [
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
