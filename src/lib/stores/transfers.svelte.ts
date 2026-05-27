/**
 * Background SFTP transfers — independent of any open SftpBrowser overlay.
 *
 * Each transfer opens its own sftp subsystem on a live SSH session, runs to
 * completion, then closes. Closing the browser does not affect anything here:
 * we never share its sftp_id.
 *
 * 进度事件遵守 AGENT.md R1：`sftp:progress:{transfer_id}` 三段式。每条 transfer
 * 起一个 listener、跑完即解绑——避免一个全局 listener 被全部 transfer 喂事件。
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { errMsg } from "../i18n/index.svelte.ts";

export type TransferKind = "download" | "upload";
export type TransferStatus = "queued" | "running" | "done" | "failed" | "cancelled";

/// Global concurrency cap: at most N running transfers at a time; the rest
/// stay queued. Default 10 is a pragmatic sweet spot — comfortably within
/// SSH channel limits (typically 10-100) while keeping per-connection
/// bandwidth share reasonable. User-overridable in Shell settings, bounded
/// [MIN, MAX] —— 太低用户搬大量小文件慢得难受；太高撞 SSH 服务端 channel 限或
/// 把单条 SSH 连接带宽切成碎片。
const DEFAULT_MAX_CONCURRENT = 10;
const MIN_MAX_CONCURRENT = 1;
const MAX_MAX_CONCURRENT = 20;
const SETTING_KEY_MAX_CONCURRENT = "sftp_max_concurrent";

let _maxConcurrent = $state(DEFAULT_MAX_CONCURRENT);
/// 标记"本地已经显式 set 过"。loadMaxConcurrent 是 fire-and-forget，可能晚于
/// 用户在 Settings 里的保存才解出 DB 旧值——那次旧值不能反过来盖掉刚 save 的
/// 新值。set 之后置 true，后续 load 看到 true 就跳过覆盖。
let _setLocally = false;

export function maxConcurrent(): number { return _maxConcurrent; }
export function maxConcurrentBounds(): { min: number; max: number; def: number } {
  return { min: MIN_MAX_CONCURRENT, max: MAX_MAX_CONCURRENT, def: DEFAULT_MAX_CONCURRENT };
}

/** App 启动时调一次 —— 从 DB 拉持久化值覆盖默认。失败保留默认值，不阻塞。
 *  若期间用户已经手动 set 过（_setLocally==true），就别用 DB 旧值反向覆盖。 */
export async function loadMaxConcurrent(): Promise<void> {
  try {
    const v = await invoke<string | null>("get_setting", { key: SETTING_KEY_MAX_CONCURRENT });
    if (_setLocally) return;
    if (v) {
      const n = parseInt(v, 10);
      if (Number.isFinite(n) && n >= MIN_MAX_CONCURRENT && n <= MAX_MAX_CONCURRENT) {
        _maxConcurrent = n;
      }
    }
  } catch {
    // setting backend 没就绪等情况：留默认 10 即可
  }
}

/** ShellSettings 改值时调。clamp 到 [MIN, MAX]，先写 DB 成功再改 runtime
 *  —— DB 失败时 runtime 不动，避免重启后两者不一致。最后 kick 一下队列让
 *  调大后多出来的额度立刻消化掉 queued 项；调小不主动 cancel 正在跑的。 */
export async function setMaxConcurrent(n: number): Promise<void> {
  const clamped = Math.max(MIN_MAX_CONCURRENT, Math.min(MAX_MAX_CONCURRENT, Math.floor(n)));
  await invoke("set_setting", { key: SETTING_KEY_MAX_CONCURRENT, value: String(clamped) });
  _maxConcurrent = clamped;
  _setLocally = true;
  while (runningCount() < _maxConcurrent) {
    const before = runningCount();
    promoteNextQueued();
    if (runningCount() === before) break; // 没 queued 了
  }
}

/// 后端用这个 i18n code 标记"用户主动取消"。errStr 包含 `__rssh_err__|{"code":"transfer_cancelled",...}`
/// 时识别为 cancelled。对应 src-tauri/src/ssh/sftp.rs::CANCELLED_CODE，前后端必须保持一致。
const CANCELLED_TAG = "transfer_cancelled";

export interface Transfer {
  id: string;
  kind: TransferKind;
  sessionId: string;
  remotePath: string;
  localPath: string;
  total: number;
  transferred: number;
  status: TransferStatus;
  error?: string;
  startedAt: number;
  finishedAt?: number;
}

let _list = $state<Transfer[]>([]);

interface ProgressPayload {
  transferred: number;
  total: number;
}

async function attachProgressListener(id: string): Promise<UnlistenFn> {
  return await listen<ProgressPayload>(`sftp:progress:${id}`, (ev) => {
    const t = _list.find((x) => x.id === id);
    if (!t) return;
    t.transferred = ev.payload.transferred;
    t.total = ev.payload.total;
  });
}

export function list(): Transfer[] {
  return _list;
}

export function activeCount(): number {
  return _list.filter((t) => t.status === "running" || t.status === "queued").length;
}

/** Look up a transfer in the proxied store; needed so mutations trigger reactivity. */
function find(id: string): Transfer | undefined {
  return _list.find((x) => x.id === id);
}

function runningCount(): number {
  return _list.filter((t) => t.status === "running").length;
}

