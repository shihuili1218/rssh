<script lang="ts">
  import { onMount } from "svelte";
  import * as app from "../stores/app.svelte.ts";
  import { t } from "../i18n/index.svelte.ts";

  let commandBlockBar = $state(true);
  let autoColorBlocks = $state(false);

  onMount(async () => {
    commandBlockBar = await app.loadCommandBlockBar();
    autoColorBlocks = await app.loadAutoColorBlocks();
  });

  async function saveCommandBlockBar() {
    await app.setCommandBlockBar(commandBlockBar);
  }

  async function saveAutoColorBlocks() {
    await app.setAutoColorBlocks(autoColorBlocks);
  }
</script>

<div class="page">
  <div class="section-label">{t("settings.shell.command_block")}</div>
  <!-- 命令块侧栏开关（主）→ 开启后展开：自动染色开关 + 快捷键提示。
       跟 .danger-card 同款"主开关 + 分隔 + 子内容"结构。色带渲染被侧栏开关罩着，
       关掉侧栏就没有块也没有染色，所以子项天然嵌在开启分支里。 -->
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
      <div class="cmd-block-head">
        <div class="cmd-block-head-body">
          <div class="cmd-block-title" class:on={autoColorBlocks} class:off={!autoColorBlocks}>
            {t("settings.shell.command_block_auto_color")}
          </div>
          <div class="cmd-block-desc">{t("settings.shell.command_block_auto_color_desc")}</div>
        </div>
        <label class="switch">
          <input type="checkbox" bind:checked={autoColorBlocks} onchange={saveAutoColorBlocks} />
          <span class="slider"></span>
        </label>
      </div>

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

  /* 卡片：复用全局 .card.surface-raised，本地只加 padding + 内布局。 */
  .cmd-block-card {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  /* 主开关行（title/desc + switch）。 */
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

  /* Tips 列表 —— 嵌在卡片内，不再有自己的 bg/border。 */
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
