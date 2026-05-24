<script lang="ts">
    import {onMount} from "svelte";
    import {invoke} from "@tauri-apps/api/core";
    import { errMsg } from "../i18n/index.svelte.ts";

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

    onMount(async () => {
        githubToken = await invoke<string | null>("get_setting", {key: "github_token"}) ?? "";
        githubRepo = await invoke<string | null>("get_setting", {key: "github_repo"}) ?? "";
        githubBranch = await invoke<string | null>("get_setting", {key: "github_branch"}) ?? "main";
    });

    async function saveSettings() {
        await invoke("set_setting", {key: "github_token", value: githubToken});
        await invoke("set_setting", {key: "github_repo", value: githubRepo});
        await invoke("set_setting", {key: "github_branch", value: githubBranch});
        msg = "Settings saved";
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
            pwError = "Password cannot be empty";
            return;
        }
        if (pwMode === "push" && pw1 !== pw2) {
            pwError = "Passwords do not match";
            return;
        }

        showPwDialog = false;
        syncing = true;
        msg = "";
        try {
            if (pwMode === "push") {
                await invoke("github_push", {password: pw1});
                msg = "Push successful";
            } else {
                await invoke("github_pull", {password: pw1});
                msg = "Pull successful";
            }
        } catch (e: any) {
            msg = "Failed: " + errMsg(e);
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
            Create a PAT at github.com → Settings → Developer settings → Personal access tokens → Fine-grained tokens.<br/>
            Repository access: Select "Only select repositories" (instead of "All repositories")<br/>
            with "Contents" read &amp; write permission.
        </p>

        <div class="field">
            <label for="gh-token">Personal Access Token</label>
            <input id="gh-token" type="password" bind:value={githubToken} placeholder="ghp_xxxx"/>
        </div>
        <div class="field">
            <label for="gh-repo">Repository (owner/repo)</label>
            <input id="gh-repo" type="text" bind:value={githubRepo} placeholder="user/rssh-config"/>
        </div>
        <div class="field">
            <label for="gh-branch">Branch</label>
            <input id="gh-branch" type="text" bind:value={githubBranch} placeholder="main"/>
        </div>

        <!-- Save 跟 Push 同属"主操作"，用同样的 btn-accent 样式。
             Pull 用默认 btn（secondary）跟 sshell buildSecondaryButton 对齐。 -->
        <button class="btn btn-accent btn-sm save-btn" onclick={saveSettings}>⛰ Save</button>

        <div class="btn-row">
            <button class="btn btn-accent btn-sm" onclick={() => askPassword("push")} disabled={syncing}>𓍼 ོ☁︎ Push to GitHub</button>
            <button class="btn btn-sm" onclick={() => askPassword("pull")} disabled={syncing}>༄ Pull from GitHub</button>
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
            <h3 id="gh-pw-title">{pwMode === "push" ? "Set Encryption Password" : "Enter Decryption Password"}</h3>
            <input type="password" bind:value={pw1} placeholder="Password"
                   onkeydown={(e) => { if (e.key === "Enter") confirmPassword(); }}/>
            {#if pwMode === "push"}
                <input type="password" bind:value={pw2} placeholder="Confirm Password"
                       onkeydown={(e) => { if (e.key === "Enter") confirmPassword(); }}/>
            {/if}
            {#if pwError}
                <div class="pw-error">{pwError}</div>
            {/if}
            <div class="btn-row">
                <button class="btn btn-sm" onclick={() => showPwDialog = false}>Cancel</button>
                <button class="btn btn-accent btn-sm" onclick={confirmPassword}>Confirm</button>
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