/// Promote the queued transfer matching `id` to running and kick it off.
/// Returns early when already running/cancelled/done/etc. When the capacity
/// is full, the entry remains queued and is picked up by the next
/// `promoteNextQueued()` scan in some other transfer's `finally` block.
function tryDispatch(id: string): void {
  if (runningCount() >= _maxConcurrent) return;
  const t = find(id);
  if (!t || t.status !== "queued") return;
  t.status = "running";
  t.startedAt = Date.now();
  void runTransfer(id);
}

/// Called from runTransfer's finally: scan for the oldest queued entry and
/// promote one. Iterates from the tail because `startDownload` unshifts new
/// transfers to the front, so the oldest queued entry lives at the end.
function promoteNextQueued(): void {
  if (runningCount() >= _maxConcurrent) return;
  for (let i = _list.length - 1; i >= 0; i--) {
    if (_list[i].status === "queued") {
      tryDispatch(_list[i].id);
      return;
    }
  }
}

async function runTransfer(id: string): Promise<void> {
  const snap = find(id);
  if (!snap) return;
  let mySftpId: string | null = null;
  // listener 必须先 attach 后再 invoke：避免后端 emit 早于前端 listen 时丢首批事件。
  // 如果 listen() 本身就失败（事件系统没就绪等罕见情况），把 transfer 标 failed
  // 并退出 —— 不能让 attachProgressListener 抛出未捕获 rejection（caller 只 void 了）。
  let unlisten: UnlistenFn;
  try {
    unlisten = await attachProgressListener(id);
  } catch (e) {
    const cur = find(id);
    if (cur) {
      cur.status = "failed";
      cur.error = errMsg(e);
      cur.finishedAt = Date.now();
    }
    // This early-return bypasses the try/finally below, so we must promote
    // the next queued transfer here — otherwise this slot stays "taken" by
    // a failed transfer and the queue stalls until something else finishes.
    promoteNextQueued();
    return;
  }
  try {
    mySftpId = await invoke<string>("sftp_connect_session", { sessionId: snap.sessionId });
    if (snap.kind === "download") {
      await invoke("sftp_download_to", {
        sftpId: mySftpId,
        remotePath: snap.remotePath,
        localPath: snap.localPath,
        transferId: id,
      });
    } else {
      await invoke("sftp_upload_from", {
        sftpId: mySftpId,
        localPath: snap.localPath,
        remotePath: snap.remotePath,
        transferId: id,
      });
    }
    const cur = find(id);
    if (cur) {
      cur.status = "done";
      if (cur.total > 0) cur.transferred = cur.total;
      cur.finishedAt = Date.now();
    }
  } catch (e) {
    // 先用裸 String(e) 识别 cancel 标记（协议串），再用 errMsg(e) 翻译展示。
    const rawStr = String(e);
    const cur = find(id);
    if (cur) {
      cur.status = rawStr.includes(CANCELLED_TAG) ? "cancelled" : "failed";
      cur.error = errMsg(e);
      cur.finishedAt = Date.now();
    }
  } finally {
    unlisten();
    if (mySftpId) invoke("sftp_close", { sftpId: mySftpId }).catch(() => {});
    promoteNextQueued();
  }
}

export async function startDownload(args: {
  sessionId: string;
  remotePath: string;
  localPath: string;
  sizeHint?: number;
}): Promise<string> {
  const id = crypto.randomUUID();
  const t: Transfer = {
    id,
    kind: "download",
    sessionId: args.sessionId,
    remotePath: args.remotePath,
    localPath: args.localPath,
    total: args.sizeHint ?? 0,
    transferred: 0,
    status: "queued",
    startedAt: Date.now(),
  };
  _list.unshift(t);
  tryDispatch(id);
  return id;
}

export async function startUpload(args: {
  sessionId: string;
  localPath: string;
  remotePath: string;
}): Promise<string> {
  const id = crypto.randomUUID();
  const t: Transfer = {
    id,
    kind: "upload",
    sessionId: args.sessionId,
    remotePath: args.remotePath,
    localPath: args.localPath,
    total: 0,
    transferred: 0,
    status: "queued",
    startedAt: Date.now(),
  };
  _list.unshift(t);
  tryDispatch(id);
  return id;
}

export async function retry(id: string): Promise<void> {
  const t = find(id);
  if (!t || t.status === "running" || t.status === "queued") return;
  t.status = "queued";
  t.error = undefined;
  t.transferred = 0;
  t.startedAt = Date.now();
  t.finishedAt = undefined;
  tryDispatch(id);
}

/** Cancel a transfer.
 *  - queued: not running, mark cancelled directly without any IPC.
 *  - running: raise the backend cancel flag; runTransfer's catch branch will
 *    flip the status when the streaming loop exits. */
export async function cancel(id: string): Promise<void> {
  const t = find(id);
  if (!t) return;
  if (t.status === "queued") {
    t.status = "cancelled";
    t.finishedAt = Date.now();
    return;
  }
  if (t.status !== "running") return;
  try {
    await invoke("sftp_cancel_transfer", { transferId: id });
  } catch (e) {
    console.error("[transfers] cancel failed:", e);
  }
}

export function remove(id: string): void {
  _list = _list.filter((t) => t.id !== id);
}

export function clearFinished(): void {
  _list = _list.filter((t) => t.status === "running" || t.status === "queued");
}
