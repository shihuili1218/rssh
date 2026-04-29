/**
 * Background SFTP transfers — independent of any open SftpBrowser overlay.
 *
 * Each transfer opens its own sftp subsystem on a live SSH session, runs to
 * completion, then closes. Closing the browser does not affect anything here:
 * we never share its sftp_id.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type TransferKind = "download" | "upload";
export type TransferStatus = "running" | "done" | "failed";

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
let _unlisten: UnlistenFn | null = null;
let _initPromise: Promise<void> | null = null;

interface ProgressPayload {
  id: string;
  transferred: number;
  total: number;
}

async function ensureListener(): Promise<void> {
  if (_unlisten) return;
  if (_initPromise) return _initPromise;
  _initPromise = (async () => {
    _unlisten = await listen<ProgressPayload>("sftp:progress", (ev) => {
      const t = _list.find((x) => x.id === ev.payload.id);
      if (!t) return;
      t.transferred = ev.payload.transferred;
      t.total = ev.payload.total;
    });
  })();
  return _initPromise;
}

export function list(): Transfer[] {
  return _list;
}

export function activeCount(): number {
  return _list.filter((t) => t.status === "running").length;
}

/** Look up a transfer in the proxied store; needed so mutations trigger reactivity. */
function find(id: string): Transfer | undefined {
  return _list.find((x) => x.id === id);
}

async function runTransfer(id: string): Promise<void> {
  const snap = find(id);
  if (!snap) return;
  let mySftpId: string | null = null;
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
    const cur = find(id);
    if (cur) {
      cur.status = "failed";
      cur.error = String(e);
      cur.finishedAt = Date.now();
    }
  } finally {
    if (mySftpId) invoke("sftp_close", { sftpId: mySftpId }).catch(() => {});
  }
}

export async function startDownload(args: {
  sessionId: string;
  remotePath: string;
  localPath: string;
  sizeHint?: number;
}): Promise<string> {
  await ensureListener();
  const id = crypto.randomUUID();
  const t: Transfer = {
    id,
    kind: "download",
    sessionId: args.sessionId,
    remotePath: args.remotePath,
    localPath: args.localPath,
    total: args.sizeHint ?? 0,
    transferred: 0,
    status: "running",
    startedAt: Date.now(),
  };
  _list = [t, ..._list];
  void runTransfer(id);
  return id;
}

export async function startUpload(args: {
  sessionId: string;
  localPath: string;
  remotePath: string;
}): Promise<string> {
  await ensureListener();
  const id = crypto.randomUUID();
  const t: Transfer = {
    id,
    kind: "upload",
    sessionId: args.sessionId,
    remotePath: args.remotePath,
    localPath: args.localPath,
    total: 0,
    transferred: 0,
    status: "running",
    startedAt: Date.now(),
  };
  _list = [t, ..._list];
  void runTransfer(id);
  return id;
}

export async function retry(id: string): Promise<void> {
  const t = find(id);
  if (!t || t.status === "running") return;
  await ensureListener();
  t.status = "running";
  t.error = undefined;
  t.transferred = 0;
  t.startedAt = Date.now();
  t.finishedAt = undefined;
  void runTransfer(id);
}

export function remove(id: string): void {
  _list = _list.filter((t) => t.id !== id);
}

export function clearFinished(): void {
  _list = _list.filter((t) => t.status === "running");
}
