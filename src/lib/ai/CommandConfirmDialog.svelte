<script lang="ts" module>
    /** ack-only 工具（download_file / analyze_locally）不走 PTY，
     *  store 的 _runningExecutions 表登记的是 PTY 句柄，无法守门重入。
     *  Dialog 实例可能销毁重建（panel close/reopen），且 result prop 在事件
     *  抵达前是 undefined → isPending=true，会再次自动 approve 发重复 ack。
     *
     *  Set 必须在 `<script module>` 里 —— 写在 instance script 里每次组件 mount
     *  都会新建一个，完全不能跨实例共享，等于没防护。
     *  生命周期：invoke 成功后才 add；result/rejected 抵达由 $effect 清理。 */
    const _ackedToolCalls = new Set<string>();
</script>

<script lang="ts">
    import { onMount } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import * as ai from "./store.svelte.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import type { AiSettings, AiTargetKind, CommandKind, CommandProposed, CommandResult } from "./types.ts";

    let { tabId, targetKind, targetSessionId, cmd, result, rejected } = $props<{
        tabId: string;
        targetKind: AiTargetKind;
        targetSessionId: string;
        cmd: CommandProposed;
        result?: CommandResult;
        rejected?: { reason: string };
    }>();

    let askingReason = $state(false);
    let rejectReason = $state("");
    let executing = $state(false);
    let terminating = $state(false);
    // Serial only: the "submit output" button (distinct from terminate) is in flight.
    let submitting = $state(false);

    let isPending = $derived(!result && !rejected);
    // patch 卡片视觉特化（accent 高亮 + diff 框）—— 4 个阶段任一都算
    let isPatch = $derived(
        cmd.kind === "patch_cp"
        || cmd.kind === "patch_modify"
        || cmd.kind === "patch_diff"
        || cmd.kind === "patch_mv"
    );
    // download_file / analyze_locally 不走 PTY，approve 只发 ack 给后端；视觉无需特化。
    let isAckOnly = $derived(cmd.kind === "download_file" || cmd.kind === "analyze_locally");

    /** 把 kind 映射到 settings 上对应的 auto_* 字段 —— 命中即可自动批准。
     *  default 分支用 `never` 哨兵：CommandKind 新增联合成员时这里类型推断会失败、
     *  编译报错提醒补 case；同时运行时 fail-closed（type-violation 入参也不放过）。 */
    function autoApproveAllowed(s: AiSettings | null, kind?: CommandKind): boolean {
        if (!s || !s.danger_mode || !kind) return false;
        switch (kind) {
            case "run_command":     return s.auto_run_command;
            case "match_file":      return s.auto_match_file;
            case "download_file":   return s.auto_download_file;
            case "analyze_locally": return s.auto_analyze_locally;
            case "patch_cp":        return s.auto_patch_cp;
            case "patch_modify":    return s.auto_patch_modify;
            case "patch_diff":      return s.auto_patch_diff;
            case "patch_mv":        return s.auto_patch_mv;
            default: {
                const _exhaustive: never = kind;
                void _exhaustive;
                return false;
            }
        }
    }

    // 自动批准：每次新 command 进 chat 会创建一个新的 CommandConfirmDialog 实例，
    // onMount 触发一次按 kind 查 settings.auto_<kind>。UI 上"提议→执行"全程可见，
    // 审计 trail 完整；后端 emit 流程不变。挂载时若已有 result/rejected（历史记录
    // 回放）自然跳过。
    //
    // 重入防御：组件可能被销毁重建（panel close/reopen、chat list 重新 key 等）。
    // 重建实例的 executing=false，单看 executing 拦不住第二次 approve —— 同一 tool_call_id
    // 会被粘到 PTY 两次（rm/reboot 双执行级别的灾难）。用全局 _runningExecutions 表
    // （isCommandRunning）守门：命令还在 in-flight 时拒绝再次自动批准。
    //
    // 历史卡片没有 kind 字段 → autoApproveAllowed 返回 false → 走人审，符合 fail-safe。
    onMount(() => {
        // Command already in flight when this dialog (re)mounts — e.g. the AI
        // panel was closed and reopened mid-execution. Reflect the running state
        // so the card shows Terminate/Submit instead of a stale Approve button
        // (clicking which would be a no-op now that executeCommand guards on the
        // running map, but a dead button is confusing). The original execution
        // still owns the listener/timer and delivers the result.
        const inFlight = isAckOnly
            ? _ackedToolCalls.has(cmd.tool_call_id)
            : ai.isCommandRunning(cmd.tool_call_id);
        if (isPending && inFlight) {
            executing = true;
            return;
        }
        if (
            isPending
            && !executing
            // No danger mode on serial: a bare device (firmware / PLC / bootloader)
            // is too sensitive to auto-paste into. run_command always asks here.
            && targetKind !== "serial"
            && !ai.isCommandRunning(cmd.tool_call_id)
            && !_ackedToolCalls.has(cmd.tool_call_id)
            && autoApproveAllowed(ai.settings(), cmd.kind)
        ) {
            void approve();
        }
    });

    // result / rejected prop 抵达 → 把对应 tool_call_id 从 _ackedToolCalls 移除。
    // 此后该 dialog 再 remount 走的是 isPending=false 分支不会触发 approve，
    // Set 也不会无限增长。
    $effect(() => {
        if (result || rejected) {
            _ackedToolCalls.delete(cmd.tool_call_id);
        }
    });

    async function approve() {
        if (executing) return;
        if (isAckOnly && _ackedToolCalls.has(cmd.tool_call_id)) return;
        executing = true;
        try {
            if (isAckOnly) {
                // 不走 PTY：后端 actor 此刻阻塞在 wait_command_outcome 等批准结果。
                // 投一个 stub result 让它继续，由后端自己跑 SFTP / 开窗，跑完会
                // emit command_completed 把卡片切到结果态。executing 不在 finally 里
                // reset —— 让卡片维持"executing"视觉直到 result prop 抵达。
                //
                // 双重守门防 invoke 失败 stuck：
                // 1) 先 add 防 await 期间并发 onMount 撞重复 invoke
                // 2) catch 里 delete 回退，让用户能重试
                // 走到 return 之前 invoke 已 resolve，acked 状态留着到 result 抵达
                _ackedToolCalls.add(cmd.tool_call_id);
                try {
                    await invoke("ai_command_result", {
                        tabId,
                        toolCallId: cmd.tool_call_id,
                        exitCode: 0,
                        output: "",
                        timedOut: false,
                        earlyTerminated: false,
                    });
                } catch (e) {
                    _ackedToolCalls.delete(cmd.tool_call_id);
                    throw e;
                }
                return;
            }
            await ai.executeCommand(tabId, cmd, targetKind, targetSessionId);
        } catch (e) {
            console.error("[ai] execute failed:", e);
            alert(t("ai.cmd.alert.exec_failed", { error: errMsg(e) }));
            executing = false;
            terminating = false;
            submitting = false;
            return;
        }
        // 成功路径：ack-only 等 result 抵达再 reset；PTY 路径 executeCommand 已等到 result。
        if (!isAckOnly) {
            executing = false;
            terminating = false;
            submitting = false;
        }
    }

    async function reject() {
        if (!askingReason) {
            askingReason = true;
            return;
        }
        const reason = rejectReason.trim();
        if (!reason) return;
        await ai.rejectCommand(tabId, cmd.tool_call_id, reason);
        askingReason = false;
        rejectReason = "";
    }

    /** ssh/local 执行中点的"提前终止"：发 Ctrl+C；后续 finish() 上报 early_terminated=true。 */
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

    /**
     * Serial-only "submit output": the user watched the device finish responding.
     * Reports the accumulated buffer as a CLEAN result — no Ctrl+C (nothing to
     * interrupt), not early-terminated. A dedicated button, fully separate from
     * terminate, so neither action is overloaded onto the other.
     */
    async function submit() {
        if (submitting) return;
        submitting = true;
        try {
            await ai.submitCommand(cmd.tool_call_id);
        } catch (e) {
            // Match approve()'s feedback — otherwise a failed submit looks like a
            // dead button (user clicked, nothing happened, no clue why).
            console.error("[ai] submit failed:", e);
            alert(t("ai.cmd.alert.submit_failed", { error: errMsg(e) }));
            submitting = false;
        }
    }
