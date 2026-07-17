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

export interface AiTerminalMutation {
  kind: string;
  payload: unknown;
}

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
    if (item.kind === "user") {
      // client_id/client_seq only correlate optimistic mutations in the live
      // renderer. Old versions persisted them, but carrying those sequence
      // numbers into a new process would make a later context clear compare
      // against a counter from the wrong runtime.
      items.push({ kind: "user", text: item.text, at: item.at });
      continue;
    }
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

/** Replay the backend actor's canonical close-time terminal events onto a
 * private timeline snapshot. The actor records these before emitting them, and
 * prepare-stop returns them only after the actor drains. Event callbacks may
 * therefore arrive before or after the invoke reply without changing the
 * persisted result. Every mutation is keyed by message/card id and idempotent. */
export function applyTerminalMutations(
  source: ChatItem[],
  mutations: readonly AiTerminalMutation[],
): ChatItem[] {
  let items = source;
  for (const mutation of mutations) {
    if (!mutation.payload || typeof mutation.payload !== "object") continue;
    const payload = mutation.payload as Record<string, unknown>;
    if (!isStr(payload.id)) continue;

    if (mutation.kind === "assistant_message_end") {
      if (!isStr(payload.text)) continue;
      const index = findLastIndex(items, (item) =>
        item.kind === "assistant" && item.id === payload.id);
      if (index < 0) {
        if (payload.text || payload.cancelled === true) {
          items = [...items, {
            kind: "assistant",
            id: payload.id,
            text: payload.text,
            at: Date.now(),
            streaming: false,
            cancelled: payload.cancelled === true,
          }];
        }
        continue;
      }
      const item = items[index];
      if (item.kind !== "assistant") continue;
      if (!payload.text && payload.cancelled !== true) {
        items = [...items.slice(0, index), ...items.slice(index + 1)];
        continue;
      }
      const replacement: ChatItem = {
        ...item,
        text: payload.text || item.text,
        streaming: false,
        cancelled: payload.cancelled === true,
      };
      items = replaceAt(items, index, replacement);
      continue;
    }

    const index = findLastIndex(items, (item) =>
      item.kind === "command" && item.cmd.id === payload.id);
    if (index < 0) continue;
    const item = items[index];
    if (item.kind !== "command") continue;

    if (mutation.kind === "command_rejected") {
      if (!isStr(payload.reason)) continue;
      items = replaceAt(items, index, {
        ...item,
        result: undefined,
        rejected: { reason: payload.reason },
      });
      continue;
    }
    if (mutation.kind !== "command_completed") continue;
    if (
      typeof payload.exit_code !== "number"
      || typeof payload.timed_out !== "boolean"
      || typeof payload.duration_ms !== "number"
      || !isStr(payload.output)
      || typeof payload.original_bytes !== "number"
      || typeof payload.truncated_bytes !== "number"
    ) continue;
    items = replaceAt(items, index, {
      ...item,
      rejected: undefined,
      result: {
        id: payload.id,
        exit_code: payload.exit_code,
        timed_out: payload.timed_out,
        early_terminated: payload.early_terminated === true,
        duration_ms: payload.duration_ms,
        output: payload.output,
        original_bytes: payload.original_bytes,
        truncated_bytes: payload.truncated_bytes,
      },
    });
  }
  // prepare-stop has drained the actor: no stream can still produce deltas.
  // If the actor panicked before recording its terminal event, persist the
  // partial bubble as cancelled instead of resurrecting a permanent cursor.
  return items.map((item) => item.kind === "assistant" && item.streaming
    ? { ...item, streaming: false, cancelled: true }
    : item);
}

function findLastIndex(
  items: ChatItem[],
  predicate: (item: ChatItem) => boolean,
): number {
  for (let index = items.length - 1; index >= 0; index--) {
    if (predicate(items[index])) return index;
  }
  return -1;
}

function replaceAt(items: ChatItem[], index: number, item: ChatItem): ChatItem[] {
  return [...items.slice(0, index), item, ...items.slice(index + 1)];
}
