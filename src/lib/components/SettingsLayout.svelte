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

  const COMPACT_BREAKPOINT = 640;
  let compact = $state(window.innerWidth < COMPACT_BREAKPOINT);

  $effect(() => {
    const mq = window.matchMedia(`(max-width: ${COMPACT_BREAKPOINT - 1}px)`);
    const onChange = (e: MediaQueryListEvent) => { compact = e.matches; };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  });

  const allMenu: MenuItem[] = [
    { id: "profiles", label: "Profile", section: "Connections" },
    { id: "credentials", label: "Credential", section: "Connections" },
    { id: "forwards", label: "Port Forward", section: "Connections" },
    { id: "import-export", label: "Import & Export", section: "Connections" },
    { id: "github-sync", label: "GitHub Sync", section: "Connections" },
    { id: "shell-settings", label: "Shell & Logs", section: "Sessions" },
    { id: "recording-settings", label: "Session Record", section: "Sessions" },
    { id: "highlights", label: "Key Word Highlight", section: "Appearance" },
    { id: "snippets", label: "Command Snippet", section: "Appearance" },
    { id: "cli", label: "CLI Tool", section: "Tools" },
    { id: "help", label: "Shortcuts", section: "Help" },
  ];

  const hiddenOnCompact = new Set(["cli", "help"]);
  let menu = $derived(compact ? allMenu.filter(m => !hiddenOnCompact.has(m.id)) : allMenu);

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

<div class="settings-layout" class:compact>
  {#if !compact || app.settingsPage() === "menu"}
  <nav class="settings-menu">
    <div class="menu-header">Setting</div>
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

  {#if !compact || app.settingsPage() !== "menu"}
  <div class="settings-content">
    {#if compact}
      <button class="mobile-back" onclick={() => app.settingsBack()}>← Back</button>
    {/if}
    {#if app.settingsPage() === "menu"}
      <div class="welcome">
        <h2>Settings</h2>
        <p>Select a category from the menu</p>
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
    padding: 4px 0px 4px;
  }

  .settings-menu .section-label {
    padding: 8px 0px 8px;
    font-size: 8px;
  }

  .menu-item {
    display: block;
    width: 100%;
    padding: 9px 12px;
    margin-bottom: 10px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--bg);
    box-shadow: var(--raised-sm);
    color: var(--text-sub);
    font-family: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    transition: box-shadow 0.15s, color 0.15s, transform 0.1s;
  }
  .menu-item:hover { box-shadow: var(--raised-sm); color: var(--text); }
  .menu-item:active { box-shadow: var(--pressed); transform: scale(0.99); }
  .menu-item.active {
    box-shadow: var(--pressed);
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
  .settings-layout.compact .settings-menu {
    width: 100%;
    border-right: none;
  }
  .settings-layout.compact .settings-content {
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