</script>

<div class="cmd-card surface-flat" class:pending={isPending} class:done={!!result} class:rejected={!!rejected} class:patch={isPatch}>
    <div class="head">
        <span class="tag" class:patch-tag={isPatch}>
            {isPatch ? t("ai.cmd.patch.tag") : t("ai.cmd.proposed.tag")}
        </span>
        <code class="cmd" title={cmd.cmd}>{cmd.cmd}</code>
    </div>
    <div class="meta">
        <div><span class="label">{t("ai.cmd.label.explain")}</span><span class="val" title={cmd.explain}>{cmd.explain}</span></div>
        <div><span class="label">{t("ai.cmd.label.side_effect")}</span><span class="val" title={cmd.side_effect}>{cmd.side_effect}</span></div>
        <div><span class="label">{t("ai.cmd.label.timeout")}</span><span class="val">{cmd.timeout_s}s</span></div>
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
                {#if executing && !isAckOnly && targetKind === "serial"}
                    <!-- Serial: a dedicated "submit output" button, fully separate from
                         Terminate. The user clicks it when the device has finished
                         responding; it reports the buffer as a clean result. -->
                    <button class="btn btn-submit" onclick={submit} disabled={submitting}>
                        {submitting ? t("ai.cmd.btn.submitting") : t("ai.cmd.btn.submit")}
                    </button>
                {:else if executing && !isAckOnly}
                    <!-- ack-only 命令（download_file / analyze_locally）没 PTY，
                         Terminate 发 Ctrl+C 是 no-op，不该露给用户当 affordance。 -->
                    <button class="btn btn-terminate" onclick={terminate} disabled={terminating}>
                        {terminating ? t("ai.cmd.btn.terminating") : t("ai.cmd.btn.terminate")}
                    </button>
                {:else if !executing}
                    <button class="btn btn-reject" onclick={reject}>{t("ai.cmd.btn.reject")}</button>
                {/if}
            </div>
            {#if executing}
                <div class="hint">{targetKind === "serial" ? t("ai.cmd.hint.executing_serial") : t("ai.cmd.hint.executing")}</div>
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

    .head { display: flex; gap: 8px; align-items: center; }
    .tag {
        flex: none;
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
        flex: 1;
        min-width: 0;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .meta { font-size: 12px; margin-top: 6px; color: var(--text-dim); }
    .meta > div { display: flex; gap: 8px; }
    .label { flex: none; min-width: 50px; color: var(--text-dim); }
    .val {
        flex: 1;
        min-width: 0;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
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
    /* Serial "submit output" — a positive completion action, so green like approve
       (the two never co-occur: approve shows pre-exec, submit shows during exec). */
    .btn-submit { background: var(--success); color: var(--white); border: none; }
    .btn-submit:disabled { opacity: 0.6; cursor: default; }
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
