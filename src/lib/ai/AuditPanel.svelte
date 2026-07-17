<script lang="ts">
    import { onMount } from "svelte";
    import * as ai from "./store.svelte.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import { truncateCommand, formatBytes } from "./format.ts";
    import type { AuditEntry, AuditLog } from "./types.ts";
    import type { SessionInstanceRef } from "./session-identity.ts";
    import { toast } from "../stores/toast.svelte.ts";

    let { tabId } = $props<{ tabId: string }>();

    let log = $state<AuditLog | null>(null);
    let loading = $state(false);

    function currentSession(): SessionInstanceRef | null {
        const info = ai.sessionForTab(tabId);
        return info ? { tabId, instanceId: info.instance_id } : null;
    }

    async function refresh() {
        loading = true;
        const requested = currentSession();
        if (!requested) { loading = false; return; }
        try {
            const next = await ai.getAudit(requested);
            if (currentSession()?.instanceId === requested.instanceId) log = next;
        } catch (e) {
            // Closing/replacing the actor invalidates this request. Only that
            // lifecycle race is expected; a live actor's real failure is visible.
            if (currentSession()?.instanceId === requested.instanceId) {
                toast.error(errMsg(e));
            }
        } finally {
            loading = false;
        }
    }

    async function saveToFile() {
        const requested = currentSession();
        if (!requested) return;
        try {
            const path = await ai.saveAuditWithDialog(requested);
            if (path && currentSession()?.instanceId === requested.instanceId) {
                alert(t("ai.audit.alert.saved", { path }));
            }
        } catch (e) {
            if (currentSession()?.instanceId === requested.instanceId) {
                toast.error(errMsg(e));
            }
        }
    }

    onMount(refresh);

    function fmt(at: string) {
        return new Date(at).toLocaleTimeString();
    }

    function summary(e: AuditEntry): string {
        const k = e.kind;
        switch (k.type) {
            case "session_started": return t("ai.audit.summary.session_started", { skill: k.skill, target: k.target });
            case "session_ended": return t("ai.audit.summary.session_ended");
            case "llm_request": return t("ai.audit.summary.llm_request", { model: k.model });
            case "llm_response": return t("ai.audit.summary.llm_response", { tin: k.tokens_in ?? "?", tout: k.tokens_out ?? "?" });
            case "command_proposed": return t("ai.audit.summary.command_proposed", { cmd: k.cmd });
            case "command_rejected": return t("ai.audit.summary.command_rejected", { id: k.id.slice(0, 8), reason: k.reason });
            case "command_blocked": return t("ai.audit.summary.command_blocked", { cmd: truncateCommand(k.cmd), reason: k.reason });
            case "command_executed": {
                const trunc = k.truncated_bytes > 0
                    ? t("ai.audit.summary.command_executed_truncated", { bytes: k.truncated_bytes })
                    : "";
                return t("ai.audit.summary.command_executed", {
                    id: k.id.slice(0, 8), exit: k.exit_code, dur: k.duration_ms, trunc,
                });
            }
            case "download_proposed": return t("ai.audit.summary.download_proposed", { path: k.remote_path, max_mb: k.max_mb });
            case "download_completed": return t("ai.audit.summary.download_completed", { path: k.local_path, size: formatBytes(k.bytes) });
            case "analyze_proposed": return t("ai.audit.summary.analyze_proposed", { path: k.local_path, task: k.task });
            case "skill_loaded": return t("ai.audit.summary.skill_loaded", { name: k.name, id: k.id });
            case "note": return t("ai.audit.summary.note", { message: k.message });
            case "error": return t("ai.audit.summary.error", { message: k.message });
        }
    }
</script>

<div class="audit">
    <div class="audit-toolbar">
        <button onclick={refresh} disabled={loading}>{loading ? t("ai.audit.refresh_loading") : t("ai.audit.refresh")}</button>
        <button onclick={saveToFile} disabled={!log || log.entries.length === 0}>{t("ai.audit.save_to_file")}</button>
    </div>
    <div class="audit-list">
        {#if !log}
            <div class="placeholder">{t("ai.audit.placeholder.loading")}</div>
        {:else if log.entries.length === 0}
            <div class="placeholder">{t("ai.audit.placeholder.empty")}</div>
        {:else}
            {#each log.entries as entry, i (i)}
                <div class="audit-entry">
                    <span class="ts">{fmt(entry.at)}</span>
                    <span class="text">{summary(entry)}</span>
                    {#if entry.kind.type === "llm_request"}
                        <details class="dropdown">
                            <summary>{t("ai.audit.toggle.payload")}</summary>
                            <pre>{entry.kind.redacted_payload}</pre>
                        </details>
                    {:else if entry.kind.type === "llm_response"}
                        <details class="dropdown">
                            <summary>{t("ai.audit.toggle.response")}</summary>
                            <pre>{entry.kind.text}</pre>
                        </details>
                    {:else if entry.kind.type === "command_executed"}
                        <details class="dropdown">
                            <summary>{t("ai.audit.toggle.output", { bytes: entry.kind.original_bytes })}</summary>
                            <pre>{entry.kind.output_redacted}</pre>
                        </details>
                    {:else if entry.kind.type === "command_proposed"}
                        <div class="cmd-detail">
                            <div>{t("ai.audit.cmd.explain")}: {entry.kind.explain}</div>
                            <div>{t("ai.audit.cmd.side_effect")}: {entry.kind.side_effect}</div>
                        </div>
                    {/if}
                </div>
            {/each}
        {/if}
    </div>
</div>

<style>
    .audit { display: flex; flex-direction: column; height: 100%; overflow: hidden; }
    .audit-toolbar {
        display: flex; gap: 8px; padding: 8px;
        border-bottom: 1px solid var(--divider);
        flex-shrink: 0;
    }
    .audit-toolbar button {
        padding: 4px 12px; border: 1px solid var(--divider);
        border-radius: 4px; background: var(--bg); color: var(--text);
        cursor: pointer;
    }
    .audit-toolbar button:disabled { opacity: 0.5; cursor: not-allowed; }
    .audit-list { flex: 1; overflow-y: auto; padding: 8px; }
    .placeholder { color: var(--text-dim); text-align: center; padding: 24px; }
    .audit-entry {
        font-size: 12px; padding: 6px 0;
        border-bottom: 1px solid color-mix(in srgb, var(--divider) 50%, transparent);
    }
    .ts { color: var(--text-dim); margin-right: 8px; font-family: monospace; }
    .text { word-break: break-word; }
    .dropdown { margin-top: 4px; font-size: 11px; }
    .dropdown summary { cursor: pointer; color: var(--text-dim); }
    .dropdown pre {
        max-height: 200px; overflow: auto;
        background: color-mix(in srgb, var(--text) 5%, var(--bg));
        padding: 6px 8px; border-radius: 4px;
        font-size: 11px; white-space: pre-wrap;
    }
    .cmd-detail { margin-top: 4px; padding-left: 8px; color: var(--text-dim); font-size: 11px; }
</style>
