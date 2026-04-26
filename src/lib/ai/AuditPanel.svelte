<script lang="ts">
    import { onMount } from "svelte";
    import * as ai from "./store.svelte.ts";
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
            if (path) alert(`已保存到 ${path}`);
        } catch (e) {
            alert(`保存失败: ${e}`);
        }
    }

    onMount(refresh);

    function fmt(at: string) {
        return new Date(at).toLocaleTimeString();
    }

    function summary(e: AuditEntry): string {
        const k = e.kind;
        switch (k.type) {
            case "session_started": return `会话开始 [${k.skill}] target=${k.target}`;
            case "session_ended": return "会话结束";
            case "llm_request": return `→ LLM (${k.model})`;
            case "llm_response": return `← LLM ${k.tokens_in ?? "?"}/${k.tokens_out ?? "?"} tokens`;
            case "command_proposed": return `提议命令: ${k.cmd}`;
            case "command_rejected": return `拒绝命令 ${k.id.slice(0, 8)}: ${k.reason}`;
            case "command_executed": return `执行 ${k.id.slice(0, 8)} exit=${k.exit_code} ${k.duration_ms}ms${k.truncated_bytes > 0 ? ` (截断 ${k.truncated_bytes}B)` : ""}`;
            case "download_proposed": return `提议下载: ${k.remote_path}`;
            case "download_completed": return `下载完成: ${k.local_path} (${k.bytes}B)`;
            case "note": return `备注: ${k.message}`;
            case "error": return `错误: ${k.message}`;
        }
    }
</script>

<div class="audit">
    <div class="audit-toolbar">
        <button onclick={refresh} disabled={loading}>{loading ? "加载…" : "🔄 刷新"}</button>
        <button onclick={saveToFile} disabled={!log || log.entries.length === 0}>💾 保存到文件</button>
    </div>
    <div class="audit-list">
        {#if !log}
            <div class="placeholder">加载中...</div>
        {:else if log.entries.length === 0}
            <div class="placeholder">暂无审计记录</div>
        {:else}
            {#each log.entries as entry, i (i)}
                <div class="audit-entry">
                    <span class="ts">{fmt(entry.at)}</span>
                    <span class="text">{summary(entry)}</span>
                    {#if entry.kind.type === "llm_request"}
                        <details class="dropdown">
                            <summary>查看 payload (脱敏后)</summary>
                            <pre>{entry.kind.redacted_payload}</pre>
                        </details>
                    {:else if entry.kind.type === "llm_response"}
                        <details class="dropdown">
                            <summary>查看响应文本</summary>
                            <pre>{entry.kind.text}</pre>
                        </details>
                    {:else if entry.kind.type === "command_executed"}
                        <details class="dropdown">
                            <summary>查看输出 (脱敏后, {entry.kind.original_bytes}B)</summary>
                            <pre>{entry.kind.output_redacted}</pre>
                        </details>
                    {:else if entry.kind.type === "command_proposed"}
                        <div class="cmd-detail">
                            <div>含义: {entry.kind.explain}</div>
                            <div>副作用: {entry.kind.side_effect}</div>
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
