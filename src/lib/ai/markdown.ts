/**
 * LLM 回复的 markdown 渲染。
 *
 * 安全：marked 解析 → DOMPurify 净化（防 XSS / 防 prompt injection 引诱用户点恶意链接）。
 * 链接保留 href 但加 rel=noopener；UI 上点击仍会经过 AppShell 的禁链处理（决议 1.9）。
 */

import { marked } from "marked";
import DOMPurify from "dompurify";

marked.setOptions({
  gfm: true,        // GitHub-flavored markdown（表格、删除线、任务列表）
  breaks: false,    // 单换行不强制 <br>，保持段落紧凑（连续两个换行才换段）
});

DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") {
    node.setAttribute("rel", "noopener noreferrer");
    node.setAttribute("target", "_blank");
    // AppShell 的 onclick 拦截会再二次确认（见 1.9 决议）
  }
});

export function renderMarkdown(text: string): string {
  const html = marked.parse(text, { async: false }) as string;
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: ["script", "style", "iframe", "object", "embed", "form", "input", "button"],
    FORBID_ATTR: ["onerror", "onclick", "onload", "onmouseover"],
  });
}
