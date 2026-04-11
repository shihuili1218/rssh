<script lang="ts">
    import {onMount} from "svelte";
    import {invoke} from "@tauri-apps/api/core";

    let status = $state<{ installed: boolean; path: string; bundled: boolean } | null>(null);
    let installing = $state(false);
    let msg = $state("");

    onMount(refresh);

    async function refresh() {
        status = await invoke("cli_status");
    }

    async function install() {
        installing = true;
        msg = "";
        try {
            const path = await invoke<string>("cli_install");
            msg = `Installed to ${path}`;
            await refresh();
        } catch (e: any) {
            msg = String(e);
        } finally {
            installing = false;
        }
    }
</script>

<div class="page">
    <h3>CLI Tool</h3>

    <div class="status-card">
        {#if status === null}
            <p>Loading...</p>
        {:else if status.installed}
            <div class="badge installed">Installed</div>
            <p class="path">{status.path}</p>
            <button class="btn btn-sm" onclick={install} disabled={installing || !status.bundled}>
                {installing ? "Reinstalling..." : "Reinstall / Update"}
            </button>
        {:else}
            <div class="badge not-installed">Not installed</div>
            {#if status.bundled}
                <p class="hint">Click to install <code>rssh</code> to your system PATH. Admin password required.</p>
                <button class="btn btn-accent btn-sm" onclick={install} disabled={installing}>
                    {installing ? "Installing..." : "Install CLI"}
                </button>
            {:else}
                <p class="hint">CLI binary not bundled in dev build. Build manually:</p>
                <pre class="code-block">cargo build --release --features cli --bin rssh-cli</pre>
            {/if}
        {/if}
        {#if msg}
            <p class="msg">{msg}</p>
        {/if}
    </div>

    <h3>Shell Completions</h3>
    <p class="hint">Completions are auto-configured during install. To manually set up for a different shell:</p>
    <pre class="code-block">rssh completions zsh &gt; ~/.zsh/completions/_rssh
rssh completions bash &gt;&gt; ~/.bashrc
rssh completions fish &gt; ~/.config/fish/completions/rssh.fish
rssh completions powershell  # paste into $PROFILE</pre>

    <h3>Commands</h3>
    <table class="cmd-table">
        <tbody>
        <tr><td class="cmd">rssh</td><td>List all profiles (default)</td></tr>
        <tr><td class="cmd">rssh ls [query]</td><td>Search profiles by name/host</td></tr>
        <tr><td class="cmd">rssh ls cred</td><td>List credentials</td></tr>
        <tr><td class="cmd">rssh ls fwd</td><td>List port forwards</td></tr>
        <tr class="sep"><td></td><td></td></tr>
        <tr><td class="cmd">rssh open &lt;name&gt;</td><td>SSH connect to a profile</td></tr>
        <tr><td class="cmd">rssh open fwd &lt;name&gt;</td><td>Start a port forward</td></tr>
        <tr class="sep"><td></td><td></td></tr>
        <tr><td class="cmd">rssh add profile</td><td>Create profile (interactive)</td></tr>
        <tr><td class="cmd">rssh add cred</td><td>Create credential (interactive)</td></tr>
        <tr><td class="cmd">rssh add fwd</td><td>Create forward (interactive)</td></tr>
        <tr class="sep"><td></td><td></td></tr>
        <tr><td class="cmd">rssh edit profile &lt;name&gt;</td><td>Edit profile</td></tr>
        <tr><td class="cmd">rssh edit cred &lt;name&gt;</td><td>Edit credential</td></tr>
        <tr><td class="cmd">rssh edit fwd &lt;name&gt;</td><td>Edit forward</td></tr>
        <tr class="sep"><td></td><td></td></tr>
        <tr><td class="cmd">rssh rm profile &lt;name&gt;</td><td>Delete profile</td></tr>
        <tr><td class="cmd">rssh rm cred &lt;name&gt;</td><td>Delete credential</td></tr>
        <tr><td class="cmd">rssh rm fwd &lt;name&gt;</td><td>Delete forward</td></tr>
        <tr class="sep"><td></td><td></td></tr>
        <tr><td class="cmd">rssh config export &lt;file&gt;</td><td>Encrypted backup to file</td></tr>
        <tr><td class="cmd">rssh config import &lt;file&gt;</td><td>Restore from encrypted file</td></tr>
        <tr><td class="cmd">rssh config set</td><td>Configure GitHub sync</td></tr>
        <tr><td class="cmd">rssh config push</td><td>Push config to GitHub</td></tr>
        <tr><td class="cmd">rssh config pull</td><td>Pull config from GitHub</td></tr>
        </tbody>
    </table>
</div>

<style>
    .page {
        padding: 24px;
        display: flex;
        flex-direction: column;
        gap: 16px;
    }
    h3 {
        font-size: 15px;
        font-weight: 700;
        color: var(--text);
        margin: 0;
    }
    .status-card {
        display: flex;
        flex-direction: column;
        gap: 8px;
        padding: 16px;
        border-radius: var(--radius);
        background: var(--surface);
    }
    .badge {
        display: inline-block;
        padding: 2px 10px;
        border-radius: 12px;
        font-size: 12px;
        font-weight: 600;
        width: fit-content;
    }
    .badge.installed { background: rgba(76,184,138,0.15); color: #4cb88a; }
    .badge.not-installed { background: rgba(224,85,85,0.15); color: #e05555; }
    .path { font-family: monospace; font-size: 12px; color: var(--text-sub); }
    .hint { font-size: 13px; color: var(--text-sub); }
    .msg { font-size: 12px; color: var(--accent); }
    .code-block {
        font-family: monospace;
        font-size: 12px;
        background: var(--surface);
        padding: 12px;
        border-radius: var(--radius-sm);
        overflow-x: auto;
        white-space: pre;
        color: var(--text-sub);
    }
    .cmd-table {
        width: 100%;
        border-collapse: collapse;
        font-size: 13px;
    }
    .cmd-table td {
        padding: 4px 0;
        vertical-align: top;
    }
    .cmd-table .cmd {
        font-family: monospace;
        font-size: 12px;
        color: var(--accent);
        white-space: nowrap;
        padding-right: 20px;
    }
    .cmd-table .sep td {
        height: 6px;
    }
</style>
