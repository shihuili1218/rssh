/**
 * 极简 i18n：locale 状态 + `t()` 翻译函数。
 *
 * - 启动时自动检测：localStorage > navigator.language > 'en'
 * - `setLocale()` 会写回 localStorage
 * - 翻译函数读 `_locale` 这个 $state，所以 Svelte 模板里的 `{t('foo')}`
 *   在切换语言时会自动重渲染。
 */

import en, { type MessageKey, type Messages } from "./locales/en";
import zh from "./locales/zh";

export type Locale = "en" | "zh";

const CATALOGS: Record<Locale, Messages> = { en, zh };
const STORAGE_KEY = "rssh.locale";

function detectLocale(): Locale {
  const stored = localStorage.getItem(STORAGE_KEY) as Locale | null;
  if (stored && stored in CATALOGS) return stored;
  const sys = (navigator.language || "en").toLowerCase();
  return sys.startsWith("zh") ? "zh" : "en";
}

let _locale = $state<Locale>(detectLocale());

export function locale(): Locale {
  return _locale;
}

export function setLocale(next: Locale): void {
  _locale = next;
  localStorage.setItem(STORAGE_KEY, next);
}

/**
 * 翻译函数。如果 key 在当前语言里找不到，回退到英文；都找不到就原样返回 key。
 *
 * `params` 用于 `{name}` 风格的占位符替换。
 */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const cat = CATALOGS[_locale];
  const msg = cat[key] ?? CATALOGS.en[key] ?? (key as string);
  if (!params) return msg;
  return msg.replace(/\{(\w+)\}/g, (_, k) => {
    const v = params[k];
    return v === undefined ? `{${k}}` : String(v);
  });
}

/** 用于 lang picker 的元数据。 */
export const AVAILABLE_LOCALES: { code: Locale; label: string }[] = [
  { code: "en", label: "English" },
  { code: "zh", label: "中文" },
];
