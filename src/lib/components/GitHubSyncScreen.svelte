<script lang="ts">
    import {onMount} from "svelte";
    import {invoke} from "@tauri-apps/api/core";

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
            msg = "Failed: " + String(e);
        } finally {
            syncing = false;
        }
    }
</script>

<div class="page">
    <div class="form">
        <div>
            Create a PAT at github.com → Settings → Developer settings → Personal access tokens → Fine-grained tokens.<br/>
            Repository access：Select "Only select repositories" (instead of "All repositories")<br/>
            with "Contents" read & write permission.
        </div>
        <label>Personal Access Token</label>
        <input type="password" bind:value={githubToken} placeholder="ghp_xxxx"/>
        <label>Repository (owner/repo)</label>
        <input type="text" bind:value={githubRepo} placeholder="user/rssh-config"/>
        <label>Branch</label>
        <input type="text" bind:value={githubBranch} placeholder="main"/>
        <button class="btn btn-sm" onclick={saveSettings}>Save</button>
        <div class="divider"></div>
        <div class="btn-row">
            <button class="btn btn-accent btn-sm" onclick={() => askPassword("push")} disabled={syncing}>Push to GitHub</button>
            <button class="btn btn-sm" onclick={() => askPassword("pull")} disabled={syncing}>Pull from GitHub</button>
        </div>
        {#if msg}
            <div class="msg">{msg}</div>
        {/if}
    </div>
</div>

<!-- Password dialog -->
{#if showPwDialog}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dialog-backdrop" onclick={() => showPwDialog = false}>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="dialog" onclick={(e) => e.stopPropagation()}>
            <h3>{pwMode === "push" ? "Set Encryption Password" : "Enter Decryption Password"}</h3>
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

    .form {
        display: flex;
        flex-direction: column;
        gap: 10px;
    }

    .btn-row {
        display: flex;
        gap: 8px;
    }

    .msg {
        font-size: 12px;
        color: var(--accent);
    }

    .dialog-backdrop {
        position: fixed;
        inset: 0;
        z-index: 500;
        background: rgba(0, 0, 0, 0.5);
        display: flex;
        align-items: center;
        justify-content: center;
    }

    .dialog {
        background: var(--bg);
        box-shadow: var(--raised);
        border-radius: var(--radius);
        padding: 24px;
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
