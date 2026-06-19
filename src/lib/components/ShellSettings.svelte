<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import * as transfers from "../stores/transfers.svelte.ts";
  import { t } from "../i18n/index.svelte.ts";
  import Select from "./Select.svelte";

  let shells = $state<string[]>([]);
  let selectedShell = $state("");
  /** Custom radio 自己的 path —— 独立持有，切到内置 shell 再回来不丢用户输入。 */
  let customPath = $state("");
  /** 用户选了 Custom 但还没填路径的瞬态标记 —— 不持久化。
   *  避免把占位字符串写进 local_shell 设置导致重启坏 shell。 */
  let pendingCustom = $state(false);
  let verboseLog = $state(true);
  let connectTimeout = $state(10);
  let commandBlockBar = $state(true);
  let copyOnSelect = $state(false);
  let confirmCloseTab = $state(false);
  let rightClickAction = $state<app.RightClickAction>("menu");
  let rightClickOptions = $derived([
    { value: "menu", label: t("settings.shell.right_click_menu") },
    { value: "paste", label: t("settings.shell.right_click_paste") },
    { value: "copyPaste", label: t("settings.shell.right_click_copy_paste") },
  ]);
  /** SFTP 并发上限。main.ts 启动时 loadMaxConcurrent 已写过 store，
   *  这里 onMount 再读一次显示当前值。 */
  let sftpMaxConcurrent = $state(transfers.maxConcurrent());
  const sftpBounds = transfers.maxConcurrentBounds();

  /** 用户当前选中的是 Custom 还是某个内置 shell。
   *  pendingCustom（点了 Custom 没填）或 selectedShell 不在 shells 里都算 custom。 */
  let customMode = $derived(pendingCustom || (selectedShell !== "" && !shells.includes(selectedShell)));

  onMount(async () => {
    try { shells = await invoke<string[]>("list_shells"); } catch { shells = []; }
    selectedShell = await invoke<string | null>("get_setting", { key: "local_shell" }) ?? "";
    if (selectedShell && !shells.includes(selectedShell)) {
      customPath = selectedShell;
    }
    verboseLog = (await invoke<string | null>("get_setting", { key: "verbose_log" })) !== "false";
    const ts = await invoke<string | null>("get_setting", { key: "connect_timeout" });
    if (ts) connectTimeout = parseInt(ts, 10) || 10;
    commandBlockBar = await app.loadCommandBlockBar();
    copyOnSelect = await app.loadCopyOnSelect();
    confirmCloseTab = await app.loadConfirmCloseTab();
    rightClickAction = await app.loadRightClickAction();
    // SFTP 并发上限：main.ts 启动时已读过持久值进 store，但用户可能在打开 Settings 前
    // 还没 await 完。再读一次确保 input 显示真实当前值。
    await transfers.loadMaxConcurrent();
    sftpMaxConcurrent = transfers.maxConcurrent();
  });

  async function saveShell() {
    await invoke("set_setting", { key: "local_shell", value: selectedShell });
  }

  /** 选中内置 shell —— radio onchange 触发。 */
  function pickShell(sh: string) {
    pendingCustom = false;
    selectedShell = sh;
    saveShell();
  }

  /** 切到 Custom radio：仅在 customPath 已有值时才写入持久化；
   *  没填路径时只切 UI 状态，保留之前 selectedShell 不动，避免存空/占位污染。
   *  幂等：input refocus 时也调用本函数，已经等于 selectedShell 就不重复 invoke。 */
  function pickCustom() {
    pendingCustom = true;
    const v = customPath.trim();
    if (v && v !== selectedShell) {
      selectedShell = v;
      saveShell();
    }
  }

  /** Custom input blur：把 input 内容写回 selectedShell。 */
  function onCustomBlur() {
    const v = customPath.trim();
    if (v && pendingCustom) {
      selectedShell = v;
      saveShell();
    }
  }

  async function saveVerbose() {
    await invoke("set_setting", { key: "verbose_log", value: String(verboseLog) });
  }

  async function saveTimeout() {
    const val = Math.max(1, Math.min(300, connectTimeout));
    connectTimeout = val;
    await invoke("set_setting", { key: "connect_timeout", value: String(val) });
  }

  async function saveCommandBlockBar() {
    await app.setCommandBlockBar(commandBlockBar);
  }

  async function saveCopyOnSelect() {
    await app.setCopyOnSelect(copyOnSelect);
  }

  async function saveConfirmCloseTab() {
    await app.setConfirmCloseTab(confirmCloseTab);
  }

  function saveRightClickAction(v: app.RightClickAction) {
    void app.setRightClickAction(v);
  }

  async function saveSftpMaxConcurrent() {
    const clamped = Math.max(sftpBounds.min, Math.min(sftpBounds.max, sftpMaxConcurrent | 0));
    sftpMaxConcurrent = clamped;
    await transfers.setMaxConcurrent(clamped);
  }
