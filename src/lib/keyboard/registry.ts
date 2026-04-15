/**
 * 声明式键盘快捷键注册表。
 *
 * 用法：定义一组 `Shortcut`，调用 `attachShortcuts` 注册到 window，返回 detach 函数。
 *
 * 顺序很重要：先 match 上的 shortcut 先生效。把 capture 优先级高的放前面。
 */

import * as app from "../stores/app.svelte.ts";

export interface Shortcut {
  /** 人类可读的快捷键名称（用于 help 文档生成）。 */
  display: string;
  /** 简短描述这个快捷键做什么。 */
  description?: string;
  /** 判定是否匹配本次按键事件。 */
  match: (e: KeyboardEvent) => boolean;
  /** 当 settings 页面打开时，是否跳过本快捷键？默认 false。 */
  skipInSettings?: boolean;
  /**
   * 处理函数。返回 `false` 表示 NOT 调用 preventDefault/stopPropagation
   * （让浏览器或下层组件继续看到这个事件）；其他返回值都会拦截。
   */
  handler: (e: KeyboardEvent) => void | false;
}

/**
 * 把一组 shortcut 挂到 window 的 keydown 事件上（capture 优先），
 * 返回 detach 函数（适合 onMount return）。
 */
export function attachShortcuts(shortcuts: readonly Shortcut[]): () => void {
  const onKeydown = (e: KeyboardEvent) => {
    for (const s of shortcuts) {
      if (s.skipInSettings && app.settingsActive()) continue;
      if (!s.match(e)) continue;
      const result = s.handler(e);
      if (result !== false) {
        e.preventDefault();
        e.stopPropagation();
      }
      return;
    }
  };
  window.addEventListener("keydown", onKeydown, { capture: true });
  return () => window.removeEventListener("keydown", onKeydown, { capture: true });
}

/** 同上，但挂 keyup。 */
export function attachKeyup(handler: (e: KeyboardEvent) => void): () => void {
  window.addEventListener("keyup", handler, { capture: true });
  return () => window.removeEventListener("keyup", handler, { capture: true });
}
