<script lang="ts">
    import {onMount} from "svelte";
    import {invoke} from "@tauri-apps/api/core";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import type { MessageKey } from "../i18n/locales/en";

    let githubToken = $state("");
    let githubRepo = $state("");
    let githubBranch = $state("main");
    let syncing = $state(false);
    let msg = $state("");

    /* Password dialog state */
    let showPwDialog = $state(false);
    let pwMode = $state<"push" | "pull">("push");
    let pw1 = $state("");
    let pw2 = $state("");
    let pwError = $state("");

    /* Per-category sync toggles (push-side filter). Stored in settings as
       "1"/"0"; absent = on. A disabled item is simply not uploaded. */
    const SYNC_ITEMS: { key: string; label: MessageKey }[] = [
        { key: "sync_include_credentials", label: "github.sync_credentials" },
        { key: "sync_include_forwards", label: "github.sync_forwards" },
        { key: "sync_include_groups", label: "github.sync_groups" },
        { key: "sync_include_serial", label: "github.sync_serial" },
        { key: "sync_include_highlights", label: "github.sync_highlights" },
        { key: "sync_include_snippets", label: "github.sync_snippets" },
        { key: "sync_include_skills", label: "github.sync_skills" },
        { key: "sync_include_ai_redact", label: "github.sync_ai_redact" },
        { key: "sync_include_ai_blacklist", label: "github.sync_ai_blacklist" },
        { key: "sync_include_ai", label: "github.sync_ai" },
        { key: "sync_include_ai_key", label: "github.sync_ai_key" },
    ];
    let flags = $state<Record<string, boolean>>({});
    let groups = $state<{ id: string; name: string; color: string }[]>([]);
    // Group ids whose profiles sync. Default = all selected. When all are
    // selected we persist "" (= no filter, syncs everything incl. ungrouped);
    // a strict subset persists as a JSON array; an empty array syncs nothing.
    let selectedGroups = $state<string[]>([]);

    onMount(async () => {
        githubToken = await invoke<string | null>("get_setting", {key: "github_token"}) ?? "";
        githubRepo = await invoke<string | null>("get_setting", {key: "github_repo"}) ?? "";
        githubBranch = await invoke<string | null>("get_setting", {key: "github_branch"}) ?? "main";

        // Load sync toggles (absent = on) + the profile group selection.
        for (const it of SYNC_ITEMS) {
            const v = await invoke<string | null>("get_setting", {key: it.key});
            flags[it.key] = v === null || v !== "0";
        }
        groups = await invoke<{ id: string; name: string; color: string }[]>("list_groups").catch(() => []);
        const gjson = await invoke<string | null>("get_setting", {key: "sync_profile_group_ids"});
        // Empty/absent = all groups selected (default). Otherwise the saved subset.
        if (gjson === null || gjson === "") {
            selectedGroups = groups.map((g) => g.id);
        } else {
            const valid = new Set(groups.map((g) => g.id));
            let parsed: unknown;
            try { parsed = JSON.parse(gjson); } catch { parsed = undefined; }
            if (Array.isArray(parsed) && parsed.every((v) => typeof v === "string")) {
                // Valid string array (what the backend can parse). Drop stale ids
                // (deleted groups) + de-dup so the "all selected" check stays
                // accurate; the backend tolerates stale ids, so leave it persisted.
                selectedGroups = [...new Set((parsed as string[]).filter((v) => valid.has(v)))];
            } else {
                // Corrupted value the backend rejects on push (not a string array)
                // → every push/pull would fail in a silent loop. Reset to "all" AND
                // repair the persisted setting so the screen and the next push agree.
                selectedGroups = groups.map((g) => g.id);
                await invoke("set_setting", { key: "sync_profile_group_ids", value: "" });
            }
        }
    });

    async function setFlag(key: string, val: boolean) {
        flags[key] = val;
        await invoke("set_setting", {key, value: val ? "1" : "0"});
    }

    async function toggleGroup(id: string, checked: boolean) {
        selectedGroups = checked
            ? [...selectedGroups, id]
            : selectedGroups.filter((g) => g !== id);
        // All selected → persist "" (no filter, syncs everything incl. ungrouped
        // and any future groups). Otherwise the explicit subset ("[]" = none).
        const allSelected = groups.length > 0 && selectedGroups.length === groups.length;
        await invoke("set_setting", {
            key: "sync_profile_group_ids",
            value: allSelected ? "" : JSON.stringify(selectedGroups),
        });
    }

    async function saveSettings() {
        await invoke("set_setting", {key: "github_token", value: githubToken});
        await invoke("set_setting", {key: "github_repo", value: githubRepo});
        await invoke("set_setting", {key: "github_branch", value: githubBranch});
        msg = t("github.saved");
        setTimeout(() => msg = "", 2000);
    }

    function askPassword(mode: "push" | "pull") {
        pwMode = mode;
        pw1 = "";
        pw2 = "";
        pwError = "";
        showPwDialog = true;
    }

    async function confirmPassword() {
        if (!pw1) {
            pwError = t("github.password_empty");
            return;
        }
        if (pwMode === "push" && pw1 !== pw2) {
            pwError = t("github.password_mismatch");
            return;
        }

        showPwDialog = false;
        syncing = true;
        msg = "";
        try {
            if (pwMode === "push") {
                await invoke("github_push", {password: pw1});
                msg = t("github.push_ok");
            } else {
                await invoke("github_pull", {password: pw1});
                msg = t("github.pull_ok");
            }
        } catch (e: any) {
            msg = t("github.failed", { error: errMsg(e) });
        } finally {
            syncing = false;
        }
    }
