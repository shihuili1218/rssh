import { invoke } from "@tauri-apps/api/core";

const INITIAL_DELAY_MS = 10_000;
const INTERVAL_MS = 60 * 60 * 1000;

export type SyncSource = "github" | "webdav";
export type ManualSyncAction = "push" | "pull";

export function commandFor(
  source: SyncSource,
  action: ManualSyncAction,
): `${SyncSource}_${ManualSyncAction}` {
  return `${source}_${action}`;
}

export interface SyncMetadata {
  version: number;
  config_digest: string;
}

export interface SyncProviderStatus {
  enabled: boolean;
  auto_pull: boolean;
  remote: SyncMetadata | null;
  error: string | null;
  pulled: boolean;
}

export interface SyncCheckResult {
  local: SyncMetadata;
  github: SyncProviderStatus;
  webdav: SyncProviderStatus;
}

export type SyncState =
  | { kind: "unknown" }
  | { kind: "checking"; previous: SyncCheckResult | null }
  | { kind: "ready"; result: SyncCheckResult }
  | { kind: "error"; error: unknown };

let _state = $state<SyncState>({ kind: "unknown" });
let _local = $state<SyncMetadata | null>(null);
let _localRevision = 0;
let _localInFlight: Promise<void> | null = null;
let _autoPullRevision: Record<SyncSource, number> = { github: 0, webdav: 0 };
let _inFlight: Promise<void> | null = null;
let _scheduled = false;
let _initialTimer: ReturnType<typeof setTimeout> | null = null;
let _intervalTimer: ReturnType<typeof setInterval> | null = null;

export function state(): SyncState {
  return _state;
}

function visibleResult(): SyncCheckResult | null {
  if (_state.kind === "ready") return _state.result;
  if (_state.kind === "checking") return _state.previous;
  return null;
}

export function localMetadata(): SyncMetadata | null {
  return _local ?? visibleResult()?.local ?? null;
}

export function providerStatus(source: SyncSource): SyncProviderStatus | null {
  return visibleResult()?.[source] ?? null;
}

export function autoPullEnabled(source: SyncSource): boolean {
  return providerStatus(source)?.auto_pull ?? false;
}

export async function saveAutoPull(
  source: SyncSource,
  enabled: boolean,
  password: string | null,
): Promise<void> {
  await invoke("set_sync_auto_pull", { provider: source, enabled, password });
  _autoPullRevision[source] += 1;

  const result = visibleResult();
  if (!result) return;
  const updated: SyncCheckResult = {
    ...result,
    [source]: { ...result[source], auto_pull: enabled },
  };
  if (_state.kind === "checking") {
    _state = { kind: "checking", previous: updated };
  } else {
    _state = { kind: "ready", result: updated };
  }
}

export function hasRemoteUpdate(source: SyncSource): boolean {
  const local = localMetadata();
  const remote = providerStatus(source)?.remote;
  return local !== null && remote !== null && remote.version > local.version;
}

export function hasLocalUpdate(source: SyncSource): boolean {
  const local = localMetadata();
  const remote = providerStatus(source)?.remote;
  return local !== null && remote !== null && local.version > remote.version;
}

export function anyRemoteUpdate(): boolean {
  return hasRemoteUpdate("github") || hasRemoteUpdate("webdav");
}

export function anyVersionDifference(): boolean {
  return (
    anyRemoteUpdate() ||
    hasLocalUpdate("github") ||
    hasLocalUpdate("webdav")
  );
}

export function refreshLocalMetadata(): Promise<void> {
  if (_localInFlight) return _localInFlight;

  const task = invoke<SyncMetadata>("sync_refresh_local_metadata").then((local) => {
    _local = local;
    _localRevision += 1;
  });
  _localInFlight = task;
  const clear = () => {
    if (_localInFlight === task) _localInFlight = null;
  };
  void task.then(clear, clear);
  return task;
}

/** A config write that overlaps an older local snapshot needs one fresh pass
 * after that snapshot, otherwise the newer write can be missed. */
export async function refreshLocalAfterMutation(): Promise<void> {
  const active = _localInFlight;
  if (active) await active;
  await refreshLocalMetadata();
}

export function runCheck(opts?: { silent?: boolean }): Promise<void> {
  // Every explicit check request refreshes the local snapshot, even when an
  // older remote pass is still running. The remote work remains coalesced.
  const localRefresh = refreshLocalMetadata();
  const active = _inFlight;
  if (active) {
    return localRefresh.then(
      () => active,
      (error) => {
        console.error("sync.refreshLocalMetadata failed:", error);
        if (!opts?.silent) _state = { kind: "error", error };
      },
    );
  }

  const previous = _state;
  const previousResult = visibleResult();
  const autoPullRevisionAtStart = { ..._autoPullRevision };
  _state = { kind: "checking", previous: previousResult };
  const task = (async () => {
    try {
      await localRefresh;
      const localRevisionAtRemoteStart = _localRevision;
      const result = await invoke<SyncCheckResult>("sync_check_remotes");
      const current = visibleResult();
      const reconciled = { ...result };
      for (const source of ["github", "webdav"] as const) {
        if (
          current !== null &&
          autoPullRevisionAtStart[source] !== _autoPullRevision[source]
        ) {
          reconciled[source] = {
            ...result[source],
            auto_pull: current[source].auto_pull,
          };
        }
      }
      if (
        localRevisionAtRemoteStart === _localRevision ||
        reconciled.github.pulled ||
        reconciled.webdav.pulled
      ) {
        _local = reconciled.local;
      } else if (_local) {
        reconciled.local = _local;
      }
      _state = { kind: "ready", result: reconciled };
    } catch (error) {
      console.error("sync.runCheck failed:", error);
      if (opts?.silent) {
        const current = visibleResult();
        _state = current ? { kind: "ready", result: current } : previous;
      } else {
        _state = { kind: "error", error };
      }
    }
  })();
  _inFlight = task;
  const clear = () => {
    if (_inFlight === task) _inFlight = null;
  };
  void task.then(clear, clear);
  return task;
}

/** A state-changing sync action must not reuse a check that started before it.
 * Wait for that older pass, then start (or coalesce onto) the next fresh pass. */
export async function refreshAfterMutation(): Promise<void> {
  const active = _inFlight;
  if (active) await active;
  await runCheck({ silent: true });
}

export function startBackgroundChecks(): void {
  if (_scheduled) return;
  _scheduled = true;
  _initialTimer = setTimeout(() => {
    _initialTimer = null;
    void runCheck({ silent: true });
    _intervalTimer = setInterval(
      () => void runCheck({ silent: true }),
      INTERVAL_MS,
    );
  }, INITIAL_DELAY_MS);
}

export function stopBackgroundChecks(): void {
  if (_initialTimer !== null) {
    clearTimeout(_initialTimer);
    _initialTimer = null;
  }
  if (_intervalTimer !== null) {
    clearInterval(_intervalTimer);
    _intervalTimer = null;
  }
  _scheduled = false;
}
