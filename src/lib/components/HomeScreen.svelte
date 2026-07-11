<script lang="ts">
  import { onMount } from "svelte";
  import * as app from "../stores/app.svelte.ts";
  import type {
    Profile,
    Credential,
    Forward,
    Group,
    SerialProfile,
    TelnetProfile,
    DynamicDiscoveredTarget,
  } from "../stores/app.svelte.ts";
  import { errMsg, locale, t } from "../i18n/index.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { createHomeRefresh } from "./home-refresh.ts";
  import {
    buildHomeSections,
    HOME_VIEW_STORAGE_KEY,
    loadHomeViewPreferences,
    parseHomeViewPreferences,
    updateHomeViewPreferences,
    type HomeGrouping,
    type HomeItemKind,
    type HomeLayoutItem,
    type HomeSorting,
    type HomeViewPreferences,
  } from "./home-layout.ts";

  let profiles = $state<Profile[]>([]);
  let credentials = $state<Credential[]>([]);
  let forwards = $state<Forward[]>([]);
  let groups = $state<Group[]>([]);
  let serialProfiles = $state<SerialProfile[]>([]);
  let telnetProfiles = $state<TelnetProfile[]>([]);
  let dynamicTargets = $state<DynamicDiscoveredTarget[]>([]);
  let query = $state("");
  let viewPreferences = $state<HomeViewPreferences>(loadHomeViewPreferences());

  onMount(() => {
    const syncPreferences = (event: StorageEvent) => {
      if (event.key !== HOME_VIEW_STORAGE_KEY) return;
      viewPreferences = parseHomeViewPreferences(event.newValue);
    };
    window.addEventListener("storage", syncPreferences);
    return () => window.removeEventListener("storage", syncPreferences);
  });

  const homeRefresh = createHomeRefresh({
    loadStatic: async () => {
      const [
        loadedProfiles,
        loadedCredentials,
        loadedForwards,
        loadedGroups,
        loadedSerial,
        loadedTelnet,
      ] = await Promise.all([
        app.loadProfiles(),
        app.loadCredentials(),
        app.loadForwards(),
        app.loadGroups(),
        app.loadSerialProfiles(),
        app.loadTelnetProfiles(),
      ]);
      return {
        loadedProfiles,
        loadedCredentials,
        loadedForwards,
        loadedGroups,
        loadedSerial,
        loadedTelnet,
      };
    },
    loadDynamic: () => app.discoverDynamicTargets(),
    applyStatic: (loaded) => {
      profiles = loaded.loadedProfiles;
      credentials = loaded.loadedCredentials;
      forwards = loaded.loadedForwards;
      groups = loaded.loadedGroups;
      serialProfiles = loaded.loadedSerial;
      telnetProfiles = loaded.loadedTelnet;
    },
    applyDynamic: (snapshot) => {
      dynamicTargets = snapshot.targets;
    },
    onError: (error) => toast.error(errMsg(error)),
  });

  // One global nav index into the flat (display-order) item list.
  let navIdx = $state(-1);
  // One grid element per section; all share the same CSS so any one gives the
  // column count. Plain array (not $state): only read inside the key handler.
  let gridEls: HTMLDivElement[] = [];

  // ── Normalize saved and discovered targets into one shape ──────────────────
  // Static profiles and dynamic connector results share the same HomeItem path.
  // Opening behavior lives on the item itself, so Home does not assume that every
  // discovered target is a container/pod connector.
  interface HomeItem extends HomeLayoutItem {
    sub: string;
    icon: string;
    iconClass: string;
    pinProfileId: string | null;
    open: () => void;
  }

  function dynamicTargetPresentation(target: DynamicDiscoveredTarget): Pick<HomeItem, "icon" | "iconClass"> {
    switch (target.connector_spec.type) {
      case "docker_exec":
        return { icon: "D", iconClass: "docker" };
      case "kubectl_exec":
        return { icon: "K", iconClass: "k8s" };
    }

    const _exhaustive: never = target.connector_spec;
    throw new Error(`Unsupported connector spec: ${JSON.stringify(_exhaustive)}`);
  }

  let allItems = $derived<HomeItem[]>([
    ...profiles.map((p): HomeItem => {
      const cred = credentials.find((c) => c.id === p.credential_id);
      return {
        kind: "ssh", id: `ssh:${p.id}`, name: p.name,
        sub: `${cred?.username ?? "?"}@${p.host}:${p.port}`,
        icon: "S", iconClass: "", groupId: p.group_id ?? null,
        pinProfileId: p.id, open: () => connectProfile(p),
      };
    }),
    ...forwards.map((f): HomeItem => ({
      kind: "forward", id: `forward:${f.id}`, name: f.name,
      sub: `:${f.local_port} → ${f.remote_host}:${f.remote_port}`,
      icon: f.type === "dynamic" ? "D" : f.type === "local" ? "L" : "R",
      iconClass: "fwd", groupId: f.group_id ?? null,
      pinProfileId: null, open: () => openForward(f),
    })),
    ...serialProfiles.map((s): HomeItem => ({
      kind: "serial", id: `serial:${s.id}`, name: s.name,
      sub: `${s.port} · ${s.baud_rate}`,
      icon: "⎓", iconClass: "serial", groupId: s.group_id ?? null,
      pinProfileId: null, open: () => app.connectSerialProfile(s),
    })),
    ...telnetProfiles.map((tp): HomeItem => ({
      kind: "telnet", id: `telnet:${tp.id}`, name: tp.name,
      sub: `${tp.host}:${tp.port}`,
      icon: "T", iconClass: "telnet", groupId: tp.group_id ?? null,
      pinProfileId: null, open: () => app.connectTelnetProfile(tp),
    })),
    ...dynamicTargets.map((target): HomeItem => {
      const presentation = dynamicTargetPresentation(target);
      return {
        kind: target.connector_spec.type,
        id: `dynamic:${target.source_id}:${target.id}`,
        name: target.name,
        sub: `${target.source_name} · ${target.sub}`,
        icon: presentation.icon,
        iconClass: presentation.iconClass,
        groupId: null,
        pinProfileId: null,
        open: () => app.connectDynamicTarget(target),
      };
    }),
  ]);

  // Search filters all item kinds at once (#4): match the name or the sub-line
  // (which carries host / ports / baud), case-insensitive.
  let filtered = $derived(
    query
      ? allItems.filter((it) => {
          const q = query.toLowerCase();
          return it.name.toLowerCase().includes(q) || it.sub.toLowerCase().includes(q);
        })
      : allItems
  );

  function kindLabel(kind: HomeItemKind): string {
    switch (kind) {
      case "ssh": return t("settings.section.profiles");
      case "forward": return t("settings.section.forwards");
      case "serial": return t("settings.section.serial");
      case "telnet": return t("settings.section.telnet");
      case "docker_exec": return t("home.type.docker");
      case "kubectl_exec": return t("home.type.kubernetes");
    }
  }

  // Section order is independent from item sorting. Group mode keeps the
  // configured group order; type mode uses the fixed connection-kind order.
  let sections = $derived(buildHomeSections({
    grouping: viewPreferences.grouping,
    sorting: viewPreferences.sorting,
    items: filtered,
    groups: groups.map((group: Group) => ({
      id: group.id,
      name: group.name,
      color: group.color,
      sortOrder: group.sort_order,
    })),
    recentItemIds: app.recentHomeItemIds(),
    ungroupedLabel: t("profile.ungrouped"),
    kindLabel,
    locale: locale(),
  }));

  // Flat list in display order — what arrow-key nav walks.
  let navItems = $derived(sections.flatMap((section) => section.items));

  function getCols(): number {
    const el = gridEls.find(Boolean);
    if (!el) return 3;
    return getComputedStyle(el).gridTemplateColumns.split(" ").length;
  }

  // Locate which section a global navIdx falls into.
  function findSection(idx: number) {
    for (let sIdx = 0; sIdx < sections.length; sIdx++) {
      const s = sections[sIdx];
      if (idx < s.offset + s.items.length) return { s, sIdx, i: idx - s.offset };
    }
    return null;
  }

  function handleHomeKey(e: KeyboardEvent) {
    if (app.activeTabId() !== "home" || app.settingsActive()) return;
    if (e.target instanceof Element && e.target.closest(".home-controls, .search-input")) return;
    const total = navItems.length;
    if (!total) return;

    // Initialize nav on first arrow / Enter — top-left.
    if (navIdx < 0 && (e.key.startsWith("Arrow") || e.key === "Enter")) {
      navIdx = 0;
      e.preventDefault();
      return;
    }
    const cols = getCols();

    if (e.key === "ArrowRight") {
      e.preventDefault();
      navIdx = Math.min(navIdx + 1, total - 1);
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      navIdx = Math.max(navIdx - 1, 0);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      const cur = findSection(navIdx);
      if (!cur) return;
      const col = cur.i % cols;
      if (cur.i + cols < cur.s.items.length) {
        navIdx = cur.s.offset + cur.i + cols; // next row within this section
      } else if (cur.sIdx + 1 < sections.length) {
        const next = sections[cur.sIdx + 1]; // next section, same column
        navIdx = next.offset + Math.min(col, next.items.length - 1);
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const cur = findSection(navIdx);
      if (!cur) return;
      const col = cur.i % cols;
      if (cur.i - cols >= 0) {
        navIdx = cur.s.offset + cur.i - cols; // prev row within this section
      } else if (cur.sIdx - 1 >= 0) {
        const prev = sections[cur.sIdx - 1]; // prev section, last row, same column
        const lastRowStart = Math.floor((prev.items.length - 1) / cols) * cols;
        navIdx = prev.offset + Math.min(lastRowStart + col, prev.items.length - 1);
      }
    } else if (e.key === "Enter" && navIdx >= 0 && navIdx < total) {
      e.preventDefault();
      activate(navItems[navIdx]);
    }
  }

  $effect(() => {
    if (app.activeTabId() !== "home" || app.settingsActive()) {
      homeRefresh.cancel();
      return;
    }
    void homeRefresh.refresh();
    return () => homeRefresh.cancel();
  });

  // Clear the selection whenever any layout input changes — otherwise the
  // highlight could point at a different item after filtering or reordering.
  // First arrow press re-enters at top-left.
  $effect(() => {
    void query;
    void viewPreferences.grouping;
    void viewPreferences.sorting;
    void navItems.map((item) => item.id).join("\0");
    navIdx = -1;
  });

  function setGrouping(grouping: HomeGrouping) {
    if (viewPreferences.grouping === grouping) return;
    viewPreferences = updateHomeViewPreferences(viewPreferences, { grouping });
  }

  function setSorting(sorting: HomeSorting) {
    if (viewPreferences.sorting === sorting) return;
    viewPreferences = updateHomeViewPreferences(viewPreferences, { sorting });
  }

  function activate(it: HomeItem) {
    it.open();
  }

  function toggleProfilePin(profileId: string) {
    if (app.isProfilePinned(profileId)) app.unpinProfile(profileId);
    else app.pinProfile(profileId);
  }

  function connectProfile(p: Profile) {
    const tabId = `ssh:${crypto.randomUUID()}`;
    const cred = credentials.find((c) => c.id === p.credential_id);
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
    const fp = profiles.find((p) => p.id === f.profile_id);
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
    <h1 class="logo">RSSH ㋡</h1>
    <input
      class="search-input"
      type="search"
      bind:value={query}
      aria-label={t("common.search")}
      placeholder={`${t("common.search")}...`}
    />
    <div class="home-controls">
      <div class="segmented" role="group" aria-label={t("home.display.aria")}>
        <button
          type="button"
          class:active={viewPreferences.grouping === "group"}
          aria-pressed={viewPreferences.grouping === "group"}
          onclick={() => setGrouping("group")}
        >{t("home.display.group")}</button>
        <button
          type="button"
          class:active={viewPreferences.grouping === "type"}
          aria-pressed={viewPreferences.grouping === "type"}
          onclick={() => setGrouping("type")}
        >{t("home.display.type")}</button>
      </div>
      <div class="segmented" role="group" aria-label={t("home.sort.aria")}>
        <button
          type="button"
          class:active={viewPreferences.sorting === "name"}
          aria-pressed={viewPreferences.sorting === "name"}
          onclick={() => setSorting("name")}
        >{t("home.sort.az")}</button>
        <button
          type="button"
          class:active={viewPreferences.sorting === "recent"}
          aria-pressed={viewPreferences.sorting === "recent"}
          onclick={() => setSorting("recent")}
        >{t("home.sort.recent")}</button>
      </div>
    </div>
  </div>

  {#if sections.length > 0}
    {#each sections as section, sIdx (section.key)}
      {@const headingId = `home-section-${sIdx}`}
      <section aria-labelledby={headingId}>
        <h2 id={headingId} class="section-label" style={section.color ? `border-left: 3px solid ${section.color}; padding-left: 8px` : ""}>
          {section.label}
        </h2>
        <div class="grid" aria-labelledby={headingId} bind:this={gridEls[sIdx]}>
          {#each section.items as it, i (it.id)}
            <div class="card-wrap">
              <button
                class="card-btn surface-raised"
                class:selected={navIdx === section.offset + i}
                onclick={() => activate(it)}
              >
                <div class="card-icon {it.iconClass}">{it.icon}</div>
                <div class="card-body">
                  <div class="card-name">{it.name}</div>
                  <div class="card-sub">{it.sub}</div>
                </div>
              </button>
              {#if it.pinProfileId}
                <button
                  class="pin-btn"
                  class:pinned={app.isProfilePinned(it.pinProfileId)}
                  title={app.isProfilePinned(it.pinProfileId) ? "Unpin" : "Pin to sidebar"}
                  onclick={(e) => { e.stopPropagation(); if (it.pinProfileId) toggleProfilePin(it.pinProfileId); }}
                >{app.isProfilePinned(it.pinProfileId) ? "★" : "☆"}</button>
              {/if}
            </div>
          {/each}
        </div>
      </section>
    {/each}
  {:else if allItems.length === 0}
    <div class="empty-state">
      <p>No Profiles or Port Forwards yet</p>
      <button class="btn btn-accent" onclick={() => app.navigate("settings")}>Go to Settings</button>
    </div>
  {/if}
</div>

<style>
  .home { padding: 24px; flex: 1; overflow-y: auto; min-height: 0; }
  .home-header { display: flex; align-items: center; flex-wrap: wrap; gap: 12px 16px; margin-bottom: 20px; }
  .logo { font-size: 22px; color: var(--accent); font-weight: 700; white-space: nowrap; }
  .search-input { flex: 1 1 200px; min-width: 160px; }
  .home-controls { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  .segmented {
    display: inline-flex;
    padding: 2px;
    border: 1px solid var(--divider);
    border-radius: var(--radius-sm);
    background: var(--surface);
  }
  .segmented button {
    padding: 5px 10px;
    border: 0;
    border-radius: calc(var(--radius-sm) - 2px);
    background: transparent;
    color: var(--text-sub);
    cursor: pointer;
    font-family: inherit;
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
  }
  .segmented button:hover:not(.active) { color: var(--text); }
  .segmented button.active {
    background: color-mix(in srgb, var(--accent) 12%, var(--bg));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 55%, transparent);
    color: var(--text);
  }
  .segmented button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  /* Override global section-label: move spacing from padding → margin so the
     group color border-left only covers the text, not the full padded area.
     The 12px bottom margin clears the card's neumorphism shadow (~10px). */
  .home :global(.section-label) {
    margin: 20px 0 12px;
    padding: 2px 0;
  }

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
    display: flex; align-items: flex-start; gap: calc(12px * var(--density));
    padding: calc(14px * var(--density)); text-align: left; width: 100%;
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
  .card-icon.fwd { background: color-mix(in srgb, var(--success) 15%, transparent); color: var(--success); }
  .card-icon.serial { background: color-mix(in srgb, var(--warning) 18%, transparent); color: var(--warning); }
  .card-icon.telnet { background: color-mix(in srgb, var(--purple) 15%, transparent); color: var(--purple); }
  .card-icon.docker { background: var(--accent-soft); color: var(--accent); }
  .card-icon.k8s { background: color-mix(in srgb, var(--purple) 15%, transparent); color: var(--purple); }

  .card-body { flex: 1; min-width: 0; }
  .card-name { font-weight: 600; font-size: 14px; color: var(--text); margin-bottom: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .card-sub { font-size: 12px; color: var(--text-sub); font-family: monospace; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .empty-state { text-align: center; padding: 60px 24px; color: var(--text-dim); }
  .empty-state p { margin-bottom: 12px; }

  @media (max-width: 620px) {
    .logo { width: 100%; }
    .search-input { flex-basis: 100%; }
    .home-controls { width: 100%; }
  }
</style>
