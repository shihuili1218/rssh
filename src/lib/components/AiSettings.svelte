<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import * as ai from "../ai/store.svelte.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import type { LlmProvider, ModelInfo, SkillRecord } from "../ai/types.ts";

    function openExternal(e: MouseEvent, url: string) {
        e.preventDefault();
        invoke("open_external_url", { url }).catch(err =>
            console.error("open_external_url failed:", err)
        );
    }

    // ─── BYOK ─────────────────────────────────────────────────
    let provider = $state<LlmProvider>("anthropic");
    let model = $state("");
    let endpoint = $state("");
    let apiKey = $state("");
    let hasKey = $state(false);
    let savingByok = $state(false);
    let byokNote = $state<string | null>(null);

    // ─── Danger mode（全局，跟 provider 无关）────────────────────────
    let dangerMode = $state(false);
    let savingDanger = $state(false);
    let showDangerDialog = $state(false);
    let dangerNote = $state<string | null>(null);

    /** onclick + preventDefault：Tauri webview 不支持原生 confirm()，且依赖 checkbox
     *  默认 toggle 会导致 cancel 后 DOM 与 Svelte state 错位。这里手动接管：
     *  开启时弹自定义模态等用户确认；关闭直接 save（关 = 回到安全默认，不拦）。 */
    function handleDangerToggle(e: MouseEvent) {
        e.preventDefault();
        if (savingDanger) return;
        if (!dangerMode) {
            showDangerDialog = true;
            return;
        }
        void applyDangerMode(false);
    }
    async function applyDangerMode(wantOn: boolean) {
        savingDanger = true;
        showDangerDialog = false;
        dangerNote = null;
        try {
            await ai.saveSettings({ dangerMode: wantOn });
            dangerMode = wantOn;
        } catch (err) {
            dangerNote = t("ai.settings.danger.save_failed", { error: errMsg(err) });
        } finally {
            savingDanger = false;
        }
    }
    let modelOptions = $state<ModelInfo[]>([]);
    let loadingModels = $state(false);
    /** byokNote 自清 timer 句柄，避免后续动作被旧 timer 误清。 */
    let byokNoteTimer: number | null = null;
    /** 切 provider 的代际号：在途的 loadSettings 解到一半时如果代际过期，丢弃结果。 */
    let providerGen = 0;

    function setByokNote(msg: string | null, autoClearMs?: number) {
        if (byokNoteTimer !== null) {
            clearTimeout(byokNoteTimer);
            byokNoteTimer = null;
        }
        byokNote = msg;
        if (msg !== null && autoClearMs !== undefined) {
            byokNoteTimer = window.setTimeout(() => {
                byokNote = null;
                byokNoteTimer = null;
            }, autoClearMs);
        }
    }

    /**
     * 切换 provider：清空所有字段，从后端拉**该 provider** 已保存的快照回显。
     * 没存过 → 字段保持空。这是用户唯一显式触发数据替换的入口，不再用 $effect 做隐式同步。
     */
    async function onProviderChange() {
        const gen = ++providerGen;
        setByokNote(null);
        modelOptions = [];
        apiKey = "";
        model = "";
        endpoint = "";
        hasKey = false;
        const s = await ai.loadSettings(provider);
        if (gen !== providerGen) return; // 用户又切了，丢弃过期结果
        model = s.model;
        endpoint = s.endpoint ?? "";
        hasKey = s.has_api_key;
        if (hasKey) void autoLoadModels();
    }

    /** 静默拉取（失败不打扰）。供 onMount / 切换 provider / apiKey 失焦使用。 */
    async function autoLoadModels() {
        try {
            const list = await ai.listModels(
                provider,
                apiKey.trim() || undefined,
                endpoint.trim() || undefined,
            );
            modelOptions = list;
        } catch {
            // 没填 key、网络错等，不打扰用户。手动按钮会显示真实错误。
        }
    }

    /** 显式按钮：失败要给反馈。 */
    async function loadModels() {
        if (!apiKey.trim() && !hasKey) {
            setByokNote(t("ai.settings.note.api_key_required"));
            return;
        }
        loadingModels = true;
        setByokNote(null);
        try {
            const list = await ai.listModels(
                provider,
                apiKey.trim() || undefined,
                endpoint.trim() || undefined,
            );
            modelOptions = list;
            setByokNote(t("ai.settings.note.models_loaded", { count: list.length }), 2000);
        } catch (e) {
            setByokNote(t("ai.settings.note.models_failed", { error: errMsg(e) }));
        } finally {
            loadingModels = false;
        }
    }

    function onApiKeyBlur() {
        if (apiKey.trim()) void autoLoadModels();
    }

    // ─── Skill 管理 ────────────────────────────────────────────
    let skills = $state<SkillRecord[]>([]);
    let editing = $state<SkillRecord | null>(null);
    let isNew = $state(false);
    let savingSkill = $state(false);
    let skillNote = $state<string | null>(null);
    let confirmingDelete = $state(false);
    let confirmDeleteTimer: number | null = null;

    function resetDeleteConfirm() {
        confirmingDelete = false;
        if (confirmDeleteTimer !== null) {
            clearTimeout(confirmDeleteTimer);
            confirmDeleteTimer = null;
        }
    }

    onMount(async () => {
        const s = await ai.loadSettings();
        provider = s.provider as LlmProvider;
        model = s.model;
        endpoint = s.endpoint ?? "";
        hasKey = s.has_api_key;
        dangerMode = s.danger_mode;
        if (hasKey) void autoLoadModels();
        await refreshSkills();
    });

    onDestroy(() => {
        if (byokNoteTimer !== null) clearTimeout(byokNoteTimer);
        if (confirmDeleteTimer !== null) clearTimeout(confirmDeleteTimer);
    });

    async function refreshSkills() {
        try {
            skills = await ai.listSkills();
        } catch (e) {
            skillNote = t("ai.settings.skills.error.load_failed", { error: errMsg(e) });
        }
    }

    async function saveByok() {
        savingByok = true;
        setByokNote(null);
        try {
            await ai.saveSettings({
                provider,
                model: model.trim(),
                endpoint: endpoint.trim() || null,
                apiKey: apiKey.trim() || null,
            });
            const s = await ai.loadSettings();
            hasKey = s.has_api_key;
            apiKey = "";
            setByokNote(t("ai.settings.note.saved"), 2000);
        } catch (e) {
            setByokNote(t("ai.settings.note.save_failed", { error: errMsg(e) }));
        } finally {
            savingByok = false;
        }
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
        resetDeleteConfirm();
    }

    function viewSkill(s: SkillRecord) {
        editing = { ...s };
        isNew = false;
        skillNote = null;
        resetDeleteConfirm();
    }

    function cancelEdit() {
        editing = null;
        isNew = false;
        skillNote = null;
        resetDeleteConfirm();
    }

    async function saveSkill() {
        if (!editing) return;
        if (editing.builtin) {
            skillNote = t("ai.settings.skills.error.builtin_readonly");
            return;
        }
        if (!editing.name.trim() || !editing.content.trim() || !editing.id.trim()) {
            skillNote = t("ai.settings.skills.error.empty_fields");
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
            skillNote = t("ai.settings.skills.error.save_failed", { error: errMsg(e) });
        } finally {
            savingSkill = false;
        }
    }

    async function removeSkill(s: SkillRecord) {
        if (s.builtin) return;
        // 二次点击确认：第一次切到 "click again to confirm" 状态，3s 内不再点就回退
        if (!confirmingDelete) {
            confirmingDelete = true;
            confirmDeleteTimer = window.setTimeout(() => {
                confirmingDelete = false;
                confirmDeleteTimer = null;
            }, 3000);
            return;
        }
        resetDeleteConfirm();
        try {
            await ai.deleteSkill(s.id);
            editing = null;
            isNew = false;
            await refreshSkills();
        } catch (e) {
            skillNote = t("ai.settings.skills.error.delete_failed", { error: errMsg(e) });
        }
    }