</script>

<div class="page">
  <div class="section-label" id="local-shell-label">{t("settings.shell.local_shell")}</div>
  <div class="card surface-raised shell-card">
    <div class="shell-hint">
      {t("settings.shell.pick_hint")}
    </div>
    <div class="radio-group" role="radiogroup" aria-labelledby="local-shell-label">
      {#each shells as sh, i}
        {@const id = `shell-r-${i}`}
        {@const basename = (sh.split("/").pop() || sh).toUpperCase()}
        <div class="radio-wrapper">
          <input type="radio" id={id} name="local-shell" class="radio-state"
                 value={sh} checked={!customMode && (selectedShell === sh || (!selectedShell && shells[0] === sh))}
                 onchange={() => pickShell(sh)} />
          <label for={id} class="radio-label">
            <span class="shell-radio-indicator" aria-hidden="true"></span>
            <span class="info">
              <span class="name">{basename}</span>
              <span class="path">({sh})</span>
            </span>
          </label>
        </div>
      {/each}
      <div class="radio-wrapper">
        <input type="radio" id="shell-r-custom" name="local-shell" class="radio-state"
               checked={customMode}
               onchange={pickCustom} />
        <label for="shell-r-custom" class="radio-label">
          <span class="shell-radio-indicator" aria-hidden="true"></span>
          <span class="info">
            <span class="name">{t("settings.shell.custom")}</span>
            <input class="custom-input" type="text"
                   bind:value={customPath}
                   placeholder={t("settings.shell.custom_placeholder")}
                   onfocus={() => pickCustom()}
                   onblur={onCustomBlur} />
          </span>
        </label>
      </div>
    </div>
  </div>

  <div class="section-label">{t("settings.shell.connection_timeout")}</div>
  <div class="timeout-row">
    <label>{t("settings.shell.timeout_label")}</label>
    <input type="number" bind:value={connectTimeout} min="1" max="300" onblur={saveTimeout}
      onkeydown={(e) => { if (e.key === "Enter") saveTimeout(); }} />
    <span class="timeout-hint">{t("settings.shell.timeout_hint")}</span>
  </div>

  <div class="section-label">{t("settings.shell.sftp_concurrent")}</div>
  <div class="timeout-row">
    <label>{t("settings.shell.sftp_concurrent_label")}</label>
    <input type="number" bind:value={sftpMaxConcurrent}
      min={sftpBounds.min} max={sftpBounds.max}
      onblur={saveSftpMaxConcurrent}
      onkeydown={(e) => { if (e.key === "Enter") saveSftpMaxConcurrent(); }} />
    <span class="timeout-hint">{t("settings.shell.sftp_concurrent_hint", {
      min: sftpBounds.min, max: sftpBounds.max, def: sftpBounds.def,
    })}</span>
  </div>

  <div class="section-label">{t("settings.shell.connection_logging")}</div>
  <div class="switch-card">
    <div class="switch-card-body">
      <div class="switch-card-title" class:on={verboseLog} class:off={!verboseLog}>{t("settings.shell.verbose_log")}</div>
      <div class="switch-card-desc">{t("settings.shell.verbose_log_desc")}</div>
    </div>
    <label class="switch">
      <input type="checkbox" bind:checked={verboseLog} onchange={saveVerbose} />
      <span class="slider"></span>
    </label>
  </div>

  <div class="section-label">{t("settings.shell.interaction")}</div>
  <!-- 终端交互：选中即复制（开关）+ 关闭标签页确认（开关）+ 右键动作（下拉）合在一张卡片，
       "行 + 分隔线 + 行"结构，跟命令块卡片同款，避免控件割裂。 -->
  <div class="card surface-raised mouse-card">
    <div class="cmd-block-head">
      <div class="cmd-block-head-body">
        <div class="cmd-block-title" class:on={copyOnSelect} class:off={!copyOnSelect}>{t("settings.shell.copy_on_select")}</div>
        <div class="cmd-block-desc">{t("settings.shell.copy_on_select_desc")}</div>
      </div>
      <label class="switch">
        <input type="checkbox" bind:checked={copyOnSelect} onchange={saveCopyOnSelect} />
        <span class="slider"></span>
      </label>
    </div>

    <div class="card-divider"></div>

    <div class="cmd-block-head">
      <div class="cmd-block-head-body">
        <div class="cmd-block-title" class:on={confirmCloseTab} class:off={!confirmCloseTab}>{t("settings.shell.confirm_close_tab")}</div>
        <div class="cmd-block-desc">{t("settings.shell.confirm_close_tab_desc")}</div>
      </div>
      <label class="switch">
        <input type="checkbox" bind:checked={confirmCloseTab} onchange={saveConfirmCloseTab} />
        <span class="slider"></span>
      </label>
    </div>

    <div class="card-divider"></div>

    <div class="cmd-block-head">
      <div class="cmd-block-head-body">
        <label for="rca-select" class="cmd-block-title">{t("settings.shell.right_click")}</label>
        <div class="cmd-block-desc">{t("settings.shell.right_click_desc")}</div>
      </div>
      <div class="rca-select">
        <Select id="rca-select" bind:value={rightClickAction}
                options={rightClickOptions}
                onchange={(v) => saveRightClickAction(v as app.RightClickAction)} />
      </div>
    </div>
  </div>

  <div class="section-label">{t("settings.shell.command_block")}</div>
  <!-- 命令块侧栏开关 + 启用后的快捷键提示合在一个 .card.surface-raised。
       关时只有开关行；开时分隔线下展开 tips，跟 .danger-card 同款"主开关 + 分隔 + 子内容"结构。 -->
  <div class="card surface-raised cmd-block-card">
    <div class="cmd-block-head">
      <div class="cmd-block-head-body">
        <div class="cmd-block-title"
             class:on={commandBlockBar} class:off={!commandBlockBar}>
          {t("settings.shell.command_block_bar")}
        </div>
        <div class="cmd-block-desc">{t("settings.shell.command_block_bar_desc")}</div>
      </div>
      <label class="switch">
        <input type="checkbox" bind:checked={commandBlockBar} onchange={saveCommandBlockBar} />
        <span class="slider"></span>
      </label>
    </div>

    {#if commandBlockBar}
      <div class="card-divider"></div>
      <div class="tips-group">
        <div class="tips-title">{t("settings.shell.command_block_tips_title")}</div>
        <ul class="tips-list">
          <li>{t("settings.shell.command_block_tip_click")}</li>
          <li>{t("settings.shell.command_block_tip_shift_click")}</li>
          <li>{t("settings.shell.command_block_tip_cmd_click")}</li>
          <li>{t("settings.shell.command_block_tip_right_click")}</li>
          <li>{t("settings.shell.command_block_tip_clear")}</li>
        </ul>
      </div>
    {/if}
  </div>

</div>

<style>
  .page { padding: 24px; display: flex; flex-direction: column; gap: 16px; }

  /* 卡片：复用全局 .card.surface-raised，本地只加 padding + 内布局，
     跟 SyncScreen / AiSettings 同款。 */
  .shell-card,
  .cmd-block-card,
  .mouse-card {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  /* 提示文本：跟 SyncScreen .pat-hint 同一档（11px / text-dim / 行高 1.5）。 */
  .shell-hint {
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.5;
  }

  /* Radio group —— 复刻 uiverse neu radio：三层圆形阴影（外圈 raised + 内圈 reversed well +
     凸起盖板）。选中时盖板缩小+下移+淡出，露出底下的"井"。
     颜色 token 化：#ecf0f3 → var(--surface)、#d1d9e6 → var(--shadow-dark)、#fff → var(--shadow-light)。
     尺寸：indicator 从参考的 30px 缩到 20px（rssh 字体 13-14px，30 太大），阴影 offset/blur 按比例缩。 */
  .radio-group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .radio-wrapper {
    position: relative;
  }

  /* 真 input：照搬参考。pointer-events:none → 鼠标穿透到 label，label[for] 转发让 input
     获取 focus；focus 状态触发 `:focus ~ .radio-label .info` 右移 8px。
     注意不能照参考留默认 width/height —— 全局 input 给了 0/0 之外的尺寸会盖住后面 sibling。 */
  .radio-state {
    position: absolute;
    top: 0;
    right: 0;
    width: 1px;
    height: 1px;
    opacity: 1e-5;
    pointer-events: none;
    margin: 0;
    padding: 0;
    box-shadow: none;
  }

  .radio-label {
    display: flex;
    align-items: center;
    gap: 12px;
    cursor: pointer;
    min-height: 20px;
  }

  /* indicator 视觉按主题分化：neu 在这里写（三层阴影），flat / material 各自在
     styles/shapes/*.css 接管。class 用 :global() 暴露给外部 shape selector hook。
     裸默认（无主题匹配时）= 透明圆，避免空白 fallback 难看。 */
  :global(.shell-radio-indicator) {
    position: relative;
    flex-shrink: 0;
    border-radius: 50%;
    height: 20px;
    width: 20px;
    overflow: hidden;
  }

  /* neu 主题：三层圆形阴影（外圈 raised + 内圈 reversed well + 凸起盖板）。
     :checked 时盖板缩小+下移+淡出，露出底下的"井"。 */
  :global(:root[data-shape="neumorphism"] .shell-radio-indicator) {
    box-shadow:
        -5px -3px 5px 0px var(--shadow-light),
        5px 3px 8px 0px var(--shadow-dark);
  }
  :global(:root[data-shape="neumorphism"] .shell-radio-indicator::before),
  :global(:root[data-shape="neumorphism"] .shell-radio-indicator::after) {
    content: "";
    position: absolute;
    top: 10%;
    left: 10%;
    height: 80%;
    width: 80%;
    border-radius: 50%;
  }
  :global(:root[data-shape="neumorphism"] .shell-radio-indicator::before) {
    box-shadow:
        -3px -1.5px 3px 0px var(--shadow-dark),
        3px 1.5px 5px 0px var(--shadow-light);
  }
  :global(:root[data-shape="neumorphism"] .shell-radio-indicator::after) {
    background-color: var(--surface);
    box-shadow:
        -3px -1.5px 3px 0px var(--shadow-light),
        3px 1.5px 5px 0px var(--shadow-dark);
    transform: scale3d(1, 1, 1);
    transition: opacity 0.25s ease-in-out, transform 0.25s ease-in-out;
  }
  /* :checked 用 input[type="radio"] + sibling label 的 element selector，
     避免 scoped class hash 问题；.shell-radio-indicator 限定只命中本组件的 radio。 */
  :global(:root[data-shape="neumorphism"] input[type="radio"]:checked ~ label .shell-radio-indicator::after) {
    transform: scale3d(0.975, 0.975, 1) translate3d(0, 10%, 0);
    opacity: 0;
  }

  /* 文字：name + path 单行 inline 排列。
     opacity 1 不衰减 —— 之前 0.6→1 的微交互是参考里 `:focus ~` 那套的辅助效果，
     focus 部分删了之后留着反而让 Custom 行的 input placeholder 也跟着淡到 0.36
     几乎看不见。状态对比交给 name 的 accent 配色就够。 */
  .info {
    flex: 1;
    display: inline-flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    letter-spacing: 0.04em;
    flex-shrink: 0;
  }
  .radio-state:checked ~ .radio-label .name { color: var(--accent); }
  .path {
    font-size: 11px;
    color: var(--text-dim);
    font-family: monospace;
    word-break: break-all;
  }

  /* Custom radio 行的 input 紧贴 name 后面，跟其它行的 path 同槽位、同样式（dim/monospace）。
     placeholder 写 "(/usr/local/bin/fish)"，括号风格跟内置行的 (/bin/zsh) 一致。 */
  .custom-input {
    flex: 1;
    background: transparent;
    border: none;
    box-shadow: none;
    padding: 0;
    font-size: 11px;
    font-family: monospace;
    color: var(--text-dim);
    border-radius: 0;
    min-width: 0;
  }
  .custom-input:focus { outline: none; color: var(--text); }
  .custom-input::placeholder { color: var(--text-dim); opacity: 0.6; }

  .timeout-row {
    display: flex; align-items: center; gap: 10px;
  }
  .timeout-row input[type="number"] {
    width: 80px;
  }

  /* 右键动作的下拉框：定宽，靠右，不被卡片行压缩。 */
  .rca-select { width: 260px; flex-shrink: 0; }
  .timeout-hint {
    font-size: 11px; color: var(--text-dim);
  }

  /* 命令块卡片：主开关行（title/desc + switch）→ 分隔线 → tips 列表，
     跟 .danger-card 同样的"主开关 + 子内容"结构。 */
  .cmd-block-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .cmd-block-head-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .cmd-block-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .cmd-block-title.on { color: var(--accent); }
  .cmd-block-desc {
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.5;
  }

  /* 卡片内分隔线：负边距贯穿到卡片左右边缘。 */
  .card-divider {
    height: 1px;
    background: var(--divider);
    margin: 2px -18px;
  }

  /* Tips 列表 —— 嵌在 .cmd-block-card 内，不再有自己的 bg/border。 */
  .tips-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .tips-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-sub);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .tips-list {
    margin: 0;
    padding-left: 18px;
    font-size: 12px;
    color: var(--text);
    line-height: 1.6;
  }
  .tips-list li {
    margin: 2px 0;
  }
</style>
