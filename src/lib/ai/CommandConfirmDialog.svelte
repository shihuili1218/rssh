<script lang="ts">
    import * as ai from "./store.svelte.ts";
    import type { CommandProposed, CommandResult } from "./types.ts";

    let { sessionId, targetKind, targetSessionId, cmd, result, rejected } = $props<{
        sessionId: string;
        targetKind: "ssh" | "local";
        targetSessionId: string;
        cmd: CommandProposed;
        result?: CommandResult;
        rejected?: { reason: string };
    }>();

    let askingReason = $state(false);
    let rejectReason = $state("");
    let executing = $state(false);

    let isPending = $derived(!result && !rejected);

    async function approve() {
        if (executing) return;
        executing = true;
        try {
            await ai.executeCommand(sessionId, cmd, targetKind, targetSessionId);
        } catch (e) {
            console.error("[ai] execute failed:", e);
            alert(`执行失败: ${e}`);
        } finally {
            executing = false;
        }
    }

    async function reject() {
        if (!askingReason) {
            askingReason = true;
            return;
        }
        const reason = rejectReason.trim();
        if (!reason) return;
        await ai.rejectCommand(sessionId, cmd.tool_call_id, reason);
        askingReason = false;
        rejectReason = "";
    }
</script>

<div class="cmd-card" class:pending={isPending} class:done={!!result} class:rejected={!!rejected}>
    <div class="head">
        <span class="tag">[AI proposed]</span>
        <code class="cmd">{cmd.cmd}</code>
    </div>
    <div class="meta">
        <div><span class="label">含义</span><span>{cmd.explain}</span></div>
        <div><span class="label">副作用</span><span>{cmd.side_effect}</span></div>
        <div><span class="label">超时</span><span>{cmd.timeout_s}s</span></div>
    </div>

    {#if isPending}
        {#if !askingReason}
            <div class="actions">
                <button class="btn btn-approve" onclick={approve} disabled={executing}>
                    {executing ? "⋯ 执行中" : "✓ 在终端执行"}
                </button>
                <button class="btn btn-reject" onclick={reject} disabled={executing}>✗ 拒绝</button>
            </div>
            {#if executing}
                <div class="hint">命令已粘贴并回车，正在等待终端输出 sentinel…</div>
            {/if}
        {:else}
            <div class="reject-form">
                <input
                    bind:value={rejectReason}
                    placeholder="拒绝理由（让 AI 调整方案）"
                    onkeydown={(e) => { if (e.key === "Enter") reject(); }}
                />
                <button class="btn" onclick={reject} disabled={!rejectReason.trim()}>提交</button>
                <button class="btn btn-ghost" onclick={() => (askingReason = false)}>取消</button>
            </div>
        {/if}
    {:else if rejected}
        <div class="rejected-note">已拒绝。理由: {rejected.reason}</div>
    {:else if result}
        <div class="result">
            <div class="result-meta">
                <span>exit={result.exit_code}</span>
                <span>{result.duration_ms}ms</span>
                {#if result.timed_out}<span class="warn">超时</span>{/if}
                {#if result.truncated_bytes > 0}<span class="warn">截断 {result.truncated_bytes}B</span>{/if}
            </div>
            <pre class="output">{result.output || "(空输出)"}</pre>
        </div>
    {/if}
</div>

<style>
    .cmd-card {
        border: 1px solid var(--divider);
        border-radius: 6px;
        padding: 8px 10px;
        margin: 4px 0;
        background: var(--bg);
    }
    .cmd-card.pending {
        border-left: 3px solid #d9b341;
        background: color-mix(in srgb, #d9b341 6%, var(--bg));
    }
    .cmd-card.done { border-left: 3px solid #4caf50; }
    .cmd-card.rejected { opacity: 0.6; border-left: 3px solid #888; }

    .head { display: flex; gap: 8px; align-items: baseline; }
    .tag {
        font-size: 11px;
        background: #d9b341;
        color: #000;
        padding: 1px 6px;
        border-radius: 3px;
        font-weight: 600;
    }
    .cmd {
        font-family: monospace;
        font-size: 13px;
        word-break: break-all;
    }
    .meta { font-size: 12px; margin-top: 6px; color: var(--text-dim, #888); }
    .meta > div { display: flex; gap: 8px; }
    .label { min-width: 50px; color: var(--text-dim, #888); }
    .actions { margin-top: 8px; display: flex; gap: 8px; }
    .btn { padding: 4px 12px; border-radius: 4px; cursor: pointer; }
    .btn-approve { background: #4caf50; color: #fff; border: none; }
    .btn-reject { background: transparent; border: 1px solid #888; color: var(--text); }
    .btn-ghost { background: transparent; border: 1px solid var(--divider); color: var(--text); }
    .reject-form { margin-top: 8px; display: flex; gap: 6px; }
    .reject-form input {
        flex: 1; padding: 4px 8px; border: 1px solid var(--divider);
        border-radius: 4px; background: var(--bg); color: var(--text);
    }
    .rejected-note { font-size: 12px; margin-top: 6px; color: #888; }
    .hint { font-size: 11px; color: var(--text-dim, #888); margin-top: 4px; font-style: italic; }
    .result { margin-top: 8px; }
    .result-meta { display: flex; gap: 8px; font-size: 11px; color: var(--text-dim, #888); }
    .result-meta .warn { color: #d9b341; }
    .output {
        margin-top: 4px;
        padding: 6px 8px;
        background: color-mix(in srgb, var(--text) 5%, var(--bg));
        border-radius: 4px;
        font-family: monospace;
        font-size: 12px;
        max-height: 240px;
        overflow: auto;
        white-space: pre-wrap;
        word-break: break-all;
    }
</style>
