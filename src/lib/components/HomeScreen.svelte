<script lang="ts">
  import { onMount } from "svelte";
  import * as app from "../stores/app.svelte.ts";
  import type { Profile, Credential, Forward, Group, SerialProfile } from "../stores/app.svelte.ts";

  let profiles = $state<Profile[]>([]);
  let credentials = $state<Credential[]>([]);
  let forwards = $state<Forward[]>([]);
  let groups = $state<Group[]>([]);
  let serialProfiles = $state<SerialProfile[]>([]);
  let query = $state("");

  // One global nav index into the flat (display-order) item list.
  let navIdx = $state(-1);
  // One grid element per section; all share the same CSS so any one gives the
  // column count. Plain array (not $state): only read inside the key handler.
  let gridEls: HTMLDivElement[] = [];

  // ── Normalize the three config kinds into one shape ──────────────────────
  // SSH profiles, port forwards and serial profiles all become a HomeItem, so a
  // single grouping / filtering / keyboard-nav path covers them — no per-kind
  // sections, no special cases. `kind` + the icon are all that set them apart.
  interface HomeItem {
    kind: "ssh" | "forward" | "serial";
    id: string; // `${kind}:${rawId}` — globally unique for keyed #each + nav
    rawId: string;
    name: string;
    sub: string;
    icon: string;
    iconClass: string; // "" | "fwd" | "serial"
    group_id: string | null;
    data: Profile | Forward | SerialProfile;
  }

  let allItems = $derived<HomeItem[]>([
    ...profiles.map((p): HomeItem => {
      const cred = credentials.find((c) => c.id === p.credential_id);
      return {
        kind: "ssh", id: `ssh:${p.id}`, rawId: p.id, name: p.name,
        sub: `${cred?.username ?? "?"}@${p.host}:${p.port}`,
        icon: "S", iconClass: "", group_id: p.group_id ?? null, data: p,
      };
    }),
    ...forwards.map((f): HomeItem => ({
      kind: "forward", id: `forward:${f.id}`, rawId: f.id, name: f.name,
      sub: `:${f.local_port} → ${f.remote_host}:${f.remote_port}`,
      icon: f.type === "dynamic" ? "D" : f.type === "local" ? "L" : "R",
      iconClass: "fwd", group_id: f.group_id ?? null, data: f,
    })),
    ...serialProfiles.map((s): HomeItem => ({
      kind: "serial", id: `serial:${s.id}`, rawId: s.id, name: s.name,
      sub: `${s.port} · ${s.baud_rate}`,
      icon: "⎓", iconClass: "serial", group_id: s.group_id ?? null, data: s,
    })),
  ]);

  // Search filters all three kinds at once (#4): match the name or the sub-line
  // (which carries host / ports / baud), case-insensitive.
  let filtered = $derived(
    query
      ? allItems.filter((it) => {
          const q = query.toLowerCase();
          return it.name.toLowerCase().includes(q) || it.sub.toLowerCase().includes(q);
        })
      : allItems
  );

  // Group into sections by group_id; groups sorted by sort_order, ungrouped
  // last. Empty groups never appear. `offset` makes an item's global nav index
  // `offset + i`, keeping the flat list and the rendered sections aligned.
  let groupedItems = $derived((() => {
    const map = new Map<string | null, HomeItem[]>();
    for (const it of filtered) {
      const gid = it.group_id ?? null;
      if (!map.has(gid)) map.set(gid, []);
      map.get(gid)!.push(it);
    }
    const sections: { group: Group | null; items: HomeItem[]; offset: number }[] = [];
    for (const g of [...groups].sort((a, b) => a.sort_order - b.sort_order)) {
      const items = map.get(g.id);
      if (items && items.length > 0) { sections.push({ group: g, items, offset: 0 }); map.delete(g.id); }
    }
    // Ungrouped (null group_id or unknown group_id) → one trailing section.
    const ungrouped: HomeItem[] = [];
    for (const [, items] of map) ungrouped.push(...items);
    if (ungrouped.length > 0) sections.push({ group: null, items: ungrouped, offset: 0 });
    let off = 0;
    for (const s of sections) { s.offset = off; off += s.items.length; }
    return sections;
  })());

  // Flat list in display order — what arrow-key nav walks.
  let navItems = $derived(groupedItems.flatMap((s) => s.items));

  function getCols(): number {
    const el = gridEls.find(Boolean);
    if (!el) return 3;
    return getComputedStyle(el).gridTemplateColumns.split(" ").length;
  }

  // Locate which section a global navIdx falls into.
  function findSection(idx: number) {
    for (let sIdx = 0; sIdx < groupedItems.length; sIdx++) {
      const s = groupedItems[sIdx];
      if (idx < s.offset + s.items.length) return { s, sIdx, i: idx - s.offset };
    }
    return null;
  }

  function handleHomeKey(e: KeyboardEvent) {
    if (app.activeTabId() !== "home" || app.settingsActive()) return;
    if (document.activeElement?.tagName === "INPUT") return;
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
      } else if (cur.sIdx + 1 < groupedItems.length) {
        const next = groupedItems[cur.sIdx + 1]; // next section, same column
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
        const prev = groupedItems[cur.sIdx - 1]; // prev section, last row, same column
        const lastRowStart = Math.floor((prev.items.length - 1) / cols) * cols;
        navIdx = prev.offset + Math.min(lastRowStart + col, prev.items.length - 1);
      }
    } else if (e.key === "Enter" && navIdx >= 0 && navIdx < total) {
      e.preventDefault();
      activate(navItems[navIdx]);
    }
  }

  onMount(refresh);

  $effect(() => {
    if (app.activeTabId() === "home" && !app.settingsActive()) refresh();
  });

  // Clear the selection whenever the filter changes — otherwise the highlight
  // would point at whatever item now sits at the stale navIdx in the reordered
  // (and possibly shorter) list. First arrow press re-enters at top-left.
  $effect(() => {
    void query;
    navIdx = -1;
  });

  async function refresh() {
    [profiles, credentials, forwards, groups, serialProfiles] = await Promise.all([
      app.loadProfiles(), app.loadCredentials(), app.loadForwards(), app.loadGroups(), app.loadSerialProfiles(),
    ]);
  }

  function activate(it: HomeItem) {
    if (it.kind === "ssh") connectProfile(it.data as Profile);
    else if (it.kind === "forward") openForward(it.data as Forward);
    else app.connectSerialProfile(it.data as SerialProfile);
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
    <input class="search-input" type="text" bind:value={query} placeholder="Search..." />
  </div>

  {#if groupedItems.length > 0}
    {#each groupedItems as section, sIdx (section.group?.id ?? "__ungrouped__")}
      <div class="section-label" style={section.group ? `border-left: 3px solid ${section.group.color}; padding-left: 8px` : ""}>
        {section.group?.name ?? "UNGROUPED"}
      </div>
      <div class="grid" bind:this={gridEls[sIdx]}>
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
            {#if it.kind === "ssh"}
              <button
                class="pin-btn"
                class:pinned={app.isProfilePinned(it.rawId)}
                title={app.isProfilePinned(it.rawId) ? "Unpin" : "Pin to sidebar"}
                onclick={(e) => { e.stopPropagation(); app.isProfilePinned(it.rawId) ? app.unpinProfile(it.rawId) : app.pinProfile(it.rawId); }}
              >{app.isProfilePinned(it.rawId) ? "★" : "☆"}</button>
            {/if}
          </div>
        {/each}
      </div>
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
  .home-header { display: flex; align-items: center; gap: 16px; margin-bottom: 20px; }
  .logo { font-size: 22px; color: var(--accent); font-weight: 700; white-space: nowrap; }
  .search-input { flex: 1; }

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

  .card-body { flex: 1; min-width: 0; }
  .card-name { font-weight: 600; font-size: 14px; color: var(--text); margin-bottom: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .card-sub { font-size: 12px; color: var(--text-sub); font-family: monospace; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .empty-state { text-align: center; padding: 60px 24px; color: var(--text-dim); }
  .empty-state p { margin-bottom: 12px; }
</style>
