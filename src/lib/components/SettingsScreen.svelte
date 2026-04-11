<script lang="ts">
  import * as app from "../stores/app.svelte.ts";
</script>

<div class="settings">
  <div class="header">
    <button class="btn btn-sm" onclick={() => app.navigate("main")}>← 返回</button>
    <h2>设置</h2>
  </div>

  <div class="menu">
    <div class="section-label">连接</div>
    <button class="menu-item" onclick={() => app.navigate("profiles")}>
      <span>Profiles</span><span class="arrow">›</span>
    </button>
    <button class="menu-item" onclick={() => app.navigate("credentials")}>
      <span>凭证管理</span><span class="arrow">›</span>
    </button>
    <button class="menu-item" onclick={() => app.navigate("forwards")}>
      <span>端口转发</span><span class="arrow">›</span>
    </button>
    <button class="menu-item" onclick={() => app.navigate("import-export")}>
      <span>导入 & 导出</span><span class="arrow">›</span>
    </button>
    <button class="menu-item" onclick={() => app.navigate("github-sync")}>
      <span>GitHub 同步</span><span class="arrow">›</span>
    </button>

    <div class="section-label">会话</div>
    <button class="menu-item" onclick={() => app.navigate("recording-settings")}>
      <span>会话录制</span><span class="arrow">›</span>
    </button>

    <div class="section-label">外观</div>
    <button class="menu-item" onclick={() => app.navigate("highlights")}>
      <span>关键词高亮</span><span class="arrow">›</span>
    </button>
    <button class="menu-item" onclick={() => app.navigate("snippets")}>
      <span>命令片段</span><span class="arrow">›</span>
    </button>
  </div>
</div>

<style>
  .settings { padding: 24px; max-width: 500px; margin: 0 auto; }
  .header { display: flex; align-items: center; gap: 12px; margin-bottom: 20px; }
  .header h2 { font-size: 18px; color: var(--text); }
  .menu { display: flex; flex-direction: column; gap: 2px; }
  .menu-item {
    display: flex; justify-content: space-between; align-items: center;
    padding: 12px 16px; border: none; border-radius: var(--radius-sm);
    background: transparent; color: var(--text); font-family: inherit;
    font-size: 14px; cursor: pointer; transition: background 0.15s;
  }
  .menu-item:hover { background: rgba(163,177,198,0.15); }
  .arrow { color: var(--text-dim); font-size: 18px; }
</style>