</script>

<div class="page">
    <!-- 单卡片包住所有内容（参考 sshell config_manager_screen）。
         背景用项目现成的 .surface-raised，避免新加自定义视觉令牌。 -->
    <div class="card surface-raised">
        <p class="pat-hint">
            {t("github.pat_hint1")}<br/>
            {t("github.pat_hint2")}<br/>
            {t("github.pat_hint3")}
        </p>

        <div class="field">
            <label for="gh-token">{t("github.token")}</label>
            <input id="gh-token" type="password" bind:value={githubToken} placeholder="ghp_xxxx"/>
        </div>
        <div class="field">
            <label for="gh-repo">{t("github.repo")}</label>
            <input id="gh-repo" type="text" bind:value={githubRepo} placeholder="user/rssh-config"/>
        </div>
        <div class="field">
            <label for="gh-branch">{t("github.branch")}</label>
            <input id="gh-branch" type="text" bind:value={githubBranch} placeholder="main"/>
        </div>

        <!-- Save is the only action in this card; Push/Pull live with the sync items. -->
        <button class="btn btn-accent btn-sm save-btn" onclick={saveSettings}>⛰ {t("common.save")}</button>
    </div>

    <!-- Sync items: group chips on top, one switch per category, then actions. -->
    <div class="card surface-raised sync-card">
<!--        <div class="sync-title">{t("github.sync_section")}</div>-->
<!--        <p class="pat-hint">{t("github.sync_hint")}</p>-->

        <!-- Profiles: pick groups like tags. All selected by default. -->
        <div class="sync-head">
            <div class="sync-head-body">
                <div class="sync-row-title">{t("github.sync_profiles")}</div>
                <div class="sync-row-desc">{t("github.sync_profiles_hint")}</div>
            </div>
        </div>
        {#if groups.length}
            <div class="chips">
                {#each groups as g (g.id)}
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

        <!-- One switch per category, a divider between each. -->
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

        <div class="card-divider"></div>

        <div class="btn-row">
            <button class="btn btn-accent btn-sm" onclick={() => askPassword("push")} disabled={syncing}>𓍼 ོ☁︎ {t("github.push")}</button>
            <button class="btn btn-sm" onclick={() => askPassword("pull")} disabled={syncing}>༄ {t("github.pull")}</button>
        </div>
        {#if msg}
            <div class="msg">{msg}</div>
        {/if}
    </div>
</div>

<!-- Password dialog -->
{#if showPwDialog}
    <div class="dialog-backdrop" onclick={() => showPwDialog = false} role="presentation">
        <div class="dialog surface-raised" onclick={(e) => e.stopPropagation()}
             role="dialog" aria-modal="true" aria-labelledby="gh-pw-title">
            <h3 id="gh-pw-title">{pwMode === "push" ? t("github.set_password") : t("github.enter_password")}</h3>
            <input type="password" bind:value={pw1} placeholder={t("github.password")}
                   onkeydown={(e) => { if (e.key === "Enter") confirmPassword(); }}/>
            {#if pwMode === "push"}
                <input type="password" bind:value={pw2} placeholder={t("github.confirm_password")}
                       onkeydown={(e) => { if (e.key === "Enter") confirmPassword(); }}/>
            {/if}
            {#if pwError}
                <div class="pw-error">{pwError}</div>
            {/if}
            <div class="btn-row">
                <button class="btn btn-sm" onclick={() => showPwDialog = false}>{t("common.cancel")}</button>
                <button class="btn btn-accent btn-sm" onclick={confirmPassword}>{t("common.confirm")}</button>
            </div>
        </div>
    </div>
{/if}

<style>
    .page {
        padding: 24px;
    }

    /* 卡片：复用全局 .surface-raised 提供的 bg + 阴影 + 圆角，本地只加 padding + 内布局。 */
    .card {
        padding: 18px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }

    /* 同步项卡片：与上方 token 卡片拉开间距。 */
    .sync-card {
        margin-top: 16px;
    }

    /* PAT 说明：跟 sshell 对齐 —— 11px / text-dim / 行高 1.5。
       不用 11.5/12 因为内容多行密集，11+1.5 行高最易扫读。 */
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

    /* Save 单独一行；Push/Pull 一行。两组间留同 gap，无需 divider。 */
    .save-btn {
        align-self: flex-start;
    }

    .btn-row {
        display: flex;
        gap: 8px;
    }

    /* Row: title (+ optional desc) left, control (switch) right —
       same "head-body + control" structure as ShellSettings' mouse card. */
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

    /* Card-internal divider spanning the full card width (negative margin
       cancels the 18px card padding). Same as ShellSettings. */
    .card-divider {
        height: 1px;
        background: var(--divider);
        margin: 2px -18px;
    }

    /* Group selection rendered as tag chips. The dot always shows the group's
       own color; selected chips are highlighted with a tint of that color
       (falls back to --accent for colorless groups). */
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
        white-space: pre-line; /* 让 import_partial_failed 等多行错误的 \n 真正换行 */
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
</style>
