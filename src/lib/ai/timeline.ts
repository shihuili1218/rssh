/**
 * Persisted-timeline restore. The stored blob is whatever ChatItem[] looked
 * like at the last autosave — written by a live session, so it can contain
 * live-only states that must not be resurrected verbatim:
 *
 * - assistant `streaming: true` → a blinking cursor with no stream behind it
 * - command cards with neither result nor rejection → approval buttons whose
 *   sentinel the restarted backend has never heard of
 *
 * Both are normalized here. Unknown/corrupt entries are dropped, not thrown:
 * a damaged blob should degrade to a shorter timeline, never block resume.
 */
import type { ChatItem } from "./types.ts";

function isStr(v: unknown): v is string {
  return typeof v === "string";
}

/** Per-kind shape check — the fields the templates dereference unconditionally
 *  (renderMarkdown(text), fmt(at), CommandConfirmDialog's cmd.*). A known kind
 *  with a mangled body must be dropped here, not crash the panel at render
 *  time. Deliberately NOT a full schema: blobs are written by our own
 *  serializer, this guards against crashes and visible junk ("Invalid Date"),
 *  not against every cosmetic defect of a hand-corrupted row. */
function isRenderable(item: ChatItem): boolean {
  if (typeof item.at !== "number") return false;
  switch (item.kind) {
    case "user":
    case "error":
    case "note":
      return isStr(item.text);
    case "assistant":
      return isStr(item.id) && isStr(item.text);
    case "command":
      return (
        !!item.cmd && typeof item.cmd === "object" &&
        isStr(item.cmd.id) && isStr(item.cmd.cmd)
      );
    default:
      return false;
  }
}

export function restoreTimeline(json: string, staleCommandReason: string): ChatItem[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];

  const items: ChatItem[] = [];
  for (const raw of parsed) {
    if (!raw || typeof raw !== "object") continue;
    const item = raw as ChatItem;
    if (!isRenderable(item)) continue;
    if (item.kind === "assistant") {
      // Mirror the live assistant_message_end rule: an empty non-cancelled
      // bubble (the placeholder pushed at message_start, persisted by a
      // mid-stream crash, or a pure tool-use turn) is removed there — restore
      // must drop it too, or it renders as a permanent "…".
      if (!item.text && !item.cancelled) continue;
      item.streaming = false;
    } else if (item.kind === "command") {
      // The one method-call crash vector in CommandConfirmDialog: a truthy
      // non-string diff hits `cmd.diff.split()`. Strip rather than drop —
      // the card is still meaningful without its diff preview. Every other
      // cmd/result/rejected field is either plain-rendered (Svelte renders
      // undefined as "") or crash-safe, and the action-button branch is
      // unreachable for restored cards (stale-marking below guarantees
      // result|rejected).
      if (item.cmd.diff !== undefined && !isStr(item.cmd.diff)) {
        delete item.cmd.diff;
      }
      if (!item.result && !item.rejected) {
        item.rejected = { reason: staleCommandReason };
      }
    }
    items.push(item);
  }
  return items;
}
