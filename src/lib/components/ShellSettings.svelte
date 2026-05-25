<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import * as app from "../stores/app.svelte.ts";
  import { t } from "../i18n/index.svelte.ts";

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
</script>

<div class="page">
  <div class="section-label" id="local-shell-label">LOCAL SHELL</div>
  <div class="card surface-raised shell-card">
    <div class="shell-hint">
      Pick a shell to use for new local terminals, or choose Custom and type your own path.
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
            <span class="name">CUSTOM</span>
            <input class="custom-input" type="text"
                   bind:value={customPath}
                   placeholder="/usr/local/bin/fish"
                   onfocus={() => pickCustom()}
                   onblur={onCustomBlur} />
          </span>
        </label>
      </div>
    </div>
  </div>

  <div class="section-label">CONNECTION TIMEOUT</div>
  <div class="timeout-row">
    <label>Timeout (seconds)</label>
    <input type="number" bind:value={connectTimeout} min="1" max="300" onblur={saveTimeout}
      onkeydown={(e) => { if (e.key === "Enter") saveTimeout(); }} />
    <span class="timeout-hint">1–300s, default 10s</span>
  </div>

  <div class="section-label">CONNECTION LOGGING</div>
  <div class="switch-card">
    <div class="switch-card-body">
      <div class="switch-card-title" class:on={verboseLog} class:off={!verboseLog}>VERBOSE LOG</div>
      <div class="switch-card-desc">Show detailed SSH handshake and authentication messages in terminal.</div>
    </div>
    <label class="switch">
      <input type="checkbox" bind:checked={verboseLog} onchange={saveVerbose} />
      <span class="slider"></span>
    </label>
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
     跟 GitHubSyncScreen / AiSettings 同款。 */
  .shell-card,
  .cmd-block-card {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  /* 提示文本：跟 GitHubSyncScreen .pat-hint 同一档（11px / text-dim / 行高 1.5）。 */
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
