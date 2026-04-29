<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getVersion } from "@tauri-apps/api/app";
  import { t } from "../i18n/index.svelte.ts";

  const REPO = "shihuili1218/rssh";
  const REPO_URL = `https://github.com/${REPO}`;
  const ISSUES_URL = `${REPO_URL}/issues`;
  const LICENSE_URL = `${REPO_URL}/blob/main/LICENSE`;
  const RELEASES_PAGE = `${REPO_URL}/releases/latest`;

  type UpdateStatus =
    | { kind: "idle" }
    | { kind: "checking" }
    | { kind: "latest" }
    | { kind: "outdated"; latest: string }
    | { kind: "error" };

  let version = $state("—");
  let justCopied = $state(false);
  let update = $state<UpdateStatus>({ kind: "idle" });

  onMount(async () => {
    try {
      version = await getVersion();
    } catch (e) {
      console.error("getVersion failed:", e);
    }
  });

  function parseVersion(v: string): number[] {
    return v.replace(/^v/i, "").split(/[.\-+]/).map(s => parseInt(s, 10) || 0);
  }

  function compareVersion(a: string, b: string): number {
    const aa = parseVersion(a);
    const bb = parseVersion(b);
    const len = Math.max(aa.length, bb.length);
    for (let i = 0; i < len; i++) {
      const x = aa[i] ?? 0;
      const y = bb[i] ?? 0;
      if (x !== y) return x > y ? 1 : -1;
    }
    return 0;
  }

  async function checkUpdate() {
    if (update.kind === "checking") return;
    update = { kind: "checking" };
    try {
      const tag = await invoke<string>("fetch_latest_release_tag", { repo: REPO });
      if (!tag) throw new Error("empty tag");
      update = compareVersion(tag, version) > 0
        ? { kind: "outdated", latest: tag.replace(/^v/i, "") }
        : { kind: "latest" };
    } catch (e) {
      console.error("checkUpdate failed:", e);
      update = { kind: "error" };
    }
  }

  function openUrl(url: string) {
    invoke("open_external_url", { url }).catch(e =>
      console.error("open_external_url failed:", e)
    );
  }

  async function copyDiagnostics() {
    const diag = `RSSH v${version}\n${navigator.userAgent}`;
    try {
      await navigator.clipboard.writeText(diag);
      justCopied = true;
      setTimeout(() => { justCopied = false; }, 1500);
    } catch (e) {
      console.error("clipboard write failed:", e);
    }
  }
</script>

<div class="page">
  <div class="hero">
    <div class="app-name">RSSH</div>
    <div class="app-version">v{version}</div>
  </div>

  <div class="update">
    {#if update.kind === "outdated"}
      <button class="update-btn primary" onclick={() => openUrl(RELEASES_PAGE)}>
        {t("about.update.download")} v{update.latest}
      </button>
    {:else}
      <button class="update-btn"
              disabled={update.kind === "checking"}
              onclick={checkUpdate}>
        {update.kind === "checking" ? t("about.update.checking") : t("about.update.check")}
      </button>
    {/if}
    {#if update.kind === "latest"}
      <span class="update-hint ok">{t("about.update.latest")}</span>
    {:else if update.kind === "error"}
      <span class="update-hint err">{t("about.update.error")}</span>
    {/if}
  </div>

  <div class="links">
    <button class="link-row" onclick={() => openUrl(REPO_URL)}>
      <span class="link-label">{t("about.repo")}</span>
      <span class="link-url">{REPO_URL}</span>
    </button>
    <button class="link-row" onclick={() => openUrl(ISSUES_URL)}>
      <span class="link-label">{t("about.issues")}</span>
      <span class="link-url">{ISSUES_URL}</span>
    </button>
    <button class="link-row" onclick={() => openUrl(LICENSE_URL)}>
      <span class="link-label">{t("about.license")}</span>
      <span class="link-url">MIT</span>
    </button>
  </div>

  <div class="diag">
    <button class="diag-btn" class:copied={justCopied} onclick={copyDiagnostics}>
      {justCopied ? t("about.copied") : t("about.diagnostics")}
    </button>
    <span class="diag-hint">{t("about.diagnostics.hint")}</span>
  </div>
</div>

<style>
  .page {
    padding: 32px 24px;
    display: flex;
    flex-direction: column;
    gap: 28px;
  }
  .hero {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
  }
  .app-name {
    font-size: 28px;
    font-weight: 700;
    color: var(--text);
    letter-spacing: 0.5px;
  }
  .app-version {
    font-size: 13px;
    color: var(--text-dim);
    font-family: monospace;
  }
  .links {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .link-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 16px;
    padding: 12px 16px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--bg);
    box-shadow: var(--raised-sm);
    color: var(--text-sub);
    font-family: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    transition: box-shadow 0.15s, color 0.15s;
  }
  .link-row:hover { color: var(--text); box-shadow: var(--raised); }
  .link-row:active { box-shadow: var(--pressed); }
  .link-label { font-weight: 600; }
  .link-url {
    font-family: monospace;
    font-size: 12px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .diag {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .diag-btn {
    padding: 8px 16px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--bg);
    box-shadow: var(--raised-sm);
    color: var(--text-sub);
    font-family: inherit;
    font-size: 13px;
    cursor: pointer;
    transition: box-shadow 0.15s, color 0.15s;
  }
  .diag-btn:hover { color: var(--text); box-shadow: var(--raised); }
  .diag-btn:active { box-shadow: var(--pressed); }
  .diag-btn.copied { color: var(--accent); }
  .diag-hint { font-size: 12px; color: var(--text-dim); }
  .update {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .update-btn {
    padding: 8px 16px;
    border: none;
    border-radius: var(--radius-sm);
    background: var(--bg);
    box-shadow: var(--raised-sm);
    color: var(--text-sub);
    font-family: inherit;
    font-size: 13px;
    cursor: pointer;
    transition: box-shadow 0.15s, color 0.15s;
  }
  .update-btn:hover { color: var(--text); box-shadow: var(--raised); }
  .update-btn:active { box-shadow: var(--pressed); }
  .update-btn:disabled { cursor: default; opacity: 0.6; }
  .update-btn.primary { color: var(--accent); }
  .update-hint { font-size: 12px; }
  .update-hint.ok { color: var(--text-dim); }
  .update-hint.err { color: var(--danger, #c0392b); }
</style>
