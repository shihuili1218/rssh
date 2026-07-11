<script lang="ts">
    import {onMount} from "svelte";
    import {invoke} from "@tauri-apps/api/core";
    import { t, errMsg } from "../i18n/index.svelte.ts";

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
            msg = t("settings.cli.installed_to", { path });
            await refresh();
        } catch (e: any) {
            msg = errMsg(e);
        } finally {
            installing = false;
        }
    }
</script>

<div class="page">
    <h3>{t("settings.section.cli")}</h3>

    <div class="status-card">
        {#if status === null}
            <p>{t("common.loading")}</p>
        {:else if status.installed}
            <div class="badge installed">{t("settings.cli.installed")}</div>
            <p class="path">{status.path}</p>
            <button class="btn btn-sm" onclick={install} disabled={installing || !status.bundled}>
                {installing ? t("settings.cli.reinstalling") : t("settings.cli.reinstall")}
            </button>
        {:else}
            <div class="badge not-installed">{t("settings.cli.not_installed")}</div>
            {#if status.bundled}
                <p class="hint">{t("settings.cli.install_hint")}</p>
                <button class="btn btn-accent btn-sm" onclick={install} disabled={installing}>
                    {installing ? t("settings.cli.installing") : t("settings.cli.install")}
                </button>
            {:else}
                <p class="hint">{t("settings.cli.dev_hint")}</p>
                <pre class="code-block">cargo build --release --features cli --bin rssh-cli</pre>
            {/if}
        {/if}
        {#if msg}
            <p class="msg">{msg}</p>
        {/if}
    </div>

    <h3>{t("settings.cli.completions")}</h3>
    <p class="hint">{t("settings.cli.completions_hint")}</p>
    <pre class="code-block">rssh completions zsh &gt; ~/.zsh/completions/_rssh
rssh completions bash &gt;&gt; ~/.bashrc
rssh completions fish &gt; ~/.config/fish/completions/rssh.fish
rssh completions powershell  # paste into $PROFILE</pre>

    <h3>{t("settings.cli.commands")}</h3>
    <table class="cmd-table">
        <tbody>
        <tr><td class="cmd">rssh</td><td>{t("settings.cli.cmd.list")}</td></tr>
        <tr><td class="cmd">rssh profile list [query]</td><td>{t("settings.cli.cmd.ls_query")}</td></tr>
        <tr><td class="cmd">rssh credential list</td><td>{t("settings.cli.cmd.ls_cred")}</td></tr>
        <tr><td class="cmd">rssh forward list</td><td>{t("settings.cli.cmd.ls_fwd")}</td></tr>
        <tr><td class="cmd">rssh group list</td><td>{t("settings.cli.cmd.ls_group")}</td></tr>
        <tr class="sep"><td></td><td></td></tr>
        <tr><td class="cmd">rssh profile open &lt;name&gt;</td><td>{t("settings.cli.cmd.open")}</td></tr>
        <tr><td class="cmd">rssh forward open &lt;name&gt;</td><td>{t("settings.cli.cmd.open_fwd")}</td></tr>
        <tr class="sep"><td></td><td></td></tr>
        <tr><td class="cmd">rssh profile add</td><td>{t("settings.cli.cmd.add_profile")}</td></tr>
        <tr><td class="cmd">rssh credential add</td><td>{t("settings.cli.cmd.add_cred")}</td></tr>
        <tr><td class="cmd">rssh forward add</td><td>{t("settings.cli.cmd.add_fwd")}</td></tr>
        <tr><td class="cmd">rssh group add</td><td>{t("settings.cli.cmd.add_group")}</td></tr>
        <tr class="sep"><td></td><td></td></tr>
        <tr><td class="cmd">rssh profile edit &lt;name&gt;</td><td>{t("settings.cli.cmd.edit_profile")}</td></tr>
        <tr><td class="cmd">rssh credential edit &lt;name&gt;</td><td>{t("settings.cli.cmd.edit_cred")}</td></tr>
        <tr><td class="cmd">rssh forward edit &lt;name&gt;</td><td>{t("settings.cli.cmd.edit_fwd")}</td></tr>
        <tr><td class="cmd">rssh group edit &lt;name&gt;</td><td>{t("settings.cli.cmd.edit_group")}</td></tr>
        <tr class="sep"><td></td><td></td></tr>
        <tr><td class="cmd">rssh profile rm &lt;name&gt;</td><td>{t("settings.cli.cmd.rm_profile")}</td></tr>
        <tr><td class="cmd">rssh credential rm &lt;name&gt;</td><td>{t("settings.cli.cmd.rm_cred")}</td></tr>
        <tr><td class="cmd">rssh forward rm &lt;name&gt;</td><td>{t("settings.cli.cmd.rm_fwd")}</td></tr>
        <tr><td class="cmd">rssh group rm &lt;name&gt;</td><td>{t("settings.cli.cmd.rm_group")}</td></tr>
        <tr class="sep"><td></td><td></td></tr>
        <tr><td class="cmd">rssh config export &lt;file&gt;</td><td>{t("settings.cli.cmd.export")}</td></tr>
        <tr><td class="cmd">rssh config import &lt;file&gt;</td><td>{t("settings.cli.cmd.import")}</td></tr>
        <tr><td class="cmd">rssh config github set</td><td>{t("settings.cli.cmd.config_set")}</td></tr>
        <tr><td class="cmd">rssh config github push</td><td>{t("settings.cli.cmd.config_push")}</td></tr>
        <tr><td class="cmd">rssh config github pull</td><td>{t("settings.cli.cmd.config_pull")}</td></tr>
        <tr><td class="cmd">rssh config webdav set</td><td>{t("settings.cli.cmd.webdav_set")}</td></tr>
        <tr><td class="cmd">rssh config webdav push</td><td>{t("settings.cli.cmd.webdav_push")}</td></tr>
        <tr><td class="cmd">rssh config webdav pull</td><td>{t("settings.cli.cmd.webdav_pull")}</td></tr>
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
    .badge.installed { background: color-mix(in srgb, var(--success) 15%, transparent); color: var(--success); }
    .badge.not-installed { background: color-mix(in srgb, var(--error) 15%, transparent); color: var(--error); }
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
