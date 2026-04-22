<script lang="ts">
  import type { Component } from "svelte";
  import * as app from "../stores/app.svelte.ts";
  import { t, locale, setLocale, AVAILABLE_LOCALES, type Locale } from "../i18n/index.svelte.ts";
  import ProfileManager from "./ProfileManager.svelte";
  import ProfileEditor from "./ProfileEditor.svelte";
  import CredentialManager from "./CredentialManager.svelte";
  import CredentialEditor from "./CredentialEditor.svelte";
  import ForwardManager from "./ForwardManager.svelte";
  import ForwardEditor from "./ForwardEditor.svelte";
  import GroupManager from "./GroupManager.svelte";
  import SnippetManager from "./SnippetManager.svelte";
  import HighlightManager from "./HighlightManager.svelte";
  import GitHubSyncScreen from "./GitHubSyncScreen.svelte";
  import ImportExportScreen from "./ImportExportScreen.svelte";
  import RecordingSettings from "./RecordingSettings.svelte";
  import PlaybackScreen from "./PlaybackScreen.svelte";
  import ShortcutsScreen from "./ShortcutsScreen.svelte";
  import AboutScreen from "./AboutScreen.svelte";
  import ShellSettings from "./ShellSettings.svelte";
  import CliSettings from "./CliSettings.svelte";
  import AppearanceSettings from "./AppearanceSettings.svelte";

  type MenuItem = { id: app.SettingsPage; label: string; section: string };

  type PageRoute = { component: Component<any>; needsId?: boolean };
  const routes: Partial<Record<app.SettingsPage, PageRoute>> = {
    "profiles":           { component: ProfileManager },
    "profile-edit":       { component: ProfileEditor, needsId: true },
    "credentials":        { component: CredentialManager },
    "credential-edit":    { component: CredentialEditor, needsId: true },
    "forwards":           { component: ForwardManager },
    "forward-edit":       { component: ForwardEditor, needsId: true },
    "groups":             { component: GroupManager },
    "snippets":           { component: SnippetManager },
    "highlights":         { component: HighlightManager },
    "github-sync":        { component: GitHubSyncScreen },
    "import-export":      { component: ImportExportScreen },
    "shell-settings":     { component: ShellSettings },
    "recording-settings": { component: RecordingSettings },
    "playback":           { component: PlaybackScreen },
    "cli":                { component: CliSettings },
    "appearance":         { component: AppearanceSettings },
    "shortcuts":          { component: ShortcutsScreen },
    "about":              { component: AboutScreen },
  };

  const COMPACT_BREAKPOINT = 640;
  let compact = $state(window.innerWidth < COMPACT_BREAKPOINT);

  $effect(() => {
    const mq = window.matchMedia(`(max-width: ${COMPACT_BREAKPOINT - 1}px)`);
    const onChange = (e: MediaQueryListEvent) => { compact = e.matches; };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  });

  // 注：菜单数据用 t() 直接调用，配合 $derived 触发响应式更新
  let allMenu = $derived<MenuItem[]>([
    { id: "profiles", label: t("settings.section.profiles"), section: "Connections" },
    { id: "credentials", label: t("settings.section.credentials"), section: "Connections" },
    { id: "forwards", label: t("settings.section.forwards"), section: "Connections" },
    { id: "groups", label: t("settings.section.groups"), section: "Connections" },
    { id: "import-export", label: t("settings.section.import_export"), section: "Connections" },
    { id: "github-sync", label: t("settings.section.github_sync"), section: "Connections" },
    { id: "shell-settings", label: t("settings.section.shell"), section: "Sessions" },
    { id: "recording-settings", label: t("settings.section.recording"), section: "Sessions" },
    { id: "appearance", label: t("settings.section.appearance"), section: "Appearance" },
    { id: "highlights", label: t("settings.section.highlights"), section: "Appearance" },
    { id: "snippets", label: t("settings.section.snippets"), section: "Appearance" },
    { id: "cli", label: t("settings.section.cli"), section: "Help" },
    { id: "shortcuts", label: t("settings.section.shortcuts"), section: "Help" },
    { id: "about", label: t("settings.section.about"), section: "Help" },
  ]);

  const hiddenOnCompact = new Set<string>(["cli", "shortcuts", "about"]);
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
    if (id === "groups" && p === "group-edit") return true;
    return false;
  }
</script>

<div class="settings-layout" class:compact>
  {#if !compact || app.settingsPage() === "menu"}
  <nav class="settings-menu">
    <div class="menu-header">{t("settings.title")}</div>
    {#each sections as s}
      <div class="section-label">{s.section}</div>
      {#each s.items as item}
        <button
          class="menu-item"
          class:active={isActive(item.id)}
          onclick={() => app.settingsNavigate(item.id)}
        >
          {item.label}
        </button>
      {/each}
    {/each}
    <select
      class="lang-select"
      value={locale()}
      onchange={(e) => setLocale((e.currentTarget as HTMLSelectElement).value as Locale)}
    >
      {#each AVAILABLE_LOCALES as l}
        <option value={l.code}>{l.label}</option>
      {/each}
    </select>
  </nav>
  {/if}

  {#if !compact || app.settingsPage() !== "menu"}
  <div class="settings-content">
    {#if compact}
      <button class="mobile-back" onclick={() => app.settingsBack()}>← {t("common.back")}</button>
    {/if}
    {#if app.settingsPage() === "menu"}
      <div class="welcome">
        <h2>{t("settings.title")}</h2>
        <p>Select a category from the menu</p>
      </div>
    {:else}
      {@const route = routes[app.settingsPage()]}
      {#if route}
        {@const C = route.component}
        {#if route.needsId}
          <C id={app.editingId()} />
        {:else}
          <C />
        {/if}
      {/if}
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
  .lang-select {
    width: 100%;
    padding: 6px 10px;
    border: 1px solid var(--divider);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
    font-family: inherit;
    font-size: 13px;
    cursor: pointer;
  }

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
