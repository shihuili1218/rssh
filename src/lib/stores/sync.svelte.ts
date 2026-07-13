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
  remote: SyncMetadata | null;
  error: string | null;
  pulled: boolean;
}

export interface SyncCheckResult {
  local: SyncMetadata;
  github: SyncProviderStatus;
  webdav: SyncProviderStatus;
}

interface AutoPullStatus {
  github: boolean;
  webdav: boolean;
}

interface AutoPullState {
  value: AutoPullStatus | null;
  loading: boolean;
  inFlight: Promise<void> | null;
}

interface CheckRequest {
  inFlight: Promise<void> | null;
  pending: boolean;
}

const emptyProviders = (): Record<SyncSource, SyncProviderStatus | null> => ({
  github: null,
  webdav: null,
});

let _local = $state<SyncMetadata | null>(null);
let _localRevision = 0;
let _configurationRevision = $state(0);
let _autoPull = $state<AutoPullState>({
  value: null,
  loading: false,
  inFlight: null,
});
let _providers = $state(emptyProviders());
const _localRequest: CheckRequest = { inFlight: null, pending: false };
const _request: CheckRequest = { inFlight: null, pending: false };

let _scheduled = false;
let _initialTimer: ReturnType<typeof setTimeout> | null = null;
let _intervalTimer: ReturnType<typeof setInterval> | null = null;

export function localMetadata(): SyncMetadata | null {
  return _local;
}

export function configurationRevision(): number {
  return _configurationRevision;
}

export function invalidateConfiguration(): void {
  _configurationRevision += 1;
}

export function providerStatus(source: SyncSource): SyncProviderStatus | null {
  return _providers[source];
}

export function autoPullEnabled(source: SyncSource): boolean {
  return _autoPull.value?.[source] ?? false;
}

export function autoPullStatusLoaded(): boolean {
  return _autoPull.value !== null && !_autoPull.loading;
}

export function loadAutoPullStatus(): Promise<void> {
  if (_autoPull.inFlight !== null) return _autoPull.inFlight;

  _autoPull.loading = true;
  const task = invoke<AutoPullStatus>("get_sync_auto_pull_status").then(
    (status) => {
      _autoPull.value = status;
    },
  );
  _autoPull.inFlight = task;
  const clear = () => {
    if (_autoPull.inFlight === task) {
      _autoPull.inFlight = null;
      _autoPull.loading = false;
    }
  };
  void task.then(clear, clear);
  return task;
}

export async function saveAutoPull(
  source: SyncSource,
  enabled: boolean,
  password: string | null,
): Promise<void> {
  const loading = _autoPull.inFlight;
  if (loading !== null) await loading;
  await invoke("set_sync_auto_pull", { provider: source, enabled, password });
  _autoPull.value = {
    ...(_autoPull.value ?? { github: false, webdav: false }),
    [source]: enabled,
  };
}

export function hasRemoteUpdate(source: SyncSource): boolean {
  const remote = _providers[source]?.remote;
  return (
    _local !== null &&
    remote !== null &&
    remote !== undefined &&
    remote.version > _local.version
  );
}

export function hasLocalUpdate(source: SyncSource): boolean {
  const remote = _providers[source]?.remote;
  return (
    _local !== null &&
    remote !== null &&
    remote !== undefined &&
    _local.version > remote.version
  );
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

function publishLocal(metadata: SyncMetadata): void {
  // The local fingerprint catches represented configuration changes, including
  // many partial imports. The explicit `pulled` fallback below also covers
  // successful imports of categories excluded by this device's sync filters.
  const configurationChanged =
    _local !== null && _local.config_digest !== metadata.config_digest;
  _local = metadata;
  _localRevision += 1;
  if (configurationChanged) invalidateConfiguration();
}

function sameMetadata(
  left: SyncMetadata | null,
  right: SyncMetadata,
): boolean {
  return (
    left !== null &&
    left.version === right.version &&
    left.config_digest === right.config_digest
  );
}

async function drainLocalRefreshes(): Promise<void> {
  let lastError: unknown = null;
  do {
    _localRequest.pending = false;
    try {
      publishLocal(await invoke<SyncMetadata>("sync_refresh_local_metadata"));
      lastError = null;
    } catch (error) {
      lastError = error;
    }
  } while (_localRequest.pending);
  if (lastError !== null) throw lastError;
}

export function refreshLocalMetadata(): Promise<void> {
  _localRequest.pending = true;
  if (_localRequest.inFlight === null) {
    const task = drainLocalRefreshes();
    _localRequest.inFlight = task;
    const clear = () => {
      if (_localRequest.inFlight === task) _localRequest.inFlight = null;
    };
    void task.then(clear, clear);
  }
  return _localRequest.inFlight;
}

function reconcileProvider(
  previous: SyncProviderStatus | null,
  next: SyncProviderStatus,
): SyncProviderStatus {
  if (next.error !== null && next.remote === null && previous?.remote) {
    return { ...next, remote: previous.remote };
  }
  return next;
}

async function runCheckPass(): Promise<void> {
  try {
    await refreshLocalMetadata();
    const localRevision = _localRevision;
    const result = await invoke<SyncCheckResult>("sync_check_remotes");
    const revisionBeforeReconcile = _configurationRevision;
    // The backend returns the snapshot from after all automatic-pull attempts,
    // including partial imports that report an error. If another local refresh
    // completed while the network request was running, settle the ordering with
    // one final local read instead of replacing a newer snapshot with an older
    // response.
    if (
      _localRevision !== localRevision &&
      !sameMetadata(_local, result.local)
    ) {
      await refreshLocalMetadata();
    } else {
      publishLocal(result.local);
    }
    _providers = {
      github: reconcileProvider(_providers.github, result.github),
      webdav: reconcileProvider(_providers.webdav, result.webdav),
    };
    if (
      (result.github.pulled || result.webdav.pulled) &&
      _configurationRevision === revisionBeforeReconcile
    ) {
      invalidateConfiguration();
    }
  } catch (error) {
    console.error("sync.runCheck failed:", error);
  }
}

async function drainChecks(): Promise<void> {
  do {
    _request.pending = false;
    await runCheckPass();
  } while (_request.pending);
}

export function runCheck(_opts?: { silent?: boolean }): Promise<void> {
  _request.pending = true;
  if (_request.inFlight === null) {
    const task = drainChecks();
    _request.inFlight = task;
    void task.then(
      () => {
        if (_request.inFlight === task) _request.inFlight = null;
      },
      () => {
        if (_request.inFlight === task) _request.inFlight = null;
      },
    );
  }
  return _request.inFlight;
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
