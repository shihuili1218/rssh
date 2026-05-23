<script lang="ts">
    import { onMount } from "svelte";
    import * as ai from "./store.svelte.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";
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
    let terminating = $state(false);

    let isPending = $derived(!result && !rejected);
    let isPatch = $derived(cmd.kind === "patch_file");

    // 危险模式：每次新 command 进 chat 会创建一个新的 CommandConfirmDialog 实例，
    // onMount 触发一次自动 approve。判断在前端，UI 上"提议→执行"全程仍可见，
    // 审计 trail 完整；后端 emit 流程不变。挂载时若已有 result/rejected（历史记录回放）
    // 自然跳过。
    //
    // 重入防御：组件可能被销毁重建（panel close/reopen、chat list 重新 key 等）。
    // 重建实例的 executing=false，单看 executing 拦不住第二次 approve —— 同一 tool_call_id
    // 会被粘到 PTY 两次（rm/reboot 双执行级别的灾难）。用全局 _runningExecutions 表
    // （isCommandRunning）守门：命令还在 in-flight 时拒绝再次自动批准。
    //
    // patch_file 永远走人审：文件改动的代价比 run_command 高，danger_mode 是"接受
    // 命令风险"，不等于"接受任意文件改动"。强制用户看 diff 是这一类工具的契约。
    onMount(() => {
        if (
            isPending
            && !executing
            && !isPatch
            && !ai.isCommandRunning(cmd.tool_call_id)
            && ai.settings()?.danger_mode
        ) {
            void approve();
        }
    });

    async function approve() {
        if (executing) return;
        executing = true;
        try {
            await ai.executeCommand(sessionId, cmd, targetKind, targetSessionId);
        } catch (e) {
            console.error("[ai] execute failed:", e);
            alert(t("ai.cmd.alert.exec_failed", { error: errMsg(e) }));
        } finally {
            executing = false;
            terminating = false;
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

    /** 执行中点的"提前终止"：发 Ctrl+C；后续 finish() 会上报 early_terminated=true。 */
    async function terminate() {
        if (terminating) return;
        terminating = true;
        try {
            await ai.terminateCommand(cmd.tool_call_id);
        } catch (e) {
            console.error("[ai] terminate failed:", e);
            terminating = false;
        }
    }
</script>

<div class="cmd-card surface-flat" class:pending={isPending} class:done={!!result} class:rejected={!!rejected} class:patch={isPatch}>
    <div class="head">
        <span class="tag" class:patch-tag={isPatch}>
            {isPatch ? t("ai.cmd.patch.tag") : t("ai.cmd.proposed.tag")}
        </span>
        <code class="cmd">{cmd.cmd}</code>
    </div>
    <div class="meta">
        <div><span class="label">{t("ai.cmd.label.explain")}</span><span>{cmd.explain}</span></div>
        <div><span class="label">{t("ai.cmd.label.side_effect")}</span><span>{cmd.side_effect}</span></div>
        <div><span class="label">{t("ai.cmd.label.timeout")}</span><span>{cmd.timeout_s}s</span></div>
    </div>

    {#if isPatch && cmd.diff}
        <!-- 注意：span 是 display:block，自然换行。`<pre>` + `white-space:pre` 会把任何模板里的
             字面换行/缩进当真空白渲染，所以 span 之间不能有任何 whitespace，否则 diff 每行后会出现
             多余空行。整段写在一行内，闭合标签紧贴下一个开始标签。 -->
        <pre class="diff">{#each cmd.diff.split("\n") as line, i (i)}<span class="diff-line {line.startsWith('+') && !line.startsWith('+++') ? 'add' : line.startsWith('-') && !line.startsWith('---') ? 'del' : line.startsWith('@@') ? 'hunk' : line.startsWith('+++') || line.startsWith('---') ? 'file' : 'ctx'}">{line}</span>{/each}</pre>
    {/if}

    {#if isPending}
        {#if !askingReason}
            <div class="actions">
                <button class="btn btn-approve" onclick={approve} disabled={executing}>
                    {executing ? t("ai.cmd.btn.executing") : t("ai.cmd.btn.approve")}
                </button>
                {#if executing}
                    <button class="btn btn-terminate" onclick={terminate} disabled={terminating}>
                        {terminating ? t("ai.cmd.btn.terminating") : t("ai.cmd.btn.terminate")}
                    </button>
                {:else}
                    <button class="btn btn-reject" onclick={reject}>{t("ai.cmd.btn.reject")}</button>
                {/if}
            </div>
            {#if executing}
                <div class="hint">{t("ai.cmd.hint.executing")}</div>
            {/if}
        {:else}
            <div class="reject-form">
                <input
                    bind:value={rejectReason}
                    placeholder={t("ai.cmd.reject.placeholder")}
                    onkeydown={(e) => { if (e.key === "Enter") reject(); }}
                />
                <button class="btn" onclick={reject} disabled={!rejectReason.trim()}>{t("ai.cmd.reject.submit")}</button>
                <button class="btn btn-ghost" onclick={() => (askingReason = false)}>{t("ai.cmd.reject.cancel")}</button>
            </div>
        {/if}
    {:else if rejected}
        <div class="rejected-note">{t("ai.cmd.rejected_note", { reason: rejected.reason })}</div>
    {:else if result}
        <div class="result">
            <div class="result-meta">
                <span>exit={result.exit_code}</span>
                <span>{result.duration_ms}ms</span>
                {#if result.timed_out}<span class="warn">{t("ai.cmd.warn.timed_out")}</span>{/if}
                {#if result.early_terminated}<span class="warn">{t("ai.cmd.warn.early_terminated")}</span>{/if}
                {#if result.truncated_bytes > 0}<span class="warn">{t("ai.cmd.warn.truncated", { bytes: result.truncated_bytes })}</span>{/if}
            </div>
            <pre class="output">{result.output || t("ai.cmd.empty_output")}</pre>
        </div>
    {/if}
</div>

<style>
    .cmd-card {
        border: 1px solid var(--divider);
        border-radius: 6px;
        padding: calc(8px * var(--density)) calc(10px * var(--density));
        margin: calc(4px * var(--density)) 0;
        background: var(--bg);
    }
    .cmd-card.pending {
        border-left: 3px solid var(--warning);
        background: color-mix(in srgb, var(--warning) 6%, var(--bg));
    }
    .cmd-card.done { border-left: 3px solid var(--success); }
    .cmd-card.rejected { opacity: 0.6; border-left: 3px solid var(--text-dim); }
    .cmd-card.patch.pending {
        border-left: 3px solid var(--accent);
        background: color-mix(in srgb, var(--accent) 4%, var(--bg));
    }

    .patch-tag {
        background: var(--accent);
        color: var(--white);
    }
    .diff {
        margin-top: 6px;
        padding: 6px 8px;
        background: color-mix(in srgb, var(--text) 5%, var(--bg));
        border-radius: 4px;
        font-family: monospace;
        font-size: 11.5px;
        max-height: 360px;
        overflow: auto;
        white-space: pre;
        line-height: 1.35;
    }
    .diff-line { display: block; }
    .diff-line.add { background: color-mix(in srgb, var(--success) 18%, transparent); color: var(--success); }
    .diff-line.del { background: color-mix(in srgb, var(--error) 18%, transparent); color: var(--error); }
    .diff-line.hunk { color: var(--text-dim); font-weight: 600; }
    .diff-line.file { color: var(--text-dim); }
    .diff-line.ctx { color: var(--text); }

    .head { display: flex; gap: 8px; align-items: baseline; }
    .tag {
        font-size: 11px;
        background: var(--warning);
        color: var(--black);
        padding: 1px 6px;
        border-radius: 3px;
        font-weight: 600;
    }
    .cmd {
        font-family: monospace;
        font-size: 13px;
        word-break: break-all;
    }
    .meta { font-size: 12px; margin-top: 6px; color: var(--text-dim); }
    .meta > div { display: flex; gap: 8px; }
    .label { min-width: 50px; color: var(--text-dim); }
    .actions { margin-top: 8px; display: flex; gap: 8px; }
    .btn { padding: 4px 12px; border-radius: 4px; cursor: pointer; }
    .btn-approve { background: var(--success); color: var(--white); border: none; }
    .btn-reject { background: transparent; border: 1px solid var(--text-dim); color: var(--text); }
    .btn-terminate {
        background: var(--warning);
        color: var(--black);
        border: none;
    }
    .btn-terminate:disabled { opacity: 0.6; cursor: default; }
    .btn-ghost { background: transparent; border: 1px solid var(--divider); color: var(--text); }
    .reject-form { margin-top: 8px; display: flex; gap: 6px; }
    .reject-form input {
        flex: 1; padding: 4px 8px; border: 1px solid var(--divider);
        border-radius: 4px; background: var(--bg); color: var(--text);
    }
    .rejected-note { font-size: 12px; margin-top: 6px; color: var(--text-dim); }
    .hint { font-size: 11px; color: var(--text-dim); margin-top: 4px; font-style: italic; }
    .result { margin-top: 8px; }
    .result-meta { display: flex; gap: 8px; font-size: 11px; color: var(--text-dim); }
    .result-meta .warn { color: var(--warning); }
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
