<!--
  Scene 3 — Security & Sync, modelled after the flow:

      RSSH ──┬──→ OS_KeyServer  (keys stay here, never travel)
             └──→ Profile_DB ──[encrypted]──→ GitHub repo ──┬──→ Windows
                                                            ├──→ macOS
                                                            ├──→ Linux
                                                            └──→ Android

  Implementation: 8 absolutely-positioned nodes in a 16:10 stage, an SVG
  layer with 7 connectors that draw in (stroke-dashoffset), and "doc"
  glyphs that ride the DB→repo→platforms path. The key icon is anchored
  inside the OS_KeyServer node and never animates out.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../i18n/index.svelte.ts";
  import NextButton from "./NextButton.svelte";

  let { onNext }: { onNext: () => void } = $props();

  // Cumulative phases. Each subsequent phase pulls in all previous
  // s-* class names so prior state stays painted.
  let phase = $state<
    "idle" | "nodes" | "wired" | "encrypting" | "stored" | "fanout" | "lit" | "captioned"
  >("idle");
  let ready = $state(false);

  let classes = $derived(() => {
    const order = ["nodes", "wired", "encrypting", "stored", "fanout", "lit", "captioned"];
    const idx = order.indexOf(phase);
    return order.slice(0, idx + 1).map(c => `s-${c}`).join(" ");
  });

  let timers: number[] = [];
  function at(ms: number, fn: () => void) { timers.push(window.setTimeout(fn, ms)); }

  onMount(() => {
    const reduced =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

    if (reduced) {
      phase = "captioned"; ready = true;
      return;
    }

    at(400,  () => phase = "nodes");        // all nodes fade in
    at(1500, () => phase = "wired");        // RSSH→KS, RSSH→DB
    at(2100, () => phase = "encrypting");   // c3 + profile doc flies DB→repo
    at(3300, () => phase = "stored");       // lock badge lights, doc lands
    at(3700, () => phase = "fanout");       // c4-c7 draw, 4 docs fly
    at(5100, () => phase = "lit");          // 4 platforms light up
    at(5500, () => phase = "captioned");
    at(6300, () => ready = true);

    return () => { timers.forEach(window.clearTimeout); };
  });
</script>

