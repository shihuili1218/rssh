<script lang="ts">
    import {onMount, onDestroy} from "svelte";
    import {invoke} from "@tauri-apps/api/core";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import type { MessageKey } from "../i18n/locales/en";

    /* ── GitHub source state ─────────────────────────────────────────────── */
    let githubEnabled = $state(false);
    let githubToken = $state("");
    let githubRepo = $state("");
    let githubBranch = $state("main");
    let githubSyncing = $state(false);
    let githubMsg = $state("");
    let githubSaveMsg = $state("");
    let githubSaveIsError = $state(false);
    let ghSaveTimer: ReturnType<typeof setTimeout> | null = null;

    /* ── WebDAV source state ─────────────────────────────────────────────── */
    let webdavEnabled = $state(false);
    let webdavUrl = $state("");
    let webdavUsername = $state("");
    let webdavPassword = $state("");
    let webdavSyncing = $state(false);
    let webdavMsg = $state("");
    let webdavSaveMsg = $state("");
    let webdavSaveIsError = $state(false);
    let wdSaveTimer: ReturnType<typeof setTimeout> | null = null;

    /* ── Shared sync content toggles ──────────────────────────────────────── */
    const SYNC_ITEMS: { key: string; label: MessageKey }[] = [
        { key: "sync_include_highlights", label: "sync.categories.highlights" },
        { key: "sync_include_snippets", label: "sync.categories.snippets" },
        { key: "sync_include_skills", label: "sync.categories.skills" },
        { key: "sync_include_ai_redact", label: "sync.categories.ai_redact" },
        { key: "sync_include_ai_blacklist", label: "sync.categories.ai_blacklist" },
        { key: "sync_include_ai", label: "sync.categories.ai" },
    ];
    let flags = $state<Record<string, boolean>>({});
    let groups = $state<{ id: string; name: string; color: string }[]>([]);
    let selectedGroups = $state<string[]>([]);
    // Chips = real groups + a synthetic "Ungrouped" (id ""). Selecting it syncs
    // rows with no group; the backend treats "" as the ungrouped key.
    let chipGroups = $derived([
        ...groups,
        { id: "", name: t("profile.ungrouped"), color: "" },
    ]);

    /* ── Password dialog state ───────────────────────────────────────────── */
    let showPwDialog = $state(false);
    let pwMode = $state<"push" | "pull">("push");
    let pw1 = $state("");
    let pw2 = $state("");
    let pwError = $state("");
    let globalMsg = $state("");

    let anySyncing = $derived(githubSyncing || webdavSyncing);

    onMount(async () => {
        /* GitHub */
        githubToken = await invoke<string | null>("get_setting", { key: "github_token" }) ?? "";
        githubRepo = await invoke<string | null>("get_setting", { key: "github_repo" }) ?? "";
        githubBranch = await invoke<string | null>("get_setting", { key: "github_branch" }) ?? "main";
        const ghEnabled = await invoke<string | null>("get_setting", { key: "sync_github_enabled" });
        githubEnabled = ghEnabled !== "0";

        /* WebDAV */
        webdavUrl = await invoke<string | null>("get_setting", { key: "webdav_url" }) ?? "";
        webdavUsername = await invoke<string | null>("get_setting", { key: "webdav_username" }) ?? "";
        webdavPassword = await invoke<string | null>("get_setting", { key: "webdav_password" }) ?? "";
        const wdEnabled = await invoke<string | null>("get_setting", { key: "sync_webdav_enabled" });
        webdavEnabled = wdEnabled === "1";

        /* Sync content toggles + group filter */
        for (const it of SYNC_ITEMS) {
            const v = await invoke<string | null>("get_setting", { key: it.key });
            flags[it.key] = v === null || v !== "0";
        }
        groups = await invoke<{ id: string; name: string; color: string }[]>("list_groups").catch(() => []);
        const gjson = await invoke<string | null>("get_setting", { key: "sync_profile_group_ids" });
        if (gjson === null || gjson === "") {
            selectedGroups = chipGroups.map((g) => g.id);
        } else {
            const valid = new Set(chipGroups.map((g) => g.id));
            let parsed: unknown;
            try { parsed = JSON.parse(gjson); } catch { parsed = undefined; }
            if (Array.isArray(parsed) && parsed.every((v) => typeof v === "string")) {
                selectedGroups = [...new Set((parsed as string[]).filter((v) => valid.has(v)))];
            } else {
                selectedGroups = chipGroups.map((g) => g.id);
                await invoke("set_setting", { key: "sync_profile_group_ids", value: "" });
            }
        }
    });

    onDestroy(() => {
        if (ghSaveTimer) clearTimeout(ghSaveTimer);
        if (wdSaveTimer) clearTimeout(wdSaveTimer);
    });

    function startGhSaveTimer() {
        if (ghSaveTimer) clearTimeout(ghSaveTimer);
        ghSaveTimer = setTimeout(() => { githubSaveMsg = ""; ghSaveTimer = null; }, 2000);
    }

    function startWdSaveTimer() {
        if (wdSaveTimer) clearTimeout(wdSaveTimer);
        wdSaveTimer = setTimeout(() => { webdavSaveMsg = ""; wdSaveTimer = null; }, 2000);
    }

    function isHttpUrl(url: string): boolean {
        try { return new URL(url).protocol === "http:"; } catch { return false; }
    }

    function validateWebdavUrl(url: string): MessageKey | null {
        if (!url.trim()) {
            return "error.webdav_url_missing";
        }
        let u: URL;
        try { u = new URL(url); } catch {
            return "error.webdav_url_format_invalid";
        }
        if (u.protocol !== "http:" && u.protocol !== "https:") {
            return "error.webdav_url_format_invalid";
        }
        if (u.username || u.password) {
            return "error.webdav_url_userinfo_forbidden";
        }
        if (u.search || u.hash) {
            return "error.webdav_url_query_fragment_forbidden";
        }
        return null;
    }

    async function saveGithubSettings() {
        try {
            await invoke("set_setting", { key: "github_token", value: githubToken });
            await invoke("set_setting", { key: "github_repo", value: githubRepo });
            await invoke("set_setting", { key: "github_branch", value: githubBranch });
            await invoke("set_setting", { key: "sync_github_enabled", value: githubEnabled ? "1" : "0" });
            githubSaveMsg = t("github.saved");
            githubSaveIsError = false;
        } catch (e: any) {
            githubSaveMsg = errMsg(e);
            githubSaveIsError = true;
        }
        startGhSaveTimer();
    }

    async function saveWebdavSettings() {
        const err = validateWebdavUrl(webdavUrl);
        if (err) {
            webdavSaveMsg = t(err);
            webdavSaveIsError = true;
            startWdSaveTimer();
            return;
        }
        try {
            await invoke("set_setting", { key: "webdav_url", value: webdavUrl });
            await invoke("set_setting", { key: "webdav_username", value: webdavUsername });
            await invoke("set_setting", { key: "webdav_password", value: webdavPassword });
            await invoke("set_setting", { key: "sync_webdav_enabled", value: webdavEnabled ? "1" : "0" });
            webdavSaveMsg = t("webdav.saved");
            webdavSaveIsError = false;
        } catch (e: any) {
            webdavSaveMsg = errMsg(e);
            webdavSaveIsError = true;
        }
        startWdSaveTimer();
    }

    async function setFlag(key: string, val: boolean) {
        flags[key] = val;
        await invoke("set_setting", { key, value: val ? "1" : "0" });
    }

    async function toggleGroup(id: string, checked: boolean) {
        selectedGroups = checked
            ? [...selectedGroups, id]
            : selectedGroups.filter((g) => g !== id);
        const allSelected = selectedGroups.length === chipGroups.length;
        await invoke("set_setting", {
            key: "sync_profile_group_ids",
            value: allSelected ? "" : JSON.stringify(selectedGroups),
        });
    }

    async function onEnableChange(source: "github" | "webdav") {
        if (source === "github") {
            await invoke("set_setting", { key: "sync_github_enabled", value: githubEnabled ? "1" : "0" });
        } else {
            await invoke("set_setting", { key: "sync_webdav_enabled", value: webdavEnabled ? "1" : "0" });
        }
    }

    function canPushPull(): boolean {
        const gh = githubEnabled && githubToken && githubRepo;
        const wd = webdavEnabled && webdavUrl && webdavPassword;
        return gh || wd;
    }

    function closePwDialog() {
        showPwDialog = false;
        pw1 = "";
        pw2 = "";
        pwError = "";
        globalMsg = "";
    }

    function askPassword(mode: "push" | "pull") {
        if (!canPushPull()) {
            globalMsg = t("sync.no_source");
            return;
        }
        pwMode = mode;
        pw1 = "";
        pw2 = "";
        pwError = "";
        globalMsg = "";
        showPwDialog = true;
    }

    async function confirmPassword() {
        if (!pw1) {
            pwError = t("sync.password_empty");
            return;
        }
        if (pwMode === "push" && pw1 !== pw2) {
            pwError = t("sync.password_mismatch");
            return;
        }

        showPwDialog = false;
        const password = pw1;
        pw1 = "";
        pw2 = "";

        /* GitHub */
        if (githubEnabled && githubToken && githubRepo) {
            githubSyncing = true;
            githubMsg = t(pwMode === "push" ? "sync.status.pushing" : "sync.status.pulling");
            try {
                if (pwMode === "push") {
                    await invoke("github_push", { password });
                    githubMsg = t("github.push_ok");
                } else {
                    await invoke("github_pull", { password });
                    githubMsg = t("github.pull_ok");
                }
            } catch (e: any) {
                githubMsg = t("github.failed", { error: errMsg(e) });
            } finally {
                githubSyncing = false;
            }
        }

        /* WebDAV */
        if (webdavEnabled && webdavUrl && webdavPassword) {
            webdavSyncing = true;
            webdavMsg = t(pwMode === "push" ? "sync.status.pushing" : "sync.status.pulling");
            try {
                if (pwMode === "push") {
                    await invoke("webdav_push", { password });
                    webdavMsg = t("webdav.push_ok");
                } else {
                    await invoke("webdav_pull", { password });
                    webdavMsg = t("webdav.pull_ok");
                }
            } catch (e: any) {
                webdavMsg = t("webdav.failed", { error: errMsg(e) });
            } finally {
                webdavSyncing = false;
            }
        }
    }
