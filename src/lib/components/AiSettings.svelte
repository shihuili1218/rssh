<script lang="ts">
    import { onMount } from "svelte";
    import * as ai from "../ai/store.svelte.ts";
    import type { LlmProvider, SkillRecord } from "../ai/types.ts";

    // ─── BYOK ─────────────────────────────────────────────────
    let provider = $state<LlmProvider>("anthropic");
    let model = $state("");
    let endpoint = $state("");
    let apiKey = $state("");
    let hasKey = $state(false);
    let position = $state<"left" | "right">(ai.position());
    let savingByok = $state(false);
    let byokNote = $state<string | null>(null);

    const DEFAULTS = {
        anthropic: { model: "claude-sonnet-4-6" },
        openai: { model: "gpt-4o-mini" },
    };

    // ─── Skill 管理 ────────────────────────────────────────────
    let skills = $state<SkillRecord[]>([]);
    let editing = $state<SkillRecord | null>(null);
    let isNew = $state(false);
    let savingSkill = $state(false);
    let skillNote = $state<string | null>(null);

    onMount(async () => {
        const s = await ai.loadSettings();
        provider = s.provider as LlmProvider;
        model = s.model;
        endpoint = s.endpoint ?? "";
        hasKey = s.has_api_key;
        await refreshSkills();
    });

    async function refreshSkills() {
        try {
            skills = await ai.listSkills();
        } catch (e) {
            skillNote = `加载失败: ${e}`;
        }
    }

    async function saveByok() {
        savingByok = true;
        byokNote = null;
        try {
            await ai.saveSettings({
                provider,
                model,
                endpoint: endpoint.trim() || null,
                apiKey: apiKey || null,
            });
            const s = await ai.loadSettings();
            hasKey = s.has_api_key;
            apiKey = "";
            byokNote = "已保存";
            setTimeout(() => (byokNote = null), 2000);
        } catch (e) {
            byokNote = `保存失败: ${e}`;
        } finally {
            savingByok = false;
        }
    }

    $effect(() => {
        const d = DEFAULTS[provider];
        if (!model) model = d.model;
    });

    function setPos(p: "left" | "right") {
        position = p;
        ai.setPosition(p);
    }

    function newSkill() {
        editing = {
            id: "user-" + crypto.randomUUID().slice(0, 8),
            name: "",
            description: "",
            content: "",
            builtin: false,
        };
        isNew = true;
        skillNote = null;
    }

    function viewSkill(s: SkillRecord) {
        editing = { ...s };
        isNew = false;
        skillNote = null;
    }

    function cancelEdit() {
        editing = null;
        isNew = false;
        skillNote = null;
    }

    async function saveSkill() {
        if (!editing) return;
        if (editing.builtin) {
            skillNote = "内置 skill 不可保存";
            return;
        }
        if (!editing.name.trim() || !editing.content.trim() || !editing.id.trim()) {
            skillNote = "id / 名称 / 内容不能为空";
            return;
        }
        savingSkill = true;
        skillNote = null;
        try {
            await ai.saveSkill({
                id: editing.id.trim(),
                name: editing.name.trim(),
                description: editing.description.trim(),
                content: editing.content,
            });
            editing = null;
            isNew = false;
            await refreshSkills();
        } catch (e) {
            skillNote = `保存失败: ${e}`;
        } finally {
            savingSkill = false;
        }
    }

    async function removeSkill(s: SkillRecord) {
        if (s.builtin) return;
        if (!confirm(`确认删除 skill "${s.name}"？`)) return;
        try {
            await ai.deleteSkill(s.id);
            await refreshSkills();
        } catch (e) {
            skillNote = `删除失败: ${e}`;
        }
    }
</script>

