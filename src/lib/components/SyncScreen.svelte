<script lang="ts">
    import {onMount, onDestroy} from "svelte";
    import {invoke} from "@tauri-apps/api/core";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import * as syncStatus from "../stores/sync.svelte.ts";
    import Modal from "./Modal.svelte";
    import SyncAutoPullToggle from "./SyncAutoPullToggle.svelte";
    import type { MessageKey } from "../i18n/locales/en";

    /* ── GitHub source state ─────────────────────────────────────────────── */
    let githubEnabled = $state(false);
    let githubToken = $state("");
    let githubRepo = $state("");
    let githubBranch = $state("main");
    let githubSyncing = $state(false);
    let githubMsg = $state("");
    let githubMsgIsError = $state(false);
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
    let webdavMsgIsError = $state(false);
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
    let pwSource = $state<syncStatus.SyncSource>("github");
    let pwMode = $state<"push" | "pull">("push");
    let pw1 = $state("");
    let pw2 = $state("");
    let pwError = $state("");
    let localVersion = $derived(syncStatus.localMetadata()?.version ?? null);
    let githubRemoteVersion = $derived(syncStatus.providerStatus("github")?.remote?.version ?? null);
    let webdavRemoteVersion = $derived(syncStatus.providerStatus("webdav")?.remote?.version ?? null);
    let githubAutoPull = $derived(syncStatus.autoPullEnabled("github"));
    let webdavAutoPull = $derived(syncStatus.autoPullEnabled("webdav"));
    let githubStatusError = $derived(syncStatus.providerStatus("github")?.error ?? "");
    let webdavStatusError = $derived(syncStatus.providerStatus("webdav")?.error ?? "");

    async function loadGithubSettings() {
        const [token, repo, branch, enabled] = await Promise.all([
            invoke<string | null>("get_setting", { key: "github_token" }),
            invoke<string | null>("get_setting", { key: "github_repo" }),
            invoke<string | null>("get_setting", { key: "github_branch" }),
            invoke<string | null>("get_setting", { key: "sync_github_enabled" }),
        ]);
        githubToken = token ?? "";
        githubRepo = repo ?? "";
        githubBranch = branch ?? "main";
        githubEnabled = enabled !== "0";
    }

    async function loadWebdavSettings() {
        const [url, username, password, enabled] = await Promise.all([
            invoke<string | null>("get_setting", { key: "webdav_url" }),
            invoke<string | null>("get_setting", { key: "webdav_username" }),
            invoke<string | null>("get_setting", { key: "webdav_password" }),
            invoke<string | null>("get_setting", { key: "sync_webdav_enabled" }),
        ]);
        webdavUrl = url ?? "";
        webdavUsername = username ?? "";
        webdavPassword = password ?? "";
        webdavEnabled = enabled === "1";
    }

    async function loadSyncPreferences() {
        const [values, loadedGroups, gjson] = await Promise.all([
            Promise.all(SYNC_ITEMS.map((item) =>
                invoke<string | null>("get_setting", { key: item.key })
            )),
            invoke<{ id: string; name: string; color: string }[]>("list_groups").catch(() => []),
            invoke<string | null>("get_setting", { key: "sync_profile_group_ids" }),
        ]);
        flags = Object.fromEntries(SYNC_ITEMS.map((item, index) => [
            item.key,
            values[index] === null || values[index] !== "0",
        ]));
        groups = loadedGroups;
        const availableGroups = [
            ...loadedGroups,
            { id: "", name: t("profile.ungrouped"), color: "" },
        ];
        if (gjson === null || gjson === "") {
            selectedGroups = availableGroups.map((group) => group.id);
        } else {
            const valid = new Set(availableGroups.map((group) => group.id));
            let parsed: unknown;
            try { parsed = JSON.parse(gjson); } catch { parsed = undefined; }
            if (Array.isArray(parsed) && parsed.every((v) => typeof v === "string")) {
                selectedGroups = [...new Set((parsed as string[]).filter((v) => valid.has(v)))];
            } else {
                selectedGroups = availableGroups.map((group) => group.id);
                await invoke("set_setting", { key: "sync_profile_group_ids", value: "" });
            }
        }
    }

    onMount(async () => {
        // Metadata belongs to the shared sync store, not to this form's setup.
        // Start it before form loading; provider settings and category settings
        // also load in parallel instead of serializing sixteen IPC round trips.
        void syncStatus.runCheck({ silent: true });
        await Promise.all([
            loadGithubSettings(),
            loadWebdavSettings(),
            loadSyncPreferences(),
        ]);
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
            void syncStatus.refreshAfterMutation();
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
            void syncStatus.refreshAfterMutation();
        } catch (e: any) {
            webdavSaveMsg = errMsg(e);
            webdavSaveIsError = true;
        }
        startWdSaveTimer();
    }

    async function setFlag(key: string, val: boolean) {
        flags[key] = val;
        await invoke("set_setting", { key, value: val ? "1" : "0" });
        await syncStatus.refreshLocalAfterMutation();
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
        await syncStatus.refreshLocalAfterMutation();
    }

    async function onEnableChange(source: "github" | "webdav") {
        if (source === "github") {
            await invoke("set_setting", { key: "sync_github_enabled", value: githubEnabled ? "1" : "0" });
        } else {
            await invoke("set_setting", { key: "sync_webdav_enabled", value: webdavEnabled ? "1" : "0" });
        }
        await syncStatus.refreshAfterMutation();
    }

    function sourceConfigured(source: syncStatus.SyncSource): boolean {
        return source === "github"
            ? Boolean(githubEnabled && githubToken && githubRepo)
            : Boolean(webdavEnabled && webdavUrl && webdavPassword);
    }

    function closePwDialog() {
        showPwDialog = false;
        pw1 = "";
        pw2 = "";
        pwError = "";
    }

    function askPassword(source: syncStatus.SyncSource, mode: "push" | "pull") {
        if (!sourceConfigured(source)) {
            const message = t("sync.no_source");
            if (source === "github") {
                githubMsg = message;
                githubMsgIsError = true;
            } else {
                webdavMsg = message;
                webdavMsgIsError = true;
            }
            return;
        }
        pwSource = source;
        pwMode = mode;
        pw1 = "";
        pw2 = "";
        pwError = "";
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
        const source = pwSource;
        const setSyncing = (value: boolean) => {
            if (source === "github") githubSyncing = value;
            else webdavSyncing = value;
        };
        const setMessage = (value: string, isError = false) => {
            if (source === "github") {
                githubMsg = value;
                githubMsgIsError = isError;
            } else {
                webdavMsg = value;
                webdavMsgIsError = isError;
            }
        };

        setSyncing(true);
        setMessage("");
        try {
            await invoke(syncStatus.commandFor(source, pwMode), { password });
            const key = `${source}.${pwMode}_ok` as MessageKey;
            setMessage(t(key));
            await syncStatus.refreshAfterMutation();
        } catch (e: any) {
            setMessage(t(`${source}.failed` as MessageKey, { error: errMsg(e) }), true);
        } finally {
            setSyncing(false);
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
            <div class="source-actions">
                <div class="auto-pull-row">
                    <div class="auto-pull-copy">
                        <span>{t("sync.auto_pull")}</span>
                        <small>{t("sync.auto_pull_hint")}</small>
                    </div>
                    <SyncAutoPullToggle source="github" enabled={githubAutoPull}
                                        onError={(message) => {
                                            githubMsg = t("github.failed", { error: message });
                                            githubMsgIsError = true;
                                        }}>
                        {#snippet trigger(requestToggle, saving)}
                            <label class="switch">
                                <input type="checkbox" checked={githubAutoPull}
                                       disabled={githubSyncing || saving || syncStatus.providerStatus("github") === null}
                                       onclick={(event) => {
                                           event.preventDefault();
                                           githubMsg = "";
                                           githubMsgIsError = false;
                                           if (!githubAutoPull && !sourceConfigured("github")) {
                                               githubMsg = t("sync.no_source");
                                               githubMsgIsError = true;
                                               return;
                                           }
                                           requestToggle();
                                       }}
                                       aria-label={t("sync.auto_pull_github")}/>
                                <span class="slider"></span>
                            </label>
                        {/snippet}
                    </SyncAutoPullToggle>
                </div>
                <div class="btn-row">
                    <button class="btn btn-accent btn-sm version-btn" onclick={() => askPassword("github", "push")} disabled={githubSyncing}>
                        <span>𓍼 ོ☁︎ {t("sync.push")}{#if localVersion !== null} · V{localVersion}{/if}</span>
                        {#if syncStatus.hasLocalUpdate("github")}<span class="version-dot" aria-hidden="true"></span>{/if}
                    </button>
                    <button class="btn btn-sm version-btn" onclick={() => askPassword("github", "pull")} disabled={githubSyncing}>
                        <span>༄ {t("sync.pull")}{#if githubRemoteVersion !== null} · V{githubRemoteVersion}{/if}</span>
                        {#if syncStatus.hasRemoteUpdate("github")}<span class="version-dot" aria-hidden="true"></span>{/if}
                    </button>
                </div>
            </div>
            {#if githubMsg}
                <div class="msg" class:error={githubMsgIsError}>{githubMsg}</div>
            {:else if githubStatusError}
                <div class="msg error">{errMsg(githubStatusError)}</div>
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
            <div class="source-actions">
                <div class="auto-pull-row">
                    <div class="auto-pull-copy">
                        <span>{t("sync.auto_pull")}</span>
                        <small>{t("sync.auto_pull_hint")}</small>
                    </div>
                    <SyncAutoPullToggle source="webdav" enabled={webdavAutoPull}
                                        onError={(message) => {
                                            webdavMsg = t("webdav.failed", { error: message });
                                            webdavMsgIsError = true;
                                        }}>
                        {#snippet trigger(requestToggle, saving)}
                            <label class="switch">
                                <input type="checkbox" checked={webdavAutoPull}
                                       disabled={webdavSyncing || saving || syncStatus.providerStatus("webdav") === null}
                                       onclick={(event) => {
                                           event.preventDefault();
                                           webdavMsg = "";
                                           webdavMsgIsError = false;
                                           if (!webdavAutoPull && !sourceConfigured("webdav")) {
                                               webdavMsg = t("sync.no_source");
                                               webdavMsgIsError = true;
                                               return;
                                           }
                                           requestToggle();
                                       }}
                                       aria-label={t("sync.auto_pull_webdav")}/>
                                <span class="slider"></span>
                            </label>
                        {/snippet}
                    </SyncAutoPullToggle>
                </div>
                <div class="btn-row">
                    <button class="btn btn-accent btn-sm version-btn" onclick={() => askPassword("webdav", "push")} disabled={webdavSyncing}>
                        <span>𓍼 ོ☁︎ {t("sync.push")}{#if localVersion !== null} · V{localVersion}{/if}</span>
                        {#if syncStatus.hasLocalUpdate("webdav")}<span class="version-dot" aria-hidden="true"></span>{/if}
                    </button>
                    <button class="btn btn-sm version-btn" onclick={() => askPassword("webdav", "pull")} disabled={webdavSyncing}>
                        <span>༄ {t("sync.pull")}{#if webdavRemoteVersion !== null} · V{webdavRemoteVersion}{/if}</span>
                        {#if syncStatus.hasRemoteUpdate("webdav")}<span class="version-dot" aria-hidden="true"></span>{/if}
                    </button>
                </div>
            </div>
            {#if webdavMsg}
                <div class="msg" class:error={webdavMsgIsError}>{webdavMsg}</div>
            {:else if webdavStatusError}
                <div class="msg error">{errMsg(webdavStatusError)}</div>
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

</div>

<!-- Password dialog -->
{#if showPwDialog}
    <Modal onClose={closePwDialog} class="stack" aria-labelledby="sync-pw-title">
        <h3 id="sync-pw-title" class="dialog-title">{pwMode === "push" ? t("sync.set_password") : t("sync.enter_password")}</h3>
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
        <div class="modal-actions">
            <button class="btn btn-sm" onclick={closePwDialog}>{t("common.cancel")}</button>
            <button class="btn btn-accent btn-sm" onclick={confirmPassword}>{t("common.confirm")}</button>
        </div>
    </Modal>
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
    .source-actions {
        display: flex;
        flex-direction: column;
        gap: 12px;
        padding-top: 12px;
        border-top: 1px solid var(--divider);
    }
    .version-btn {
        display: inline-flex;
        align-items: center;
        gap: 7px;
        font-variant-numeric: tabular-nums;
    }
    .version-dot {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--error);
        flex: none;
    }
    .auto-pull-row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
    }
    .auto-pull-copy {
        display: flex;
        flex-direction: column;
        gap: 3px;
        color: var(--text);
        font-size: 13px;
    }
    .auto-pull-copy small {
        color: var(--text-dim);
        font-size: 11px;
        line-height: 1.5;
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

    .msg {
        font-size: 12px;
        color: var(--accent);
        white-space: pre-line;
    }
    .msg.error {
        color: var(--error);
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
