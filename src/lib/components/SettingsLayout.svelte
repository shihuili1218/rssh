<script lang="ts">
  import * as app from "../stores/app.svelte.ts";
  import ProfileManager from "./ProfileManager.svelte";
  import ProfileEditor from "./ProfileEditor.svelte";
  import CredentialManager from "./CredentialManager.svelte";
  import CredentialEditor from "./CredentialEditor.svelte";
  import ForwardManager from "./ForwardManager.svelte";
  import ForwardEditor from "./ForwardEditor.svelte";
  import SnippetManager from "./SnippetManager.svelte";
  import HighlightManager from "./HighlightManager.svelte";
  import GitHubSyncScreen from "./GitHubSyncScreen.svelte";
  import ImportExportScreen from "./ImportExportScreen.svelte";
  import RecordingSettings from "./RecordingSettings.svelte";
  import PlaybackScreen from "./PlaybackScreen.svelte";
  import HelpScreen from "./HelpScreen.svelte";
  import ShellSettings from "./ShellSettings.svelte";
  import CliSettings from "./CliSettings.svelte";

  type MenuItem = { id: string; label: string; section: string };

  const allMenu: MenuItem[] = [
    { id: "profiles", label: "Profiles", section: "连接" },
    { id: "credentials", label: "凭证管理", section: "连接" },
    { id: "forwards", label: "端口转发", section: "连接" },
    { id: "import-export", label: "导入 & 导出", section: "连接" },
    { id: "github-sync", label: "GitHub 同步", section: "连接" },
    { id: "shell-settings", label: "Shell & 日志", section: "会话" },
    { id: "recording-settings", label: "会话录制", section: "会话" },
    { id: "highlights", label: "关键词高亮", section: "外观" },
    { id: "snippets", label: "命令片段", section: "外观" },
    { id: "cli", label: "CLI Tool", section: "工具" },
    { id: "help", label: "快捷键", section: "帮助" },
  ];

  const hiddenOnMobile = new Set(["cli", "help"]);
  const menu = app.isMobile ? allMenu.filter(m => !hiddenOnMobile.has(m.id)) : allMenu;

  let sections = $derived((() => {
    const seen = new Set<string>();
    return menu.reduce<{ section: string; items: MenuItem[] }[]>((acc, m) => {
      if (!seen.has(m.section)) {
        seen.add(m.section);
        acc.push({ section: m.section, items: [] });
      }
      acc[acc.length - 1].items.push(m);
      return acc;
    }, []);
  })());

  function isActive(id: string): boolean {
    const p = app.settingsPage();
    if (p === id) return true;
    if (id === "profiles" && p === "profile-edit") return true;
    if (id === "credentials" && p === "credential-edit") return true;
    if (id === "forwards" && p === "forward-edit") return true;
    return false;
  }
</script>

<div class="settings-layout" class:mobile={app.isMobile}>
  {#if !app.isMobile || app.settingsPage() === "menu"}
  <nav class="settings-menu">
    <div class="menu-header">设置</div>
    {#each sections as s}
      <div class="section-label">{s.section}</div>
      {#each s.items as item}
        <button
          class="menu-item"
          class:active={isActive(item.id)}
          onclick={() => app.settingsNavigate(item.id as any)}
        >
          {item.label}
        </button>
      {/each}
    {/each}
  </nav>
  {/if}

  {#if !app.isMobile || app.settingsPage() !== "menu"}
  <div class="settings-content">
    {#if app.isMobile}
      <button class="mobile-back" onclick={() => app.settingsBack()}>← 返回</button>
    {/if}
    {#if app.settingsPage() === "menu"}
      <div class="welcome">
        <h2>设置</h2>
        <p>从左侧菜单选择一个类别</p>
      </div>
    {:else if app.settingsPage() === "profiles"}
      <ProfileManager />
    {:else if app.settingsPage() === "profile-edit"}
      <ProfileEditor id={app.editingId()} />
    {:else if app.settingsPage() === "credentials"}
      <CredentialManager />
    {:else if app.settingsPage() === "credential-edit"}
      <CredentialEditor id={app.editingId()} />
    {:else if app.settingsPage() === "forwards"}
      <ForwardManager />
    {:else if app.settingsPage() === "forward-edit"}
      <ForwardEditor id={app.editingId()} />
    {:else if app.settingsPage() === "snippets"}
      <SnippetManager />
    {:else if app.settingsPage() === "highlights"}
      <HighlightManager />
    {:else if app.settingsPage() === "github-sync"}
      <GitHubSyncScreen />
    {:else if app.settingsPage() === "import-export"}
      <ImportExportScreen />
    {:else if app.settingsPage() === "shell-settings"}
      <ShellSettings />
    {:else if app.settingsPage() === "recording-settings"}
      <RecordingSettings />
    {:else if app.settingsPage() === "playback"}
      <PlaybackScreen />
    {:else if app.settingsPage() === "cli"}
      <CliSettings />
    {:else if app.settingsPage() === "help"}
      <HelpScreen />
    {/if}
  </div>
  {/if}
</div>

<style>
  .settings-layout {
    display: flex;
    height: 100%;
  }

  .settings-menu {
    width: 200px;
    flex-shrink: 0;
    background: var(--bg);
    border-right: 1px solid var(--divider);
    padding: 12px 8px;
    overflow-y: auto;
  }

  .menu-header {
    font-size: 16px;
    font-weight: 700;
    color: var(--text);
    padding: 8px 12px 16px;
  }

  .settings-menu .section-label {
    padding: 12px 12px 4px;
  }

  .menu-item {
    display: block;
    width: 100%;
    padding: 9px 12px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-sub);
    font-family: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    transition: all 0.15s;
  }
  .menu-item:hover { background: var(--surface); color: var(--text); }
  .menu-item.active {
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 600;
  }

  .settings-content {
    flex: 1;
    overflow-y: auto;
    min-width: 0;
  }

  .welcome {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-dim);
    gap: 8px;
  }
  .welcome h2 { font-size: 20px; color: var(--text-sub); }

  /* ── Mobile: stack navigation ── */
  .settings-layout.mobile .settings-menu {
    width: 100%;
    border-right: none;
  }
  .settings-layout.mobile .settings-content {
    width: 100%;
  }
  .mobile-back {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 12px 16px;
    border: none;
    border-bottom: 1px solid var(--divider);
    background: var(--bg);
    color: var(--accent);
    font-family: inherit;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    width: 100%;
    text-align: left;
  }
</style>
