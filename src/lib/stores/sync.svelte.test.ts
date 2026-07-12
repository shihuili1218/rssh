import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

type Metadata = { version: number; config_digest: string };
type ProviderStatus = {
  enabled: boolean;
  auto_pull: boolean;
  remote: Metadata | null;
  error: string | null;
  pulled: boolean;
};
type CheckResult = {
  local: Metadata;
  github: ProviderStatus;
  webdav: ProviderStatus;
};

const result: CheckResult = {
  local: { version: 5, config_digest: "local-5" },
  github: {
    enabled: true,
    auto_pull: false,
    remote: { version: 6, config_digest: "github-6" },
    error: null,
    pulled: false,
  },
  webdav: {
    enabled: false,
    auto_pull: false,
    remote: null,
    error: null,
    pulled: false,
  },
};

function queueCheck(next: CheckResult): void {
  invokeMock
    .mockResolvedValueOnce(next.local)
    .mockResolvedValueOnce(next);
}

function commandCallCount(command: string): number {
  return invokeMock.mock.calls.filter(([name]) => name === command).length;
}

function mockSuccessfulChecks(next: CheckResult = result): void {
  invokeMock.mockImplementation((command: string) => {
    if (command === "sync_refresh_local_metadata") {
      return Promise.resolve(next.local);
    }
    if (command === "sync_check_remotes") {
      return Promise.resolve(next);
    }
    return Promise.reject(new Error(`unexpected command: ${command}`));
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function loadSyncStore() {
  vi.resetModules();
  return import("./sync.svelte.ts");
}

beforeEach(() => {
  invokeMock.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("sync store", () => {
  it("maps a manual action to exactly one provider command", async () => {
    const sync = await loadSyncStore();

    expect(sync.commandFor("github", "push")).toBe("github_push");
    expect(sync.commandFor("github", "pull")).toBe("github_pull");
    expect(sync.commandFor("webdav", "push")).toBe("webdav_push");
    expect(sync.commandFor("webdav", "pull")).toBe("webdav_pull");
  });

  it("publishes a completed check through its public metadata selectors", async () => {
    queueCheck(result);
    const sync = await loadSyncStore();

    expect(sync.localMetadata()).toBeNull();
    expect(sync.providerStatus("github")).toBeNull();

    await sync.runCheck();

    expect(sync.localMetadata()).toEqual(result.local);
    expect(sync.providerStatus("github")).toEqual(result.github);
    expect(sync.providerStatus("webdav")).toEqual(result.webdav);
  });

  it("publishes local metadata while the remote check is still running", async () => {
    const local = deferred<typeof result.local>();
    const remote = deferred<typeof result>();
    invokeMock
      .mockReturnValueOnce(local.promise)
      .mockReturnValueOnce(remote.promise);
    const sync = await loadSyncStore();

    const checking = sync.runCheck();
    expect(sync.localMetadata()).toBeNull();

    local.resolve(result.local);
    await vi.waitFor(() => {
      expect(sync.localMetadata()).toEqual(result.local);
    });
    expect(sync.state().kind).toBe("checking");
    expect(sync.providerStatus("github")).toBeNull();

    remote.resolve(result);
    await checking;
    expect(sync.providerStatus("github")).toEqual(result.github);
  });

  it("refreshes local metadata without starting a remote check", async () => {
    const updated = { version: 6, config_digest: "local-6" };
    invokeMock.mockResolvedValueOnce(updated);
    const sync = await loadSyncStore();

    await sync.refreshLocalMetadata();

    expect(sync.localMetadata()).toEqual(updated);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);
    expect(commandCallCount("sync_check_remotes")).toBe(0);
  });

  it("runs a fresh local refresh after a config mutation overlaps an older snapshot", async () => {
    const stale = deferred<Metadata>();
    const updated = { version: 6, config_digest: "local-6" };
    invokeMock
      .mockReturnValueOnce(stale.promise)
      .mockResolvedValueOnce(updated);
    const sync = await loadSyncStore();

    const oldRefresh = sync.refreshLocalMetadata();
    const afterMutation = sync.refreshLocalAfterMutation();
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);

    stale.resolve(result.local);
    await Promise.all([oldRefresh, afterMutation]);

    expect(commandCallCount("sync_refresh_local_metadata")).toBe(2);
    expect(commandCallCount("sync_check_remotes")).toBe(0);
    expect(sync.localMetadata()).toEqual(updated);
  });

  it("recomputes local metadata on page open even while a remote check is in flight", async () => {
    const remote = deferred<typeof result>();
    const updated = { version: 6, config_digest: "local-6" };
    invokeMock
      .mockResolvedValueOnce(result.local)
      .mockReturnValueOnce(remote.promise)
      .mockResolvedValueOnce(updated);
    const sync = await loadSyncStore();

    const background = sync.runCheck({ silent: true });
    await vi.waitFor(() => {
      expect(commandCallCount("sync_check_remotes")).toBe(1);
    });

    const opened = sync.runCheck({ silent: true });
    await vi.waitFor(() => {
      expect(sync.localMetadata()).toEqual(updated);
    });
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(2);
    expect(commandCallCount("sync_check_remotes")).toBe(1);

    remote.resolve(result);
    await Promise.all([background, opened]);

    expect(sync.localMetadata()).toEqual(updated);
  });

  it("keeps a newer page refresh when an older remote check completes", async () => {
    const remote = deferred<CheckResult>();
    invokeMock
      .mockResolvedValueOnce(result.local)
      .mockReturnValueOnce(remote.promise);
    const sync = await loadSyncStore();

    const checking = sync.runCheck({ silent: true });
    await vi.waitFor(() => {
      expect(sync.localMetadata()).toEqual(result.local);
    });

    const latest = { version: 6, config_digest: "local-6" };
    invokeMock.mockResolvedValueOnce(latest);
    await sync.refreshLocalMetadata();
    expect(sync.localMetadata()).toEqual(latest);

    remote.resolve(result);
    await checking;
    expect(sync.localMetadata()).toEqual(latest);
    expect(sync.providerStatus("github")).toEqual(result.github);
  });

  it("reports updates only when a remote version is strictly newer", async () => {
    const newerRemote: CheckResult = {
      ...result,
      local: { version: 5, config_digest: "local-5" },
      github: {
        ...result.github,
        remote: { version: 6, config_digest: "github-6" },
      },
      webdav: {
        ...result.webdav,
        remote: { version: 4, config_digest: "webdav-4" },
      },
    };
    queueCheck(newerRemote);
    const sync = await loadSyncStore();

    await sync.runCheck();

    expect(sync.hasRemoteUpdate("github")).toBe(true);
    expect(sync.hasRemoteUpdate("webdav")).toBe(false);
    expect(sync.anyRemoteUpdate()).toBe(true);
    expect(sync.anyVersionDifference()).toBe(true);

    const sameVersion: CheckResult = {
      ...result,
      github: {
        ...result.github,
        remote: { version: 5, config_digest: "same-version" },
      },
      webdav: { ...result.webdav, remote: null },
    };
    queueCheck(sameVersion);

    await sync.runCheck();

    expect(sync.hasRemoteUpdate("github")).toBe(false);
    expect(sync.hasRemoteUpdate("webdav")).toBe(false);
    expect(sync.anyRemoteUpdate()).toBe(false);
    expect(sync.hasLocalUpdate("github")).toBe(false);
    expect(sync.anyVersionDifference()).toBe(false);
  });

  it("marks the local side and settings when the local version is newer", async () => {
    const newerLocal: CheckResult = {
      ...result,
      local: { version: 3, config_digest: "local-3" },
      github: {
        ...result.github,
        remote: { version: 1, config_digest: "github-1" },
      },
    };
    queueCheck(newerLocal);
    const sync = await loadSyncStore();

    await sync.runCheck();

    expect(sync.hasLocalUpdate("github")).toBe(true);
    expect(sync.hasRemoteUpdate("github")).toBe(false);
    expect(sync.anyVersionDifference()).toBe(true);
  });

  it("updates automatic pull after the local save without starting a remote check", async () => {
    queueCheck(result);
    const sync = await loadSyncStore();
    await sync.runCheck();
    expect(sync.autoPullEnabled("github")).toBe(false);
    const githubBefore = sync.providerStatus("github");
    const webdavBefore = sync.providerStatus("webdav");

    const pending = deferred<void>();
    invokeMock.mockReturnValueOnce(pending.promise);
    const saving = sync.saveAutoPull("github", true, "sync-password");

    expect(sync.autoPullEnabled("github")).toBe(false);
    expect(commandCallCount("sync_check_remotes")).toBe(1);
    expect(invokeMock).toHaveBeenLastCalledWith("set_sync_auto_pull", {
      provider: "github",
      enabled: true,
      password: "sync-password",
    });

    pending.resolve(undefined);
    await saving;
    expect(sync.autoPullEnabled("github")).toBe(true);
    expect(sync.providerStatus("github")).toEqual({
      ...githubBefore,
      auto_pull: true,
    });
    expect(sync.providerStatus("github")).not.toBe(githubBefore);
    expect(sync.providerStatus("webdav")).toBe(webdavBefore);
    expect(commandCallCount("sync_check_remotes")).toBe(1);
  });

  it("rolls back the automatic pull switch when the local save fails", async () => {
    queueCheck(result);
    const sync = await loadSyncStore();
    await sync.runCheck();

    invokeMock.mockRejectedValueOnce(new Error("local save failed"));
    const saving = sync.saveAutoPull("github", true, "sync-password");
    const failed = expect(saving).rejects.toThrow("local save failed");
    expect(sync.autoPullEnabled("github")).toBe(false);

    await failed;
    expect(sync.autoPullEnabled("github")).toBe(false);
    expect(commandCallCount("sync_check_remotes")).toBe(1);
  });

  it("keeps a saved automatic pull value through a stale check, then releases it once confirmed", async () => {
    queueCheck(result);
    const sync = await loadSyncStore();
    await sync.runCheck();

    const stale = deferred<typeof result>();
    invokeMock
      .mockResolvedValueOnce(result.local)
      .mockReturnValueOnce(stale.promise);
    const checking = sync.runCheck({ silent: true });
    await vi.waitFor(() => {
      expect(commandCallCount("sync_check_remotes")).toBe(2);
    });
    invokeMock.mockResolvedValueOnce(undefined);
    await sync.saveAutoPull("github", true, "sync-password");

    stale.resolve(result);
    await checking;
    expect(sync.autoPullEnabled("github")).toBe(true);

    const confirmed: CheckResult = {
      ...result,
      github: { ...result.github, auto_pull: true },
    };
    queueCheck(confirmed);
    await sync.runCheck({ silent: true });
    expect(sync.autoPullEnabled("github")).toBe(true);

    queueCheck(result);
    await sync.runCheck({ silent: true });
    expect(sync.autoPullEnabled("github")).toBe(false);
  });

  it("keeps a saved automatic pull value when an older silent check fails", async () => {
    queueCheck(result);
    vi.spyOn(console, "error").mockImplementation(() => {});
    const sync = await loadSyncStore();
    await sync.runCheck();

    const stale = deferred<typeof result>();
    invokeMock
      .mockResolvedValueOnce(result.local)
      .mockReturnValueOnce(stale.promise);
    const checking = sync.runCheck({ silent: true });
    await vi.waitFor(() => {
      expect(commandCallCount("sync_check_remotes")).toBe(2);
    });
    invokeMock.mockResolvedValueOnce(undefined);
    await sync.saveAutoPull("github", true, "sync-password");

    stale.reject(new Error("offline"));
    await checking;

    expect(sync.autoPullEnabled("github")).toBe(true);
  });

  it("coalesces re-entrant remote checks onto one request", async () => {
    const local = deferred<Metadata>();
    const remote = deferred<CheckResult>();
    invokeMock
      .mockReturnValueOnce(local.promise)
      .mockReturnValueOnce(remote.promise);
    const sync = await loadSyncStore();

    const first = sync.runCheck();
    const second = sync.runCheck({ silent: true });

    expect(invokeMock).toHaveBeenCalledTimes(1);

    local.resolve(result.local);
    await vi.waitFor(() => {
      expect(commandCallCount("sync_check_remotes")).toBe(1);
    });
    remote.resolve(result);
    await Promise.all([first, second]);

    expect(sync.localMetadata()).toEqual(result.local);
  });

  it("runs a fresh check after an in-flight request when refreshing after a mutation", async () => {
    const pending = deferred<CheckResult>();
    const updated: CheckResult = {
      ...result,
      local: { version: 6, config_digest: "local-6" },
    };
    invokeMock
      .mockResolvedValueOnce(result.local)
      .mockReturnValueOnce(pending.promise)
      .mockResolvedValueOnce(updated.local)
      .mockResolvedValueOnce(updated);
    const sync = await loadSyncStore();

    const oldCheck = sync.runCheck({ silent: true });
    await vi.waitFor(() => {
      expect(commandCallCount("sync_check_remotes")).toBe(1);
    });
    const refresh = sync.refreshAfterMutation();
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);

    pending.resolve(result);
    await oldCheck;
    await refresh;

    expect(commandCallCount("sync_refresh_local_metadata")).toBe(2);
    expect(commandCallCount("sync_check_remotes")).toBe(2);
    expect(sync.localMetadata()).toEqual(updated.local);
  });

  it("keeps the last ready snapshot visible while rechecking", async () => {
    const pending = deferred<CheckResult>();
    const updated: CheckResult = {
      ...result,
      local: { version: 6, config_digest: "local-6" },
    };
    queueCheck(result);
    const sync = await loadSyncStore();
    await sync.runCheck();

    invokeMock
      .mockResolvedValueOnce(updated.local)
      .mockReturnValueOnce(pending.promise);
    const recheck = sync.runCheck({ silent: true });
    await vi.waitFor(() => {
      expect(sync.localMetadata()).toEqual(updated.local);
    });

    expect(sync.state().kind).toBe("checking");
    expect(sync.providerStatus("github")).toEqual(result.github);
    expect(sync.hasRemoteUpdate("github")).toBe(false);
    expect(sync.anyRemoteUpdate()).toBe(false);

    pending.resolve(updated);
    await recheck;

    expect(sync.hasRemoteUpdate("github")).toBe(false);
  });

  it("preserves the previous snapshot when a silent check fails", async () => {
    const failure = new Error("offline");
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    queueCheck(result);
    const sync = await loadSyncStore();
    await sync.runCheck();

    invokeMock
      .mockResolvedValueOnce(result.local)
      .mockRejectedValueOnce(failure);
    await sync.runCheck({ silent: true });

    expect(sync.localMetadata()).toEqual(result.local);
    expect(sync.providerStatus("github")).toEqual(result.github);
    expect(sync.hasRemoteUpdate("github")).toBe(true);
    expect(errorSpy).toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it("surfaces a non-silent check failure in the public state", async () => {
    const failure = new Error("offline");
    vi.spyOn(console, "error").mockImplementation(() => {});
    invokeMock.mockRejectedValueOnce(failure);
    const sync = await loadSyncStore();

    const check = sync.runCheck();
    expect(sync.state().kind).toBe("checking");
    await check;

    expect(sync.state()).toEqual({ kind: "error", error: failure });
    expect(sync.localMetadata()).toBeNull();
  });

  it("checks after ten seconds and then once per hour", async () => {
    vi.useFakeTimers();
    mockSuccessfulChecks();
    const sync = await loadSyncStore();

    sync.startBackgroundChecks();
    await vi.advanceTimersByTimeAsync(9_999);
    expect(invokeMock).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);
    expect(commandCallCount("sync_check_remotes")).toBe(1);

    await vi.advanceTimersByTimeAsync(60 * 60 * 1000 - 1);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);

    await vi.advanceTimersByTimeAsync(1);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(2);
    expect(commandCallCount("sync_check_remotes")).toBe(2);
  });

  it("starts the background schedule only once", async () => {
    vi.useFakeTimers();
    mockSuccessfulChecks();
    const sync = await loadSyncStore();

    sync.startBackgroundChecks();
    await vi.advanceTimersByTimeAsync(1_000);
    sync.startBackgroundChecks();

    await vi.advanceTimersByTimeAsync(9_000);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);
    expect(commandCallCount("sync_check_remotes")).toBe(1);

    await vi.advanceTimersByTimeAsync(1_000);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);
  });

  it("stops pending timers and allows the schedule to start again", async () => {
    vi.useFakeTimers();
    mockSuccessfulChecks();
    const sync = await loadSyncStore();

    sync.startBackgroundChecks();
    sync.stopBackgroundChecks();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(invokeMock).not.toHaveBeenCalled();

    sync.startBackgroundChecks();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);
    expect(commandCallCount("sync_check_remotes")).toBe(1);

    sync.stopBackgroundChecks();
    await vi.advanceTimersByTimeAsync(2 * 60 * 60 * 1000);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);

    sync.startBackgroundChecks();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(2);
    expect(commandCallCount("sync_check_remotes")).toBe(2);
  });
});