</script>

<div class="page">
    <!-- GitHub source -->
    <div class="card surface-raised source-card">
        <div class="source-head">
            <h4>GitHub</h4>
            <label class="switch">
                <input type="checkbox" bind:checked={githubEnabled} onchange={() => onEnableChange("github")} aria-label={t("sync.enable_github")} />
                <span class="slider"></span>
            </label>
        </div>

        {#if githubEnabled}
            <p class="pat-hint">
                {t("github.pat_hint1")}<br/>
                {t("github.pat_hint2")}<br/>
                {t("github.pat_hint3")}
            </p>
            <div class="field">
                <label for="gh-token">{t("github.token")}</label>
                <input id="gh-token" type="password" bind:value={githubToken} placeholder="ghp_xxxx" autocomplete="off"/>
            </div>
            <div class="field">
                <label for="gh-repo">{t("github.repo")}</label>
                <input id="gh-repo" type="text" bind:value={githubRepo} placeholder="user/rssh-config" autocomplete="off"/>
            </div>
            <div class="field">
                <label for="gh-branch">{t("github.branch")}</label>
                <input id="gh-branch" type="text" bind:value={githubBranch} placeholder="main" autocomplete="off"/>
            </div>
            <button class="btn btn-accent btn-sm save-btn" onclick={saveGithubSettings}>⛰ {t("common.save")}</button>
            {#if githubSaveMsg}
                <div class="msg" class:error={githubSaveIsError}>{githubSaveMsg}</div>
            {/if}
        {/if}
    </div>

    <!-- WebDAV source -->
    <div class="card surface-raised source-card">
        <div class="source-head">
            <h4>WebDAV</h4>
            <label class="switch">
                <input type="checkbox" bind:checked={webdavEnabled} onchange={() => onEnableChange("webdav")} aria-label={t("sync.enable_webdav")} />
                <span class="slider"></span>
            </label>
        </div>

        {#if webdavEnabled}
            <div class="field">
                <label for="wd-url">{t("webdav.url")}</label>
                <input id="wd-url" type="text" bind:value={webdavUrl} placeholder={t("webdav.url_placeholder")} autocomplete="off"/>
                {#if isHttpUrl(webdavUrl)}
                    <div class="hint warning">{t("sync.webdav_http_warning")}</div>
                {/if}
            </div>
            <div class="field">
                <label for="wd-username">{t("webdav.username")}</label>
                <input id="wd-username" type="text" bind:value={webdavUsername} autocomplete="off"/>
            </div>
            <div class="field">
                <label for="wd-password">{t("webdav.password")}</label>
                <input id="wd-password" type="password" bind:value={webdavPassword} autocomplete="new-password"/>
            </div>
            <button class="btn btn-accent btn-sm save-btn" onclick={saveWebdavSettings}>⛰ {t("common.save")}</button>
            {#if webdavSaveMsg}
                <div class="msg" class:error={webdavSaveIsError}>{webdavSaveMsg}</div>
            {/if}
        {/if}
    </div>

    <!-- Sync content -->
    <div class="card surface-raised sync-card">
        <div class="sync-head">
            <div class="sync-head-body">
                <div class="sync-row-title">{t("sync.categories.profiles")}</div>
                <div class="sync-row-desc">{t("sync.categories.profiles_hint")}</div>
            </div>
        </div>
        {#if chipGroups.length}
            <div class="chips">
                {#each chipGroups as g (g.id)}
                    {@const sel = selectedGroups.includes(g.id)}
                    <button type="button" class="chip" class:selected={sel}
                            style={g.color ? `--chip: ${g.color}` : ""}
                            aria-pressed={sel}
                            onclick={() => toggleGroup(g.id, !sel)}>
                        <span class="chip-dot" style={g.color ? `background: ${g.color}` : ""}></span>
                        {g.name}
                    </button>
                {/each}
            </div>
        {/if}

        <div class="card-divider"></div>

        {#each SYNC_ITEMS as it, i (it.key)}
            <div class="sync-head">
                <div class="sync-head-body">
                    <div class="sync-row-title">{t(it.label)}</div>
                </div>
                <label class="switch">
                    <input type="checkbox" checked={flags[it.key] ?? true}
                           onchange={(e) => setFlag(it.key, e.currentTarget.checked)}
                           aria-label={t(it.label)}/>
                    <span class="slider"></span>
                </label>
            </div>
            {#if i < SYNC_ITEMS.length - 1}<div class="card-divider"></div>{/if}
        {/each}
    </div>

    <!-- Global actions -->
    <div class="card surface-raised actions-card">
        <div class="btn-row">
            <button class="btn btn-accent btn-sm" onclick={() => askPassword("push")} disabled={anySyncing}>𓍼 ོ☁︎ {t("sync.push")}</button>
            <button class="btn btn-sm" onclick={() => askPassword("pull")} disabled={anySyncing}>༄ {t("sync.pull")}</button>
        </div>
        {#if globalMsg}
            <div class="msg error">{globalMsg}</div>
        {/if}
        {#if githubMsg || webdavMsg}
            <div class="msg">{[githubMsg, webdavMsg].filter(Boolean).join("\n")}</div>
        {/if}
    </div>
</div>

<!-- Password dialog -->
{#if showPwDialog}
    <div class="dialog-backdrop" onclick={closePwDialog} onkeydown={(e) => e.key === "Escape" && closePwDialog()} role="presentation" tabindex="-1">
        <div class="dialog surface-raised" onclick={(e) => e.stopPropagation()}
             role="dialog" aria-modal="true" aria-labelledby="sync-pw-title">
            <h3 id="sync-pw-title">{pwMode === "push" ? t("sync.set_password") : t("sync.enter_password")}</h3>
            <input type="password" bind:value={pw1} placeholder={t("sync.password")} autocomplete="new-password"
                   autofocus aria-describedby={pwError ? "pw-error" : undefined}
                   onkeydown={(e) => { if (e.key === "Enter") confirmPassword(); }}/>
            {#if pwMode === "push"}
                <input type="password" bind:value={pw2} placeholder={t("sync.confirm_password")} autocomplete="new-password"
                       aria-describedby={pwError ? "pw-error" : undefined}
                       onkeydown={(e) => { if (e.key === "Enter") confirmPassword(); }}/>
            {/if}
            {#if pwError}
                <div id="pw-error" class="pw-error" role="alert">{pwError}</div>
            {/if}
            <div class="btn-row">
                <button class="btn btn-sm" onclick={closePwDialog}>{t("common.cancel")}</button>
                <button class="btn btn-accent btn-sm" onclick={confirmPassword}>{t("common.confirm")}</button>
            </div>
        </div>
    </div>
{/if}

<style>
    .page {
        padding: 24px;
        display: flex;
        flex-direction: column;
        gap: 16px;
    }

    .card {
        padding: 18px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }

    .source-card h4 {
        font-size: 14px;
        color: var(--text);
        margin: 0;
    }

    .source-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
    }

    .pat-hint {
        margin: 0;
        font-size: 11px;
        color: var(--text-dim);
        line-height: 1.5;
    }

    .field {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    .field label {
        font-size: 11px;
        color: var(--text-sub);
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .field input {
        width: 100%;
        box-sizing: border-box;
    }

    .hint {
        font-size: 11px;
        line-height: 1.5;
    }
    .hint.warning {
        color: var(--warning, var(--error));
    }

    .save-btn {
        align-self: flex-start;
    }

    .btn-row {
        display: flex;
        gap: 8px;
    }

    .sync-head {
        display: flex;
        align-items: center;
        gap: 12px;
    }
    .sync-head-body {
        flex: 1;
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    .sync-row-title {
        font-size: 13px;
        color: var(--text);
    }
    .sync-row-desc {
        font-size: 11px;
        color: var(--text-dim);
        line-height: 1.5;
    }

    .card-divider {
        height: 1px;
        background: var(--divider);
        margin: 2px -18px;
    }

    .chips {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }
    .chip {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        padding: 3px 10px;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: transparent;
        color: var(--text-sub);
        cursor: pointer;
    }
    .chip-dot {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--text-dim);
        flex: none;
    }
    .chip.selected {
        border-color: var(--chip, var(--accent));
        background: color-mix(in srgb, var(--chip, var(--accent)) 18%, transparent);
        color: var(--text);
    }

    .actions-card {
        margin-top: 8px;
    }

    .msg {
        font-size: 12px;
        color: var(--accent);
        white-space: pre-line;
    }
    .msg.error {
        color: var(--error);
    }

    .dialog-backdrop {
        position: fixed;
        inset: 0;
        z-index: 500;
        background: var(--overlay-strong);
        display: flex;
        align-items: center;
        justify-content: center;
    }

    .dialog {
        background: var(--bg);
        box-shadow: var(--raised);
        border-radius: var(--radius);
        padding: calc(24px * var(--density));
        min-width: 300px;
        max-width: calc(100vw - 32px);
        display: flex;
        flex-direction: column;
        gap: 12px;
    }

    .dialog h3 {
        font-size: 16px;
        color: var(--text);
    }

    .pw-error {
        font-size: 12px;
        color: var(--error);
    }

    @media (max-width: 480px) {
        .page {
            padding: 16px;
        }
        .card {
            padding: 14px;
        }
        .btn-row {
            flex-direction: column;
        }
        .card-divider {
            margin: 2px -14px;
        }
    }
</style>