<section class="scene">
  <div class="chip">
    <span class="chip-dot"></span>
    {t("welcome.scene.sync.chip")}
  </div>

  <div class="stage {classes()}">
    <!-- Connector lines. viewBox=1000x600 + preserveAspectRatio=none so
         each unit maps to 0.1% of stage width / 0.166% of stage height.
         Endpoints land exactly on each node's edge:
           rssh    right edge = 16%  → x=160
           keysrv  left edge  = 22%  → x=220     (Y top edge 25% → y=150)
           db      left edge  = 22%  → x=220     (Y top edge 75% → y=450)
           db      right edge = 38%  → x=380
           repo    left edge  = 45%  → x=450
           repo    right edge = 61%  → x=610
           platform left edge = 85%  → x=850
           platform centres   y = (17/37/63/83)%·6 = 102/222/378/498 -->
    <svg class="lines" viewBox="0 0 1000 600" preserveAspectRatio="none" aria-hidden="true">
      <!-- RSSH → keyserver (branch up) -->
      <path class="connect c1" d="M 160 300 H 190 V 150 H 220" fill="none" />
      <!-- RSSH → DB (branch down) -->
      <path class="connect c2" d="M 160 300 H 190 V 450 H 220" fill="none" />
      <!-- DB → repo (encrypted upload) -->
      <path class="connect c3 encrypted" d="M 380 450 H 415 V 300 H 450" fill="none" />
      <!-- repo → 4 platforms (fanout). All share the same x=730 corner. -->
      <path class="connect c4" d="M 610 300 H 730 V 102 H 850" fill="none" />
      <path class="connect c5" d="M 610 300 H 730 V 222 H 850" fill="none" />
      <path class="connect c6" d="M 610 300 H 730 V 378 H 850" fill="none" />
      <path class="connect c7" d="M 610 300 H 730 V 498 H 850" fill="none" />

      <!-- "ENCRYPTED" label hovers next to c3's vertical segment (x=415, y=300-450). -->
      <text class="enc-label" x="425" y="378">{t("welcome.scene.sync.label_encrypted")}</text>
    </svg>

    <!-- RSSH (the orchestrator) -->
    <div class="node rssh">
      <div class="node-icon rssh-icon">
        <span class="rssh-glyph">&gt;_</span>
      </div>
      <div class="node-label">{t("welcome.scene.sync.node_rssh")}</div>
      <div class="node-sub">{t("welcome.scene.sync.node_rssh_sub")}</div>
    </div>

    <!-- OS Key Server — terminal node for the secrets branch. Key never leaves. -->
    <div class="node keyserver">
      <div class="node-icon">
        <!-- Stylised lock-on-disc — generic enough to read as "OS key vault" -->
        <svg viewBox="0 0 36 30" width="42" height="36" aria-hidden="true">
          <rect x="1" y="1" width="34" height="28" rx="4" fill="#2a2c36" stroke="currentColor" stroke-width="1.4"/>
          <path d="M14 17 V14 a4 4 0 0 1 8 0 V17" fill="none" stroke="currentColor" stroke-width="1.6"/>
          <rect x="11" y="17" width="14" height="8" rx="1.5" fill="currentColor"/>
        </svg>
        <!-- Key chip stays anchored to the keystore — it never travels. -->
        <span class="key-stay" aria-hidden="true">
          <svg viewBox="0 0 22 10" width="22" height="10">
            <circle cx="4" cy="5" r="3" fill="none" stroke="#fde68a" stroke-width="1.6"/>
            <rect x="7" y="4.2" width="11" height="1.6" fill="#fde68a"/>
            <rect x="14" y="5.8" width="1.6" height="2" fill="#fde68a"/>
            <rect x="17" y="5.8" width="1.6" height="2.5" fill="#fde68a"/>
          </svg>
        </span>
      </div>
      <div class="node-label">{t("welcome.scene.sync.node_keyserver")}</div>
      <div class="node-sub">{t("welcome.scene.sync.node_keyserver_sub")}</div>
    </div>

    <!-- Profile DB — local store for profile metadata (no secrets). -->
    <div class="node db">
      <div class="node-icon">
        <svg viewBox="0 0 30 30" width="36" height="36" aria-hidden="true">
          <ellipse cx="15" cy="6" rx="12" ry="3.5" fill="#2a2c36" stroke="currentColor" stroke-width="1.4"/>
          <path d="M3 6 V14 a12 3.5 0 0 0 24 0 V6" fill="#2a2c36" stroke="currentColor" stroke-width="1.4"/>
          <path d="M3 14 V22 a12 3.5 0 0 0 24 0 V14" fill="#2a2c36" stroke="currentColor" stroke-width="1.4"/>
        </svg>
      </div>
      <div class="node-label">{t("welcome.scene.sync.node_db")}</div>
      <div class="node-sub">{t("welcome.scene.sync.node_db_sub")}</div>
    </div>

    <!-- GitHub repo — encrypted profile backup. Lock badge lights up on arrival. -->
    <div class="node repo">
      <div class="node-icon repo-icon">
        <svg viewBox="0 0 48 30" width="54" height="34" aria-hidden="true">
          <path
            d="M11 26 C 4 26  2 19  8 16
               C 7 7  17 4  22 11
               C 26 5  37 7  37 15
               C 44 15  44 26  37 26 Z"
            fill="#2a2c36" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"
          />
        </svg>
        <span class="doc-stored" aria-hidden="true">
          <svg viewBox="0 0 14 16" width="14" height="16">
            <path d="M2 1 H10 L13 4 V15 H2 Z" fill="#1c1d24" stroke="#7dd3fc" stroke-width="1.2" stroke-linejoin="round"/>
            <line x1="4" y1="6" x2="11" y2="6" stroke="#7dd3fc" stroke-width="1.2"/>
            <line x1="4" y1="9" x2="11" y2="9" stroke="#7dd3fc" stroke-width="1.2"/>
            <line x1="4" y1="12" x2="9" y2="12" stroke="#7dd3fc" stroke-width="1.2"/>
          </svg>
        </span>
        <div class="lock-badge">
          <svg viewBox="0 0 14 18" width="18" height="22" aria-hidden="true">
            <path d="M3 8 V6 a4 4 0 0 1 8 0 V8" fill="none" stroke="currentColor" stroke-width="1.6"/>
            <rect x="1.5" y="8" width="11" height="8.5" rx="1.5" fill="currentColor"/>
            <circle cx="7" cy="12" r="1" fill="#1c1d24"/>
          </svg>
        </div>
      </div>
      <div class="node-label">{t("welcome.scene.sync.node_repo")}</div>
      <div class="node-sub">{t("welcome.scene.sync.node_repo_sub")}</div>
    </div>

    <!-- 4 platform endpoints. Simple monogram tiles — colored by family. -->
    <div class="node platform p-win"  style="--pf: #4a8bf7;">
      <div class="pf-tile">W</div>
      <div class="node-label-sm">{t("welcome.scene.sync.platform_windows")}</div>
    </div>
    <div class="node platform p-mac"  style="--pf: #b0b7c4;">
      <div class="pf-tile">M</div>
      <div class="node-label-sm">{t("welcome.scene.sync.platform_macos")}</div>
    </div>
    <div class="node platform p-lin"  style="--pf: #f3a142;">
      <div class="pf-tile">L</div>
      <div class="node-label-sm">{t("welcome.scene.sync.platform_linux")}</div>
    </div>
    <div class="node platform p-and"  style="--pf: #4cb88a;">
      <div class="pf-tile">A</div>
      <div class="node-label-sm">{t("welcome.scene.sync.platform_android")}</div>
    </div>

    <!-- Profile doc that travels DB → repo. -->
    <div class="doc d-encrypt" aria-hidden="true">
      <svg viewBox="0 0 14 16" width="16" height="18">
        <path d="M2 1 H10 L13 4 V15 H2 Z" fill="#1c1d24" stroke="#7dd3fc" stroke-width="1.2" stroke-linejoin="round"/>
        <line x1="4" y1="6" x2="11" y2="6" stroke="#7dd3fc" stroke-width="1.2"/>
        <line x1="4" y1="9" x2="11" y2="9" stroke="#7dd3fc" stroke-width="1.2"/>
        <line x1="4" y1="12" x2="9" y2="12" stroke="#7dd3fc" stroke-width="1.2"/>
      </svg>
    </div>

    <!-- 4 docs that fan out repo → platforms. -->
    {#each ["d-out-1", "d-out-2", "d-out-3", "d-out-4"] as cls (cls)}
      <div class="doc {cls}" aria-hidden="true">
        <svg viewBox="0 0 14 16" width="14" height="16">
          <path d="M2 1 H10 L13 4 V15 H2 Z" fill="#1c1d24" stroke="#7dd3fc" stroke-width="1.2" stroke-linejoin="round"/>
          <line x1="4" y1="6" x2="11" y2="6" stroke="#7dd3fc" stroke-width="1.2"/>
          <line x1="4" y1="9" x2="11" y2="9" stroke="#7dd3fc" stroke-width="1.2"/>
          <line x1="4" y1="12" x2="9" y2="12" stroke="#7dd3fc" stroke-width="1.2"/>
        </svg>
      </div>
    {/each}
  </div>

  <div class="caption" class:show={phase === "captioned"}>
    <span class="kw">{t("welcome.scene.sync.caption_kw1")}</span>
    {t("welcome.scene.sync.caption_join")}
    <span class="kw">{t("welcome.scene.sync.caption_kw2")}</span>
    {t("welcome.scene.sync.caption_join")}
    <span class="kw">{t("welcome.scene.sync.caption_kw3")}</span>
  </div>

  <NextButton {ready} onClick={onNext} />
</section>

<style>
  .scene {
    position: relative;
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: clamp(18px, 3vh, 32px);
    padding: clamp(20px, 4vh, 48px) clamp(16px, 3vw, 32px);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border: 1px solid color-mix(in srgb, var(--warning) 55%, transparent);
    border-radius: 999px;
    background: color-mix(in srgb, var(--warning) 10%, transparent);
    color: var(--warning);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 1.4px;
    text-transform: uppercase;
    opacity: 0;
    animation: chip-in 500ms 200ms forwards;
  }
  @keyframes chip-in { to { opacity: 1; } }
  .chip-dot {
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--warning);
    box-shadow: 0 0 8px var(--warning);
    animation: chip-pulse 1.6s ease-in-out infinite;
  }
  @keyframes chip-pulse { 50% { opacity: 0.45; } }

  .stage {
    position: relative;
    width: min(86vw, 1100px);
    aspect-ratio: 16 / 10;
    max-height: 64vh;
    opacity: 0;
    transform: translateY(14px);
    animation: stage-in 600ms 100ms cubic-bezier(0.22, 1, 0.36, 1) forwards;
  }
  @keyframes stage-in { to { opacity: 1; transform: translateY(0); } }

  .lines {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    z-index: 1;
    pointer-events: none;
    overflow: visible;
  }
  .connect {
    stroke: #3c3f50;
    stroke-width: 2;
    stroke-dasharray: 6 4;
    stroke-dashoffset: 800;
    transition: stroke-dashoffset 700ms ease, stroke 400ms ease;
  }
  /* c1 + c2: RSSH→KS, RSSH→DB. Drawn at "wired" phase. */
  .s-wired .c1, .s-wired .c2 { stroke-dashoffset: 0; stroke: var(--accent); }
  /* c3: DB→repo, encrypted. Golden hue. Drawn at "encrypting" phase. */
  .s-encrypting .c3 { stroke-dashoffset: 0; stroke: var(--warning); }
  /* c4-c7: repo→platforms. Drawn at "fanout" phase, staggered. */
  .s-fanout .c4 { stroke-dashoffset: 0; stroke: var(--success); }
  .s-fanout .c5 { stroke-dashoffset: 0; stroke: var(--success); transition-delay: 60ms; }
  .s-fanout .c6 { stroke-dashoffset: 0; stroke: var(--success); transition-delay: 120ms; }
  .s-fanout .c7 { stroke-dashoffset: 0; stroke: var(--success); transition-delay: 180ms; }

  .enc-label {
    fill: var(--warning);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 1px;
    text-anchor: middle;
    opacity: 0;
    transition: opacity 400ms 200ms ease;
  }
  .s-encrypting .enc-label, .s-stored .enc-label,
  .s-fanout .enc-label, .s-lit .enc-label, .s-captioned .enc-label {
    opacity: 1;
  }

  /* ─── Generic node card ─── */
  .node {
    position: absolute;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 12px 14px;
    background: #1c1d24;
    border: 1px solid rgba(255, 255, 255, 0.07);
    border-radius: 12px;
    color: var(--text);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    opacity: 0;
    z-index: 2;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.4);
    transition: opacity 400ms ease, box-shadow 400ms ease, transform 400ms ease;
    text-align: center;
  }
  .s-nodes .node { opacity: 1; }

  /* Absolute positions in % of the stage. The SVG connector endpoints
     were chosen to land cleanly on each node's edge. */
  .rssh {
    left: 4%;
    top: 50%;
    transform: translate(0, -50%);
    color: var(--accent);
    width: 12%;
    min-width: 110px;
    box-shadow:
      0 12px 32px rgba(0, 0, 0, 0.4),
      0 0 0 1px color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .rssh-icon { color: var(--accent); }
  .rssh-glyph {
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.04em;
    color: var(--accent);
    text-shadow:
      0 0 8px color-mix(in srgb, var(--accent) 60%, transparent),
      0 0 18px color-mix(in srgb, var(--accent) 30%, transparent);
  }

  .keyserver {
    left: 22%;
    top: 25%;
    transform: translate(0, -50%);
    color: var(--warning);
    width: 16%;
    min-width: 140px;
    /* Yellow halo persists — emphasises "secrets live here". */
    box-shadow:
      0 12px 32px rgba(0, 0, 0, 0.4),
      0 0 0 1px color-mix(in srgb, var(--warning) 35%, transparent),
      0 0 30px color-mix(in srgb, var(--warning) 15%, transparent);
  }
  /* Key chip — anchored bottom-right of the device icon. */
  .key-stay {
    position: absolute;
    right: -8px;
    bottom: -4px;
    background: rgba(28, 30, 42, 0.95);
    border: 1px solid color-mix(in srgb, var(--warning) 55%, transparent);
    border-radius: 6px;
    padding: 2px 4px;
    display: inline-flex;
    align-items: center;
    filter: drop-shadow(0 0 6px color-mix(in srgb, var(--warning) 60%, transparent));
  }

  .db {
    left: 22%;
    top: 75%;
    transform: translate(0, -50%);
    color: var(--accent);
    width: 16%;
    min-width: 140px;
  }

  .repo {
    left: 45%;
    top: 50%;
    transform: translate(0, -50%);
    color: var(--text-sub);
    width: 16%;
    min-width: 140px;
  }
  .repo-icon { position: relative; }
  .s-stored .repo, .s-fanout .repo, .s-lit .repo, .s-captioned .repo {
    color: var(--success);
    box-shadow:
      0 12px 32px rgba(0, 0, 0, 0.4),
      0 0 0 1px color-mix(in srgb, var(--success) 35%, transparent),
      0 0 30px color-mix(in srgb, var(--success) 18%, transparent);
  }
  .doc-stored {
    position: absolute;
    left: -4px;
    bottom: -2px;
    background: #1c1d24;
    border-radius: 4px;
    padding: 1px 2px;
    opacity: 0;
    transform: scale(0.6);
    transition: opacity 320ms ease, transform 320ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .s-stored .doc-stored, .s-fanout .doc-stored, .s-lit .doc-stored, .s-captioned .doc-stored {
    opacity: 1;
    transform: scale(1);
  }
  .lock-badge {
    position: absolute;
    right: -10px;
    top: -8px;
    color: var(--success);
    opacity: 0;
    transform: scale(0.4);
    transition: opacity 300ms ease, transform 320ms cubic-bezier(0.22, 1, 0.36, 1);
    filter: drop-shadow(0 0 6px color-mix(in srgb, var(--success) 70%, transparent));
  }
  .s-stored .lock-badge, .s-fanout .lock-badge, .s-lit .lock-badge, .s-captioned .lock-badge {
    opacity: 1;
    transform: scale(1);
  }

  /* Platform endpoints (right column). Each gets a colored monogram. */
  .platform {
    right: 4%;
    width: 11%;
    min-width: 96px;
    padding: 6px 10px;
    flex-direction: row;
    align-items: center;
    justify-content: flex-start;
    gap: 10px;
    color: var(--pf);
    opacity: 0;
  }
  .s-nodes .platform { opacity: 0.55; }
  .s-lit .platform, .s-captioned .platform {
    opacity: 1;
    box-shadow:
      0 12px 32px rgba(0, 0, 0, 0.4),
      0 0 0 1px color-mix(in srgb, var(--pf) 35%, transparent);
  }
  .pf-tile {
    width: 28px;
    height: 28px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--pf) 18%, transparent);
    border: 1px solid color-mix(in srgb, var(--pf) 50%, transparent);
    color: var(--pf);
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: 14px;
    font-weight: 700;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .node-label-sm {
    font-size: 11px;
    color: var(--text);
    letter-spacing: 0.3px;
  }

  .p-win { top: 17%; transform: translate(0, -50%); }
  .p-mac { top: 37%; transform: translate(0, -50%); }
  .p-lin { top: 63%; transform: translate(0, -50%); }
  .p-and { top: 83%; transform: translate(0, -50%); }

  .node-icon {
    color: inherit;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    position: relative;
  }
  .rssh-icon {
    width: 44px;
    height: 44px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-radius: 8px;
  }
  .node-label {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.6px;
    color: var(--text);
    text-transform: uppercase;
  }
  .node-sub {
    font-size: 10px;
    color: var(--text-dim);
    letter-spacing: 0.3px;
    line-height: 1.4;
  }

  /* ─── Flying profile docs ─── */
  .doc {
    position: absolute;
    z-index: 3;
    opacity: 0;
    pointer-events: none;
    filter: drop-shadow(0 0 6px color-mix(in srgb, #7dd3fc 60%, transparent));
    will-change: left, top, opacity;
  }

  /* DB(at 22%, 75%) → repo(45%, 50%). Slight arc. */
  .doc.d-encrypt {
    left: 30%;
    top: 75%;
    transform: translate(-50%, -50%);
  }
  .s-encrypting .d-encrypt {
    animation: doc-encrypt 1300ms cubic-bezier(0.5, -0.1, 0.5, 1) forwards;
  }
  @keyframes doc-encrypt {
    0%   { opacity: 0; left: 30%; top: 75%; }
    12%  { opacity: 1; }
    50%  { left: 40%; top: 62%; }
    90%  { opacity: 1; }
    100% { opacity: 0; left: 53%; top: 50%; }
  }

  /* repo(53%, 50%) → 4 platforms (right side, varied y). Stagger 120ms. */
  .doc.d-out-1, .doc.d-out-2, .doc.d-out-3, .doc.d-out-4 {
    left: 53%;
    top: 50%;
    transform: translate(-50%, -50%);
  }
  .s-fanout .d-out-1 { animation: doc-out-1 1100ms cubic-bezier(0.5, 0, 0.5, 1) 0ms forwards; }
  .s-fanout .d-out-2 { animation: doc-out-2 1100ms cubic-bezier(0.5, 0, 0.5, 1) 120ms forwards; }
  .s-fanout .d-out-3 { animation: doc-out-3 1100ms cubic-bezier(0.5, 0, 0.5, 1) 240ms forwards; }
  .s-fanout .d-out-4 { animation: doc-out-4 1100ms cubic-bezier(0.5, 0, 0.5, 1) 360ms forwards; }
  @keyframes doc-out-1 {
    0%   { opacity: 0; left: 53%; top: 50%; }
    10%  { opacity: 1; }
    100% { opacity: 0; left: 85%; top: 17%; }
  }
  @keyframes doc-out-2 {
    0%   { opacity: 0; left: 53%; top: 50%; }
    10%  { opacity: 1; }
    100% { opacity: 0; left: 85%; top: 37%; }
  }
  @keyframes doc-out-3 {
    0%   { opacity: 0; left: 53%; top: 50%; }
    10%  { opacity: 1; }
    100% { opacity: 0; left: 85%; top: 63%; }
  }
  @keyframes doc-out-4 {
    0%   { opacity: 0; left: 53%; top: 50%; }
    10%  { opacity: 1; }
    100% { opacity: 0; left: 85%; top: 83%; }
  }

  .caption {
    font-family: "SF Mono", Menlo, Consolas, monospace;
    font-size: clamp(12px, 1.3vw, 14px);
    color: var(--text-sub);
    letter-spacing: 0.6px;
    text-align: center;
    opacity: 0;
    transform: translateY(6px);
    transition: opacity 400ms ease, transform 400ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .caption.show { opacity: 1; transform: translateY(0); }
  .caption .kw { color: var(--warning); font-weight: 700; }

  @media (prefers-reduced-motion: reduce) {
    .chip, .stage, .node, .connect, .caption,
    .lock-badge, .doc-stored, .key-stay, .enc-label {
      animation: none !important;
      transition: none !important;
      opacity: 1 !important;
      transform: none !important;
      stroke-dashoffset: 0 !important;
    }
    .doc { display: none !important; }
  }
</style>
