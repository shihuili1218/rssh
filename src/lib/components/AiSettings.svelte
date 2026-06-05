<script lang="ts">
    import { onMount, onDestroy } from "svelte";
    import { invoke } from "@tauri-apps/api/core";
    import * as ai from "../ai/store.svelte.ts";
    import { t, errMsg } from "../i18n/index.svelte.ts";
    import type { LlmProvider, ModelInfo, RedactRuleRecord, SkillRecord } from "../ai/types.ts";
    import Select from "./Select.svelte";
    import SearchSelect from "./SearchSelect.svelte";

    /** Provider 下拉选项 —— OpenAI 那项的翻译用 $derived 跟着 locale 自动重算。 */
    let providerOptions = $derived([
        { value: "anthropic", label: "Anthropic (Claude)" },
        { value: "openai",    label: `OpenAI / ${t("ai.settings.provider.openai_compat")}` },
        { value: "deepseek",  label: "DeepSeek" },
        { value: "glm",       label: "GLM (智谱)" },
    ]);

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

    // per-tool 自动批准。每个 checkbox 各自一个 boolean state。8 个字段平铺；
    // 也可以塞成数组遍历，但显式 boolean 比 metadata 表调试时更直观，也避免一行
    // toggle bug 同时拆掉 8 个开关。
    let autoRunCommand = $state(true);
    let autoMatchFile = $state(true);
    let autoDownloadFile = $state(false);
    let autoAnalyzeLocally = $state(false);
    let autoPatchCp = $state(false);
    let autoPatchModify = $state(false);
    let autoPatchDiff = $state(false);
    let autoPatchMv = $state(false);
    let savingAuto = $state(false);

    // ─── 远端 shell 自动探测（与 danger_mode 解耦的独立开关）─────────
    // 默认 off：99% Linux/macOS 远端假设 POSIX 即正确，零探测开销保持现状。
    // 用户连 Windows / 改了 DefaultShell 的远端时手动开启，每次 SSH 连接成功后跑
    // 一行 echo 探针自动定位 cmd.exe / PowerShell（对已连接会话开启需重连生效）。
    let autoDetectRemoteShell = $state(false);
    let savingShellDetect = $state(false);
    let shellDetectNote = $state<string | null>(null);

    async function persistAutoDetectShell(next: boolean) {
        savingShellDetect = true;
        shellDetectNote = null;
        try {
            await ai.saveSettings({ autoDetectRemoteShell: next });
        } catch (err) {
            // 失败回滚 UI 状态，保持跟后端单一真相
            autoDetectRemoteShell = !next;
            shellDetectNote = t("ai.settings.shell_detect.save_failed", { error: errMsg(err) });
        } finally {
            savingShellDetect = false;
        }
    }

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

    /** 子开关写回后端。失败把 UI 状态回滚到 prev，避免界面与持久化失同步。 */
    async function persistAuto(field: "autoRunCommand" | "autoMatchFile" | "autoDownloadFile"
                                    | "autoAnalyzeLocally" | "autoPatchCp" | "autoPatchModify"
                                    | "autoPatchDiff" | "autoPatchMv",
                               next: boolean) {
        savingAuto = true;
        dangerNote = null;
        try {
            await ai.saveSettings({ [field]: next });
        } catch (err) {
            // 任一保存失败都把对应字段回滚——单 source of truth 是后端
            switch (field) {
                case "autoRunCommand":     autoRunCommand     = !next; break;
                case "autoMatchFile":      autoMatchFile      = !next; break;
                case "autoDownloadFile":   autoDownloadFile   = !next; break;
                case "autoAnalyzeLocally": autoAnalyzeLocally = !next; break;
                case "autoPatchCp":        autoPatchCp        = !next; break;
                case "autoPatchModify":    autoPatchModify    = !next; break;
                case "autoPatchDiff":      autoPatchDiff      = !next; break;
                case "autoPatchMv":        autoPatchMv        = !next; break;
            }
            dangerNote = t("ai.settings.danger.save_failed", { error: errMsg(err) });
        } finally {
            savingAuto = false;
        }
    }
    let modelOptions = $state<ModelInfo[]>([]);
    /** Model dropdown options — id as value, display name (or id) as label. */
    let modelSelectOptions = $derived(
        modelOptions.map((m) => ({ value: m.id, label: m.display_name ?? m.id })),
    );
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

    // ─── 脱敏规则管理 ──────────────────────────────────────────
    // 镜像 Skill 管理：列表 + 行内编辑表单 + 二次点击删除确认。规则没有 builtin
    // 概念（默认已 seed 进 DB），全部可改可删。变更只对新会话生效。
    let redactRules = $state<RedactRuleRecord[]>([]);
    let editingRule = $state<RedactRuleRecord | null>(null);
    let isNewRule = $state(false);
    let savingRule = $state(false);
    let ruleNote = $state<string | null>(null);
    // 独立于 skill 的删除确认状态，避免两处共用一个 flag 互相串台。
    let confirmingRuleDelete = $state(false);
    let confirmRuleDeleteTimer: number | null = null;

    function resetRuleDeleteConfirm() {
        confirmingRuleDelete = false;
        if (confirmRuleDeleteTimer !== null) {
            clearTimeout(confirmRuleDeleteTimer);
            confirmRuleDeleteTimer = null;
        }
    }

    onMount(async () => {
        const s = await ai.loadSettings();
        provider = s.provider as LlmProvider;
        model = s.model;
        endpoint = s.endpoint ?? "";
        hasKey = s.has_api_key;
        dangerMode = s.danger_mode;
        autoRunCommand = s.auto_run_command;
        autoMatchFile = s.auto_match_file;
        autoDownloadFile = s.auto_download_file;
        autoAnalyzeLocally = s.auto_analyze_locally;
        autoPatchCp = s.auto_patch_cp;
        autoPatchModify = s.auto_patch_modify;
        autoPatchDiff = s.auto_patch_diff;
        autoPatchMv = s.auto_patch_mv;
        autoDetectRemoteShell = s.auto_detect_remote_shell;
        if (hasKey) void autoLoadModels();
        await refreshSkills();
        await refreshRedactRules();
    });

    onDestroy(() => {
        if (byokNoteTimer !== null) clearTimeout(byokNoteTimer);
        if (confirmDeleteTimer !== null) clearTimeout(confirmDeleteTimer);
        if (confirmRuleDeleteTimer !== null) clearTimeout(confirmRuleDeleteTimer);
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

    async function refreshRedactRules() {
        try {
            redactRules = await ai.listRedactRules();
        } catch (e) {
            ruleNote = t("ai.settings.redact.error.load_failed", { error: errMsg(e) });
        }
    }

    function newRule() {
        editingRule = {
            id: "user-" + crypto.randomUUID().slice(0, 8),
            pattern: "",
            replacement: "",
        };
        isNewRule = true;
        ruleNote = null;
        resetRuleDeleteConfirm();
    }

    function viewRule(r: RedactRuleRecord) {
        editingRule = { ...r };
        isNewRule = false;
        ruleNote = null;
        resetRuleDeleteConfirm();
    }

    function cancelRuleEdit() {
        editingRule = null;
        isNewRule = false;
        ruleNote = null;
        resetRuleDeleteConfirm();
    }

    async function saveRule() {
        if (!editingRule) return;
        // 空串校验用 === ""（不 trim）：正则里前后空格可能有意义；空 pattern 会匹配
        // 每个位置导致灾难性替换，必须挡掉。后端还会再编译校验一次。
        if (editingRule.pattern === "" || editingRule.replacement === "") {
            ruleNote = t("ai.settings.redact.error.empty_fields");
            return;
        }
        savingRule = true;
        ruleNote = null;
        try {
            await ai.saveRedactRule({
                id: editingRule.id,
                pattern: editingRule.pattern,
                replacement: editingRule.replacement,
            });
            editingRule = null;
            isNewRule = false;
            await refreshRedactRules();
        } catch (e) {
            // 坏正则在后端被拒，errMsg 解析出 error.redact_invalid_regex 的中文/英文消息。
            ruleNote = t("ai.settings.redact.error.save_failed", { error: errMsg(e) });
        } finally {
            savingRule = false;
        }
    }

    async function removeRule(r: RedactRuleRecord) {
        if (!confirmingRuleDelete) {
            confirmingRuleDelete = true;
            confirmRuleDeleteTimer = window.setTimeout(() => {
                confirmingRuleDelete = false;
                confirmRuleDeleteTimer = null;
            }, 3000);
            return;
        }
        resetRuleDeleteConfirm();
        try {
            await ai.deleteRedactRule(r.id);
            editingRule = null;
            isNewRule = false;
            await refreshRedactRules();
        } catch (e) {
            ruleNote = t("ai.settings.redact.error.delete_failed", { error: errMsg(e) });
        }
    }
</script>

<div class="page">
    <div class="section-label">{t("ai.settings.section.provider")}</div>
    <!-- Provider & Model + BYOK 警告合在一个 .card.surface-raised（跟 .danger-card / GitHubSyncScreen 同款）。
         .warn 留在卡片顶部作"PAT hint"的等价位置，但保留自身警告样式（border-left + tint bg）。 -->
    <div class="card surface-raised provider-card">
        <div class="warn">
            {t("ai.settings.warn.byok")}
            （<a href="https://www.anthropic.com/legal/privacy" onclick={(e) => openExternal(e, "https://www.anthropic.com/legal/privacy")}>Anthropic</a>
             / <a href="https://openai.com/policies/privacy-policy/" onclick={(e) => openExternal(e, "https://openai.com/policies/privacy-policy/")}>OpenAI</a>
             / <a href="https://platform.deepseek.com/downloads" onclick={(e) => openExternal(e, "https://platform.deepseek.com/downloads")}>DeepSeek</a>
             / <a href="https://docs.bigmodel.cn/cn/terms/privacy-policy" onclick={(e) => openExternal(e, "https://docs.bigmodel.cn/cn/terms/privacy-policy")}>GLM</a>）。
        </div>

        <div class="row">
            <label for="ai-provider">{t("ai.settings.label.provider")}</label>
            <Select id="ai-provider"
                    bind:value={provider as string}
                    options={providerOptions}
                    onchange={onProviderChange} />
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
                <SearchSelect id="ai-model"
                              bind:value={model}
                              options={modelSelectOptions}
                              allowCustom
                              ariaLabel={t("ai.settings.label.model")}
                              placeholder={t("ai.settings.placeholder.model")}
                              searchPlaceholder={t("ai.settings.placeholder.model")}
                              emptyText={t("ai.settings.model.empty")} />
                <button type="button" class="btn btn-sm" onclick={loadModels}
                        disabled={loadingModels}>
                    {loadingModels ? t("ai.settings.btn.loading_models") : t("ai.settings.btn.load_models")}
                </button>
            </div>
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
    <!-- 危险模式 + 8 个 per-tool 自动批准合在一个 .card.surface-raised（参考 GitHubSyncScreen）。
         视觉上是一组语义关联的配置，不再拆成两个浮空卡片。 -->
    <div class="card surface-raised danger-card" class:on={dangerMode}>
        <div class="danger-head">
            <div class="danger-head-body">
                <div id="danger-mode-title" class="danger-title"
                     class:on={dangerMode} class:off={!dangerMode}>
                    {t("ai.settings.danger.label")}
                </div>
                <div id="danger-mode-desc" class="danger-desc">{t("ai.settings.danger.desc")}</div>
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

        <div class="card-divider"></div>

        <!-- per-tool 自动批准。danger_mode 关时整组 disabled —— 视觉灰显，不隐藏，
             让用户知道这些选项存在、是怎么分粒度的；开 danger 时立刻可用。 -->
        <div class="auto-group" class:disabled={!dangerMode}>
            <div class="auto-group-title">{t("ai.settings.danger.auto.section")}</div>
            <label class="auto-row">
                <input type="checkbox" bind:checked={autoRunCommand}
                       disabled={!dangerMode || savingAuto}
                       onchange={(e) => persistAuto("autoRunCommand", (e.target as HTMLInputElement).checked)}/>
                <span>{t("ai.settings.danger.auto.run_command")}</span>
            </label>
            <label class="auto-row">
                <input type="checkbox" bind:checked={autoMatchFile}
                       disabled={!dangerMode || savingAuto}
                       onchange={(e) => persistAuto("autoMatchFile", (e.target as HTMLInputElement).checked)}/>
                <span>{t("ai.settings.danger.auto.match_file")}</span>
            </label>
            <label class="auto-row">
                <input type="checkbox" bind:checked={autoDownloadFile}
                       disabled={!dangerMode || savingAuto}
                       onchange={(e) => persistAuto("autoDownloadFile", (e.target as HTMLInputElement).checked)}/>
                <span>{t("ai.settings.danger.auto.download_file")}</span>
            </label>
            <label class="auto-row">
                <input type="checkbox" bind:checked={autoAnalyzeLocally}
                       disabled={!dangerMode || savingAuto}
                       onchange={(e) => persistAuto("autoAnalyzeLocally", (e.target as HTMLInputElement).checked)}/>
                <span>{t("ai.settings.danger.auto.analyze_locally")}</span>
            </label>
            <label class="auto-row">
                <input type="checkbox" bind:checked={autoPatchCp}
                       disabled={!dangerMode || savingAuto}
                       onchange={(e) => persistAuto("autoPatchCp", (e.target as HTMLInputElement).checked)}/>
                <span>{t("ai.settings.danger.auto.patch_cp")}</span>
            </label>
            <label class="auto-row">
                <input type="checkbox" bind:checked={autoPatchModify}
                       disabled={!dangerMode || savingAuto}
                       onchange={(e) => persistAuto("autoPatchModify", (e.target as HTMLInputElement).checked)}/>
                <span>{t("ai.settings.danger.auto.patch_modify")}</span>
            </label>
            <label class="auto-row">
                <input type="checkbox" bind:checked={autoPatchDiff}
                       disabled={!dangerMode || savingAuto}
                       onchange={(e) => persistAuto("autoPatchDiff", (e.target as HTMLInputElement).checked)}/>
                <span>{t("ai.settings.danger.auto.patch_diff")}</span>
            </label>
            <label class="auto-row">
                <input type="checkbox" bind:checked={autoPatchMv}
                       disabled={!dangerMode || savingAuto}
                       onchange={(e) => persistAuto("autoPatchMv", (e.target as HTMLInputElement).checked)}/>
                <span>{t("ai.settings.danger.auto.patch_mv")}</span>
            </label>
        </div>
    </div>

    <!-- 远端 shell 自动探测 —— 独立卡片，跟 danger_mode 解耦。
         off（默认）：远端假设 POSIX，保持 99% 用户零开销。
         on：SSH 连接成功后跑一行 echo 探针，定位 cmd.exe / PowerShell。 -->
    <div class="card surface-raised">
        <div class="danger-head">
            <div class="danger-head-body">
                <div id="shell-detect-title" class="danger-title">
                    {t("ai.settings.shell_detect.label")}
                </div>
                <div id="shell-detect-desc" class="danger-desc">{t("ai.settings.shell_detect.desc")}</div>
                {#if shellDetectNote}
                    <div class="danger-err">{shellDetectNote}</div>
                {/if}
            </div>
            <label class="switch">
                <input type="checkbox" bind:checked={autoDetectRemoteShell}
                       disabled={savingShellDetect}
                       onchange={(e) => persistAutoDetectShell((e.target as HTMLInputElement).checked)}
                       aria-labelledby="shell-detect-title"
                       aria-describedby="shell-detect-desc"/>
                <span class="slider"></span>
            </label>
        </div>
    </div>

    <!-- 脱敏规则管理 —— 镜像下方 Skill 段：列表 / 行内表单 / 二次删除确认。 -->
    <div class="section-label skill-header">
        {t("ai.settings.section.redact")}
        {#if !editingRule}
            <button class="btn btn-sm" onclick={newRule}>{t("ai.settings.redact.new")}</button>
        {/if}
    </div>
    <div class="redact-hint">{t("ai.settings.redact.hint")}</div>

    {#if ruleNote}
        <div class="banner">{ruleNote} <button class="banner-close" onclick={() => (ruleNote = null)} aria-label={t("common.close")}>×</button></div>
    {/if}

    {#if !editingRule}
        <div class="skill-list">
            {#each redactRules as r (r.id)}
                <button class="skill-item surface-raised-sm" onclick={() => viewRule(r)}>
                    <div class="rule-line">
                        <code class="rule-pattern">{r.pattern}</code>
                        <span class="rule-arrow">→</span>
                        <code class="rule-replacement">{r.replacement}</code>
                    </div>
                </button>
            {/each}
            {#if redactRules.length === 0}
                <div class="placeholder">{t("ai.settings.redact.empty")}</div>
            {/if}
        </div>
    {:else}
        <div class="form">
            <div class="row">
                <label for="rr-pattern">{t("ai.settings.redact.label.pattern")}</label>
                <input id="rr-pattern" type="text" class="mono" bind:value={editingRule.pattern}
                       placeholder={t("ai.settings.redact.placeholder.pattern")}/>
            </div>
            <div class="row">
                <label for="rr-replacement">{t("ai.settings.redact.label.replacement")}</label>
                <input id="rr-replacement" type="text" class="mono" bind:value={editingRule.replacement}
                       placeholder={t("ai.settings.redact.placeholder.replacement")}/>
            </div>
            <div class="actions">
                <button class="btn btn-accent btn-sm" onclick={saveRule} disabled={savingRule}>
                    {savingRule ? t("ai.settings.btn.saving") : t("common.save")}
                </button>
                {#if !isNewRule}
                    <button class="btn btn-sm btn-danger" class:confirming={confirmingRuleDelete}
                            onclick={() => editingRule && removeRule(editingRule)}>
                        {confirmingRuleDelete ? t("ai.settings.redact.btn.delete_confirm") : t("ai.settings.redact.btn.delete")}
                    </button>
                {/if}
                <button class="btn btn-sm" onclick={cancelRuleEdit}>{t("ai.settings.redact.btn.cancel")}</button>
            </div>
        </div>
    {/if}

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
                <label for="sk-id">{t("ai.settings.skills.label.id")}</label>
                <input id="sk-id" type="text" bind:value={editing.id}
                       disabled={!isNew || editing.builtin}/>
            </div>
            <div class="row">
                <label for="sk-name">{t("ai.settings.skills.label.name")}</label>
                <input id="sk-name" type="text" bind:value={editing.name} disabled={editing.builtin}
                       placeholder={t("ai.settings.skills.placeholder.name")}/>
            </div>
            <div class="row">
                <label for="sk-desc">{t("ai.settings.skills.label.description")}</label>
                <input id="sk-desc" type="text" bind:value={editing.description} disabled={editing.builtin}
                       placeholder={t("ai.settings.skills.placeholder.desc")}/>
            </div>
            <div class="row">
                <label for="sk-content">{t("ai.settings.skills.label.system_prompt")}</label>
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
    .model-row :global(.search-select) { flex: 1; min-width: 0; }

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

    /* Provider / Danger 卡片：复用全局 .card.surface-raised 提供 bg + 阴影 + 圆角，
       本地只加 padding + 内布局，跟 GitHubSyncScreen 同款。 */
    .provider-card,
    .danger-card {
        padding: 18px;
        display: flex;
        flex-direction: column;
        gap: 14px;
    }

    /* 主开关行：title/desc 在左，switch 在右；不再依赖全局 .switch-card 容器。 */
    .danger-head {
        display: flex;
        align-items: center;
        gap: 12px;
    }
    .danger-head-body {
        flex: 1;
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    .danger-title {
        font-size: 13px;
        font-weight: 600;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    /* "开启"时压成 error 红 —— 让危险态视觉无法忽视，防止用户开了忘了又跑命令。 */
    .danger-title.on { color: var(--error); }
    .danger-desc {
        font-size: 11px;
        color: var(--text-dim);
        line-height: 1.5;
    }
    .danger-err {
        font-size: 12px;
        color: var(--error);
    }

    /* 卡片内分隔线：用负边距贯穿到卡片左右边缘，视觉上是"横切"而非缩进线。 */
    .card-divider {
        height: 1px;
        background: var(--divider);
        margin: 2px -18px;
    }

    /* per-tool 自动批准 —— 嵌在 .danger-card 内，不再有自己的 bg/border。 */
    .auto-group {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .auto-group.disabled {
        opacity: 0.5;
    }
    .auto-group-title {
        font-size: 11px;
        color: var(--text-sub);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin-bottom: 4px;
    }
    .auto-row {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 13px;
        cursor: pointer;
    }
    .auto-row input[type="checkbox"] {
        cursor: pointer;
    }
    .auto-group.disabled .auto-row,
    .auto-group.disabled .auto-row input[type="checkbox"] {
        cursor: not-allowed;
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

    /* 脱敏规则 ─────────────────────────────────────────────── */
    .redact-hint {
        font-size: 11px;
        color: var(--text-dim);
        line-height: 1.5;
        margin-top: -6px;
    }
    /* 列表行：pattern → replacement，等宽字体，过长截断不撑破卡片。 */
    .rule-line {
        display: flex;
        align-items: baseline;
        gap: 8px;
        min-width: 0;
    }
    .rule-pattern,
    .rule-replacement {
        font-family: monospace;
        font-size: 12px;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .rule-pattern {
        color: var(--text);
        flex: 1 1 auto;
    }
    .rule-arrow {
        color: var(--text-dim);
        flex: 0 0 auto;
    }
    .rule-replacement {
        color: var(--accent);
        flex: 0 1 auto;
    }
    /* pattern / replacement 输入框用等宽，跟正则语义一致。 */
    .row input.mono {
        font-family: monospace;
    }
</style>
