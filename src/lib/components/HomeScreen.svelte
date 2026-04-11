<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import type { Profile, Credential, Forward } from "../stores/app.svelte.ts";

  let profiles = $state<Profile[]>([]);
  let credentials = $state<Credential[]>([]);
  let forwards = $state<Forward[]>([]);
  let query = $state("");

  // Grid nav: "profile" or "forward" section + index within that section
  let navSection = $state<"profile" | "forward">("profile");
  let navIdx = $state(-1);
  let profileGridEl: HTMLDivElement;
  let forwardGridEl: HTMLDivElement;

  let filtered = $derived(
    query
      ? profiles.filter(p => p.name.toLowerCase().includes(query.toLowerCase()) || p.host.toLowerCase().includes(query.toLowerCase()))
      : profiles
  );

  function getCols(gridEl: HTMLDivElement | undefined): number {
    if (!gridEl) return 3;
    const style = getComputedStyle(gridEl);
    return style.gridTemplateColumns.split(" ").length;
  }

  function handleHomeKey(e: KeyboardEvent) {
    if (app.activeTabId() !== "home" || app.settingsActive()) return;
    if (document.activeElement?.tagName === "INPUT") return;

    const pLen = filtered.length;
    const fLen = forwards.length;
    if (!pLen && !fLen) return;

    // Initialize nav if not started
    if (navIdx < 0 && (e.key.startsWith("Arrow") || e.key === "Enter")) {
      navSection = pLen > 0 ? "profile" : "forward";
      navIdx = 0;
      e.preventDefault();
      return;
    }

    const curLen = navSection === "profile" ? pLen : fLen;
    const cols = getCols(navSection === "profile" ? profileGridEl : forwardGridEl);

    if (e.key === "ArrowRight") {
      e.preventDefault();
      navIdx = Math.min(navIdx + 1, curLen - 1);
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      navIdx = Math.max(navIdx - 1, 0);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      if (navIdx + cols < curLen) {
        navIdx += cols;
      } else if (navSection === "profile" && fLen > 0) {
        // Jump to forward section
        navSection = "forward";
        navIdx = Math.min(navIdx % cols, fLen - 1);
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (navIdx - cols >= 0) {
        navIdx -= cols;
      } else if (navSection === "forward" && pLen > 0) {
        // Jump to profile section, last row
        navSection = "profile";
        const fCols = getCols(forwardGridEl);
        const col = navIdx % fCols;
        const pCols = getCols(profileGridEl);
        const lastRowStart = Math.floor((pLen - 1) / pCols) * pCols;
        navIdx = Math.min(lastRowStart + col, pLen - 1);
      }
    } else if (e.key === "Enter" && navIdx >= 0) {
      e.preventDefault();
      if (navSection === "profile" && navIdx < pLen) connectProfile(filtered[navIdx]);
      else if (navSection === "forward" && navIdx < fLen) openForward(forwards[navIdx]);
    }
  }

  onMount(refresh);

  $effect(() => {
    if (app.activeTabId() === "home" && !app.settingsActive()) refresh();
  });

  async function refresh() {
    [profiles, credentials, forwards] = await Promise.all([
      app.loadProfiles(), app.loadCredentials(), app.loadForwards(),
    ]);
  }

  function credentialFor(p: Profile): Credential | undefined {
    return credentials.find(c => c.id === p.credential_id);
  }

  function profileFor(f: Forward): Profile | undefined {
    return profiles.find(p => p.id === f.profile_id);
  }

  function connectProfile(p: Profile) {
    const tabId = `ssh:${crypto.randomUUID()}`;
    const cred = credentialFor(p);
    app.addTab({
      id: tabId, type: "ssh", label: p.name,
      meta: {
        profileId: p.id,
        host: p.host, port: String(p.port),
        username: cred?.username ?? "",
        authType: cred?.type ?? "password",
        secret: cred?.secret ?? "",
      },
    });
  }

  function openForward(f: Forward) {
    const fp = profileFor(f);
    const id = `fwd:${f.id}:${Date.now()}`;
    app.addTab({
      id, type: "forward", label: f.name,
      meta: {
        forwardId: f.id, name: f.name,
        forwardType: f.type,
        localPort: String(f.local_port),
        remoteHost: f.remote_host,
        remotePort: String(f.remote_port),
        profileName: fp?.name ?? "?",
      },
    });
  }
</script>

<svelte:window onkeydown={handleHomeKey} />

<div class="home">
  <div class="home-header">
    <h1 class="logo">SShell</h1>
    <input class="search-input" type="text" bind:value={query} placeholder="搜索..." />
  </div>

  {#if filtered.length > 0}
    <div class="section-label">PROFILES</div>
    <div class="grid" bind:this={profileGridEl}>
      {#each filtered as p, i (p.id)}
        {@const cred = credentialFor(p)}
        <div class="card-wrap">
          <button
            class="card-btn neu-raised"
            class:selected={navSection === "profile" && navIdx === i}
            onclick={() => connectProfile(p)}
          >
            <div class="card-icon">S</div>
            <div class="card-body">
              <div class="card-name">{p.name}</div>
              <div class="card-sub">{cred?.username ?? "?"}@{p.host}:{p.port}</div>
            </div>
          </button>
          <button
            class="pin-btn"
            class:pinned={app.isProfilePinned(p.id)}
            title={app.isProfilePinned(p.id) ? "取消固定" : "固定到侧栏"}
            onclick={(e) => { e.stopPropagation(); app.isProfilePinned(p.id) ? app.unpinProfile(p.id) : app.pinProfile(p.id); }}
          >{app.isProfilePinned(p.id) ? "\u2605" : "\u2606"}</button>
        </div>
      {/each}
    </div>
  {/if}

  {#if forwards.length > 0}
    <div class="section-label">PORT FORWARDS</div>
    <div class="grid" bind:this={forwardGridEl}>
      {#each forwards as f, i (f.id)}
        {@const fp = profileFor(f)}
        <button
          class="card-btn neu-raised"
          class:selected={navSection === "forward" && navIdx === i}
          onclick={() => openForward(f)}
        >
          <div class="card-icon fwd">{f.type === "local" ? "L" : "R"}</div>
          <div class="card-body">
            <div class="card-name">{f.name}</div>
            <div class="card-sub">:{f.local_port} → {f.remote_host}:{f.remote_port}</div>
            <div class="card-via">via {fp?.name ?? "?"}</div>
          </div>
        </button>
      {/each}
    </div>
  {/if}

  {#if profiles.length === 0 && forwards.length === 0}
    <div class="empty-state">
      <p>还没有 Profile 或端口转发</p>
      <button class="btn btn-accent" onclick={() => app.navigate("settings")}>前往设置添加</button>
    </div>
  {/if}
</div>

<style>
  .home { padding: 24px; }
  .home-header { display: flex; align-items: center; gap: 16px; margin-bottom: 20px; }
  .logo { font-size: 22px; color: var(--accent); font-weight: 700; white-space: nowrap; }
  .search-input { flex: 1; }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 14px;
    margin-bottom: 8px;
  }

  .card-wrap { position: relative; }
  .pin-btn {
    position: absolute; top: 6px; right: 6px;
    background: none; border: none; font-size: 16px;
    color: var(--text-dim); cursor: pointer;
    opacity: 0; transition: opacity 0.15s;
    padding: 2px 4px; line-height: 1;
  }
  .card-wrap:hover .pin-btn, .pin-btn.pinned { opacity: 1; }
  .pin-btn.pinned { color: var(--warning); }
  .pin-btn:hover { color: var(--accent); }

  .card-btn {
    display: flex; align-items: flex-start; gap: 12px;
    padding: 14px; text-align: left; width: 100%;
    border: none; cursor: pointer; font-family: inherit;
    transition: box-shadow 0.2s, transform 0.1s;
  }
  .card-btn:hover { transform: translateY(-2px); }
  .card-btn.selected { outline: 2px solid var(--accent); outline-offset: -2px; }
  .card-btn:active { box-shadow: var(--pressed); transform: translateY(0); }

  .card-icon {
    width: 36px; height: 36px; border-radius: 10px;
    background: var(--accent-soft); color: var(--accent);
    display: flex; align-items: center; justify-content: center;
    font-weight: 700; font-size: 14px; flex-shrink: 0;
  }
  .card-icon.fwd { background: rgba(76,184,138,0.15); color: var(--success); }

  .card-body { flex: 1; min-width: 0; }
  .card-name { font-weight: 600; font-size: 14px; color: var(--text); margin-bottom: 2px; }
  .card-sub { font-size: 12px; color: var(--text-sub); font-family: monospace; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .card-via { font-size: 11px; color: var(--text-dim); margin-top: 2px; }

  .empty-state { text-align: center; padding: 60px 24px; color: var(--text-dim); }
  .empty-state p { margin-bottom: 12px; }
</style>
