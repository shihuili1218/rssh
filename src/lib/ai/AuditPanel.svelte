<script lang="ts">
    import { onMount } from "svelte";
    import * as ai from "./store.svelte.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import type { AuditEntry, AuditLog } from "./types.ts";

    let { sessionId } = $props<{ sessionId: string }>();

    let log = $state<AuditLog | null>(null);
    let loading = $state(false);

    async function refresh() {
        loading = true;
        try {
            log = await ai.getAudit(sessionId);
        } finally {
            loading = false;
        }
    }

    async function saveToFile() {
        try {
            const path = await ai.saveAuditWithDialog(sessionId);
            if (path) alert(t("ai.audit.alert.saved", { path }));
        } catch (e) {
            alert(t("ai.audit.alert.save_failed", { error: errMsg(e) }));
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
            case "command_executed": {
                const trunc = k.truncated_bytes > 0
                    ? t("ai.audit.summary.command_executed_truncated", { bytes: k.truncated_bytes })
                    : "";
                return t("ai.audit.summary.command_executed", {
                    id: k.id.slice(0, 8), exit: k.exit_code, dur: k.duration_ms, trunc,
                });
            }
            case "download_proposed": return t("ai.audit.summary.download_proposed", { path: k.remote_path });
            case "download_completed": return t("ai.audit.summary.download_completed", { path: k.local_path, bytes: k.bytes });
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
    .placeholder { color: var(--text-dim, #888); text-align: center; padding: 24px; }
    .audit-entry {
        font-size: 12px; padding: 6px 0;
        border-bottom: 1px solid color-mix(in srgb, var(--divider) 50%, transparent);
    }
    .ts { color: var(--text-dim, #888); margin-right: 8px; font-family: monospace; }
    .text { word-break: break-word; }
    .dropdown { margin-top: 4px; font-size: 11px; }
    .dropdown summary { cursor: pointer; color: var(--text-dim, #888); }
    .dropdown pre {
        max-height: 200px; overflow: auto;
        background: color-mix(in srgb, var(--text) 5%, var(--bg));
        padding: 6px 8px; border-radius: 4px;
        font-size: 11px; white-space: pre-wrap;
    }
    .cmd-detail { margin-top: 4px; padding-left: 8px; color: var(--text-dim, #888); font-size: 11px; }
</style>