</script>

<div class="page">
    <div class="warn">
        {t("ai.settings.warn.byok")}
        （<a href="https://www.anthropic.com/legal/privacy" onclick={(e) => openExternal(e, "https://www.anthropic.com/legal/privacy")}>Anthropic</a>
         / <a href="https://openai.com/policies/privacy-policy/" onclick={(e) => openExternal(e, "https://openai.com/policies/privacy-policy/")}>OpenAI</a>
         / <a href="https://platform.deepseek.com/downloads" onclick={(e) => openExternal(e, "https://platform.deepseek.com/downloads")}>DeepSeek</a>
         / <a href="https://docs.bigmodel.cn/cn/terms/privacy-policy" onclick={(e) => openExternal(e, "https://docs.bigmodel.cn/cn/terms/privacy-policy")}>GLM</a>）。
    </div>

    <div class="section-label">{t("ai.settings.section.provider")}</div>
    <div class="form">
        <div class="row">
            <label for="ai-provider">{t("ai.settings.label.provider")}</label>
            <select id="ai-provider" bind:value={provider} onchange={onProviderChange}>
                <option value="anthropic">Anthropic (Claude)</option>
                <option value="openai">OpenAI / {t("ai.settings.provider.openai_compat")}</option>
                <option value="deepseek">DeepSeek</option>
                <option value="glm">GLM (智谱)</option>
            </select>
        </div>
        <div class="row">
            <label for="ai-endpoint">{t("ai.settings.label.endpoint")}</label>
            <input id="ai-endpoint" type="text" bind:value={endpoint} placeholder={t("ai.settings.placeholder.endpoint")}/>
        </div>
        <div class="row">
            <label for="ai-apikey">{t("ai.settings.label.api_key")}</label>
            <input id="ai-apikey" type="password" bind:value={apiKey}
                   onblur={onApiKeyBlur}
                   placeholder={hasKey ? t("ai.settings.placeholder.api_key_set") : t("ai.settings.placeholder.api_key_unset")}/>
        </div>
        <div class="row">
            <label for="ai-model">{t("ai.settings.label.model")}</label>
            <div class="model-row">
                <input id="ai-model" type="text" list="ai-model-options"
                       bind:value={model} placeholder={t("ai.settings.placeholder.model")} required/>
                <button type="button" class="btn btn-sm" onclick={loadModels}
                        disabled={loadingModels}>
                    {loadingModels ? t("ai.settings.btn.loading_models") : t("ai.settings.btn.load_models")}
                </button>
            </div>
            {#if modelOptions.length > 0}
                <datalist id="ai-model-options">
                    {#each modelOptions as m (m.id)}
                        <option value={m.id}>{m.display_name ?? m.id}</option>
                    {/each}
                </datalist>
            {/if}
        </div>
        <div class="actions">
            <button class="btn btn-accent btn-sm" onclick={saveByok}
                    disabled={savingByok || !model.trim()}>
                {savingByok ? t("ai.settings.btn.saving") : t("common.save")}
            </button>
            {#if byokNote}<span class="note">{byokNote}</span>{/if}
        </div>
    </div>

    <div class="section-label">{t("ai.settings.danger.section")}</div>
    <div class="switch-card danger" class:on={dangerMode}>
        <div class="switch-card-body">
            <div id="danger-mode-title" class="switch-card-title"
                 class:on={dangerMode} class:off={!dangerMode}>
                {t("ai.settings.danger.label")}
            </div>
            <div id="danger-mode-desc" class="switch-card-desc">{t("ai.settings.danger.desc")}</div>
            {#if dangerNote}
                <div class="danger-err">{dangerNote}</div>
            {/if}
        </div>
        <label class="switch">
            <input type="checkbox" checked={dangerMode}
                   disabled={savingDanger}
                   onclick={handleDangerToggle}
                   aria-labelledby="danger-mode-title"
                   aria-describedby="danger-mode-desc"/>
            <span class="slider"></span>
        </label>
    </div>

    <div class="section-label skill-header">
        {t("ai.settings.section.skills")}
        {#if !editing}
            <button class="btn btn-sm" onclick={newSkill}>{t("ai.settings.skills.new")}</button>
        {/if}
    </div>

    {#if skillNote}
        <div class="banner">{skillNote} <button class="banner-close" onclick={() => (skillNote = null)} aria-label={t("common.close")}>×</button></div>
    {/if}

    {#if !editing}
        <div class="skill-list">
            {#each skills as s (s.id)}
                <button class="skill-item neu-sm" onclick={() => viewSkill(s)}>
                    <div class="skill-row">
                        <span class="skill-name">{s.name}</span>
                        <span class="skill-tag" class:builtin={s.builtin} class:user={!s.builtin}>
                            {s.builtin ? t("ai.settings.skills.tag.builtin") : t("ai.settings.skills.tag.user")}
                        </span>
                        <span class="skill-id">{s.id}</span>
                    </div>
                    {#if s.description}<div class="skill-desc">{s.description}</div>{/if}
                </button>
            {/each}
            {#if skills.length === 0}
                <div class="placeholder">{t("ai.settings.skills.empty")}</div>
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
                       placeholder={t("ai.settings.skills.placeholder.name")}/>
            </div>
            <div class="row">
                <label for="sk-desc">DESCRIPTION</label>
                <input id="sk-desc" type="text" bind:value={editing.description} disabled={editing.builtin}
                       placeholder={t("ai.settings.skills.placeholder.desc")}/>
            </div>
            <div class="row">
                <label for="sk-content">SYSTEM PROMPT</label>
                <textarea id="sk-content" bind:value={editing.content} disabled={editing.builtin}
                          rows="20"
                          placeholder={t("ai.settings.skills.placeholder.content")}></textarea>
            </div>
            <div class="actions">
                {#if !editing.builtin}
                    <button class="btn btn-accent btn-sm" onclick={saveSkill} disabled={savingSkill}>
                        {savingSkill ? t("ai.settings.btn.saving") : t("common.save")}
                    </button>
                {/if}
                {#if !editing.builtin && !isNew}
                    <button class="btn btn-sm btn-danger" class:confirming={confirmingDelete}
                            onclick={() => editing && removeSkill(editing)}>
                        {confirmingDelete ? t("ai.settings.skills.btn.delete_confirm") : t("ai.settings.skills.btn.delete")}
                    </button>
                {/if}
                <button class="btn btn-sm" onclick={cancelEdit}>{editing.builtin ? t("ai.settings.skills.btn.back") : t("ai.settings.skills.btn.cancel")}</button>
            </div>
        </div>
    {/if}
</div>

<!-- Danger mode confirmation dialog —— Tauri webview 不弹原生 confirm，
     用自定义模态。ARIA 跟 AppearanceSettings 的 Custom theme dialog 一致：
     backdrop=presentation（纯装饰可点关闭），内容=dialog+aria-modal=true。 -->
{#if showDangerDialog}
    <div class="dialog-backdrop" onclick={() => (showDangerDialog = false)} role="presentation">
        <div class="dialog surface-raised" onclick={(e) => e.stopPropagation()}
             role="dialog" aria-modal="true"
             aria-labelledby="danger-dialog-title"
             aria-describedby="danger-dialog-body">
            <h3 id="danger-dialog-title" class="danger-dialog-title">{t("ai.settings.danger.confirm_title")}</h3>
            <div id="danger-dialog-body" class="danger-dialog-body">{t("ai.settings.danger.confirm_body")}</div>
            <div class="btn-row">
                <button class="btn btn-sm" onclick={() => (showDangerDialog = false)}>
                    {t("common.cancel")}
                </button>
                <button class="btn btn-sm btn-danger-solid"
                        onclick={() => applyDangerMode(true)}
                        disabled={savingDanger}>
                    {t("ai.settings.danger.confirm_enable")}
                </button>
            </div>
        </div>
    </div>
{/if}

<style>
    .page {
        padding: 24px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }

    .warn {
        background: color-mix(in srgb, var(--warning) 12%, var(--bg));
        border-left: 3px solid var(--warning);
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
        gap: 10px;
    }
    .row {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    .row label {
        font-size: 12px;
        color: var(--text-sub);
    }
    .row input[type="text"],
    .row input[type="password"],
    .row select,
    .row textarea {
        width: 100%;
        box-sizing: border-box;
    }
    .row textarea {
        font-family: monospace;
        font-size: 12px;
        resize: vertical;
        min-height: 240px;
    }
    .model-row {
        display: flex;
        gap: 8px;
        align-items: stretch;
    }
    .model-row input { flex: 1; }

    .actions {
        display: flex;
        gap: 8px;
        align-items: center;
        margin-top: 4px;
    }
    .btn-danger.confirming {
        animation: confirmPulse 1.2s ease-in-out infinite;
    }
    @keyframes confirmPulse {
        0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--error) 45%, transparent); }
        50%      { box-shadow: 0 0 0 6px color-mix(in srgb, var(--error) 0%, transparent); }
    }
    .note {
        font-size: 12px;
        color: var(--accent);
    }

    .skill-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding-right: 0;
    }

    /* 复用全局 .switch-card 样式（见 styles/global.css）。Danger 变体在"开启"时
       把 title 颜色压成 --error——全局 .switch-card-title.on 默认是 --accent（绿），
       这里特化让"危险态"视觉上无法忽视，防止用户开了忘了又跑命令。 */
    .switch-card.danger.on .switch-card-title.on { color: var(--error); }
    .danger-err {
        margin-top: 6px;
        font-size: 12px;
        color: var(--error);
    }

    /* Danger confirm dialog —— 仿 GitHubSyncScreen 的模态结构 */
    .dialog-backdrop {
        position: fixed;
        inset: 0;
        z-index: 500;
        background: var(--overlay-strong);
        display: flex;
        align-items: center;
        justify-content: center;
    }
    .dialog {
        background: var(--bg);
        box-shadow: var(--raised);
        border-radius: var(--radius);
        padding: calc(24px * var(--density));
        max-width: 460px;
        display: flex;
        flex-direction: column;
        gap: 12px;
    }
    .danger-dialog-title {
        font-size: 16px;
        color: var(--error);
        font-weight: 700;
    }
    .danger-dialog-body {
        font-size: 13px;
        color: var(--text);
        line-height: 1.55;
        white-space: pre-line;
    }
    .btn-row {
        display: flex;
        gap: 8px;
        justify-content: flex-end;
        margin-top: 4px;
    }
    /* "确认启用"按钮 —— 用 error 配色，让用户知道点下去等于踩雷 */
    .btn-danger-solid {
        background: var(--error);
        color: var(--white);
        border-color: var(--error);
    }
    .btn-danger-solid:disabled { opacity: 0.5; cursor: not-allowed; }

    .banner {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 6px 12px;
        background: color-mix(in srgb, var(--error) 12%, var(--bg));
        color: var(--error);
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