<div class="page">
    <div class="warn">
        ⚠️ BYOK：你的命令输出经本地脱敏后会发送到所选 LLM 提供方。提供方可能按其条款使用这些数据
        （<a href="https://www.anthropic.com/legal/privacy" target="_blank" rel="noopener">Anthropic</a>
         / <a href="https://openai.com/policies/privacy-policy/" target="_blank" rel="noopener">OpenAI</a>）。
    </div>

    <div class="section-label">PROVIDER & MODEL</div>
    <div class="form">
        <div class="row">
            <label for="ai-provider">PROVIDER</label>
            <select id="ai-provider" bind:value={provider}>
                <option value="anthropic">Anthropic (Claude)</option>
                <option value="openai">OpenAI 兼容端点</option>
            </select>
        </div>
        <div class="row">
            <label for="ai-model">MODEL</label>
            <input id="ai-model" type="text" bind:value={model} placeholder={DEFAULTS[provider].model}/>
        </div>
        <div class="row">
            <label for="ai-endpoint">ENDPOINT</label>
            <input id="ai-endpoint" type="text" bind:value={endpoint} placeholder="留空使用官方默认"/>
        </div>
        <div class="row">
            <label for="ai-apikey">API KEY</label>
            <input id="ai-apikey" type="password" bind:value={apiKey}
                   placeholder={hasKey ? "已配置（留空保留）" : "未配置 — 在此粘贴"}/>
        </div>
        <div class="actions">
            <button class="btn btn-accent btn-sm" onclick={saveByok} disabled={savingByok}>
                {savingByok ? "保存中…" : "保存"}
            </button>
            {#if byokNote}<span class="note">{byokNote}</span>{/if}
        </div>
    </div>

    <div class="section-label">PANEL POSITION</div>
    <div class="segmented">
        <button class="seg-btn" class:active={position === "left"} onclick={() => setPos("left")}>左侧</button>
        <button class="seg-btn" class:active={position === "right"} onclick={() => setPos("right")}>右侧</button>
    </div>

    <div class="section-label skill-header">
        SKILLS
        {#if !editing}
            <button class="btn btn-sm" onclick={newSkill}>+ 新建</button>
        {/if}
    </div>

    {#if skillNote}
        <div class="banner">{skillNote} <button class="banner-close" onclick={() => (skillNote = null)} aria-label="关闭">×</button></div>
    {/if}

    {#if !editing}
        <div class="skill-list">
            {#each skills as s (s.id)}
                <button class="skill-item neu-sm" onclick={() => viewSkill(s)}>
                    <div class="skill-row">
                        <span class="skill-name">{s.name}</span>
                        <span class="skill-tag" class:builtin={s.builtin} class:user={!s.builtin}>
                            {s.builtin ? "内置" : "自定义"}
                        </span>
                        <span class="skill-id">{s.id}</span>
                    </div>
                    {#if s.description}<div class="skill-desc">{s.description}</div>{/if}
                </button>
            {/each}
            {#if skills.length === 0}
                <div class="placeholder">还没有 skill — 点上方"+ 新建"创建。</div>
            {/if}
        </div>
    {:else}
        <div class="form">
            <div class="row">
                <label for="sk-id">ID</label>
                <input id="sk-id" type="text" bind:value={editing.id}
                       disabled={!isNew || editing.builtin}/>
            </div>
            <div class="row">
                <label for="sk-name">NAME</label>
                <input id="sk-name" type="text" bind:value={editing.name} disabled={editing.builtin}
                       placeholder="如：CPU 高 — Python"/>
            </div>
            <div class="row">
                <label for="sk-desc">DESCRIPTION</label>
                <input id="sk-desc" type="text" bind:value={editing.description} disabled={editing.builtin}
                       placeholder="一句话说明用途"/>
            </div>
            <div class="row vert">
                <label for="sk-content">SYSTEM PROMPT</label>
                <textarea id="sk-content" bind:value={editing.content} disabled={editing.builtin}
                          rows="20"
                          placeholder="告诉 LLM：场景、可用工具、铁律、工作流……"></textarea>
            </div>
            <div class="actions">
                {#if !editing.builtin}
                    <button class="btn btn-accent btn-sm" onclick={saveSkill} disabled={savingSkill}>
                        {savingSkill ? "保存中…" : "保存"}
                    </button>
                {/if}
                {#if !editing.builtin && !isNew}
                    <button class="btn btn-sm btn-danger" onclick={() => editing && removeSkill(editing)}>删除</button>
                {/if}
                <button class="btn btn-sm" onclick={cancelEdit}>{editing.builtin ? "返回" : "取消"}</button>
            </div>
        </div>
    {/if}
</div>

<style>
    .page {
        padding: 24px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }

    .warn {
        background: color-mix(in srgb, #d9b341 12%, var(--bg));
        border-left: 3px solid #d9b341;
        padding: 8px 12px;
        border-radius: 4px;
        font-size: 12px;
        color: var(--text-sub);
        line-height: 1.5;
    }
    .warn a { color: var(--accent); }

    .form {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .row {
        display: flex;
        align-items: center;
        gap: 12px;
    }
    .row.vert {
        flex-direction: column;
        align-items: stretch;
    }
    .row label {
        min-width: 120px;
        text-align: right;
    }
    .row.vert label {
        text-align: left;
        min-width: 0;
    }
    .row input[type="text"],
    .row input[type="password"],
    .row select,
    .row textarea {
        flex: 1;
    }
    .row textarea {
        font-family: monospace;
        font-size: 12px;
        resize: vertical;
        min-height: 240px;
    }

    .actions {
        display: flex;
        gap: 8px;
        align-items: center;
        margin-top: 4px;
    }
    .note {
        font-size: 12px;
        color: var(--accent);
    }

    .segmented {
        display: inline-flex;
        border: 1px solid var(--divider);
        border-radius: var(--radius-sm);
        overflow: hidden;
        width: fit-content;
    }
    .seg-btn {
        padding: 8px 18px;
        border: none;
        background: var(--bg);
        color: var(--text-sub);
        font-family: inherit;
        font-size: 13px;
        cursor: pointer;
        border-right: 1px solid var(--divider);
    }
    .seg-btn:last-child { border-right: none; }
    .seg-btn:hover:not(.active) {
        background: var(--surface);
        color: var(--text);
    }
    .seg-btn.active {
        background: var(--accent);
        color: #fff;
        font-weight: 600;
    }

    .skill-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding-right: 0;
    }

    .banner {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 6px 12px;
        background: color-mix(in srgb, var(--error, #c0392b) 12%, var(--bg));
        color: var(--error, #c0392b);
        border-radius: 4px;
        font-size: 12px;
    }
    .banner-close {
        margin-left: auto;
        background: transparent;
        border: none;
        color: inherit;
        font-size: 14px;
        cursor: pointer;
    }

    .skill-list {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .skill-item {
        text-align: left;
        padding: 10px 14px;
        border: none;
        background: var(--bg);
        cursor: pointer;
        font-family: inherit;
        color: var(--text);
        transition: box-shadow 0.13s;
    }
    .skill-item:hover { box-shadow: var(--raised); }
    .skill-row {
        display: flex;
        align-items: baseline;
        gap: 8px;
    }
    .skill-name {
        font-weight: 600;
        font-size: 13px;
    }
    .skill-tag {
        font-size: 10px;
        padding: 1px 6px;
        border-radius: 3px;
        font-weight: 500;
    }
    .skill-tag.builtin {
        background: var(--surface);
        color: var(--text-dim);
    }
    .skill-tag.user {
        background: var(--accent-soft);
        color: var(--accent);
    }
    .skill-id {
        font-family: monospace;
        font-size: 11px;
        color: var(--text-dim);
        margin-left: auto;
    }
    .skill-desc {
        font-size: 12px;
        color: var(--text-sub);
        margin-top: 4px;
    }

    .placeholder {
        text-align: center;
        padding: 24px;
        color: var(--text-dim);
        font-size: 13px;
    }
</style>
