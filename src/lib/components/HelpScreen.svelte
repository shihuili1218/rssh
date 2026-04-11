<script lang="ts">
  const isMac = navigator.platform?.startsWith("Mac");
  const mod = isMac ? "⌘" : "Ctrl";

  const sections = [
    {
      title: "全局",
      keys: [
        [`${mod}+K`, "打开/关闭侧边栏"],
        ["Esc", "关闭侧边栏 / 搜索栏 / SFTP"],
      ],
    },
    {
      title: "侧边栏",
      keys: [
        ["↑ ↓", "选择 tab / pinned profile"],
        ["Enter", "切换到选中项"],
      ],
    },
    {
      title: "Home 页面",
      keys: [
        ["↑ ↓ ← →", "选择 Profile / Forward 卡片"],
        ["Enter", "连接选中的 Profile 或打开 Forward"],
      ],
    },
    {
      title: "终端",
      keys: [
        [`${mod}+F`, "搜索"],
        [`${mod}+S`, "命令片段（选择后发送到终端）"],
        [`${mod}+O`, "打开 SFTP（SSH 会话）"],
        ["任意键", "断开后重连"],
      ],
    },
    {
      title: "端口转发",
      keys: [
        ["任意键", "错误/停止后重连"],
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
