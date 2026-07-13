import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

type Metadata = { version: number; config_digest: string };
type ProviderObservation = {
  remote: Metadata | null;
  error: string | null;
  pulled: boolean;
};
type CheckResult = {
  local: Metadata;
  github: ProviderObservation;
  webdav: ProviderObservation;
};

const localV5: Metadata = { version: 5, config_digest: "local-5" };
const githubV6: Metadata = { version: 6, config_digest: "github-6" };
const result: CheckResult = {
  local: localV5,
  github: { remote: githubV6, error: null, pulled: false },
  webdav: { remote: null, error: null, pulled: false },
};

function commandCallCount(command: string): number {
  return invokeMock.mock.calls.filter(([name]) => name === command).length;
}

function mockSuccessfulChecks(
  local: Metadata = localV5,
  checked: CheckResult = result,
): void {
  invokeMock.mockImplementation((command: string) => {
    if (command === "sync_refresh_local_metadata") return Promise.resolve(local);
    if (command === "sync_check_remotes") return Promise.resolve(checked);
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

  it("loads automatic pull from local storage while the remote check is pending", async () => {
    const remote = deferred<CheckResult>();
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") return Promise.resolve(localV5);
      if (command === "sync_check_remotes") return remote.promise;
      if (command === "get_sync_auto_pull_status") {
        return Promise.resolve({ github: true, webdav: false });
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();

    const checking = sync.runCheck({ silent: true });
    await vi.waitFor(() => {
      expect(commandCallCount("sync_check_remotes")).toBe(1);
    });

    await sync.loadAutoPullStatus();

    expect(sync.autoPullEnabled("github")).toBe(true);
    expect(sync.autoPullEnabled("webdav")).toBe(false);

    remote.resolve(result);
    await checking;
  });

  it("does not present automatic pull as loaded before the local read completes", async () => {
    const status = deferred<{ github: boolean; webdav: boolean }>();
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_sync_auto_pull_status") return status.promise;
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();

    const loading = sync.loadAutoPullStatus();

    expect(sync.autoPullStatusLoaded()).toBe(false);

    status.resolve({ github: true, webdav: false });
    await loading;

    expect(sync.autoPullStatusLoaded()).toBe(true);
    expect(sync.autoPullEnabled("github")).toBe(true);
  });

  it("serializes a save behind a repeated automatic-pull load", async () => {
    const reload = deferred<{ github: boolean; webdav: boolean }>();
    let statusCalls = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_sync_auto_pull_status") {
        statusCalls += 1;
        return statusCalls === 1
          ? Promise.resolve({ github: false, webdav: false })
          : reload.promise;
      }
      if (command === "set_sync_auto_pull") return Promise.resolve();
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();
    await sync.loadAutoPullStatus();

    const loading = sync.loadAutoPullStatus();
    const duplicateLoad = sync.loadAutoPullStatus();
    const saving = sync.saveAutoPull("github", true, "sync-password");

    expect(sync.autoPullStatusLoaded()).toBe(false);
    expect(commandCallCount("get_sync_auto_pull_status")).toBe(2);
    expect(commandCallCount("set_sync_auto_pull")).toBe(0);

    reload.resolve({ github: false, webdav: false });
    await Promise.all([loading, duplicateLoad, saving]);

    expect(sync.autoPullStatusLoaded()).toBe(true);
    expect(sync.autoPullEnabled("github")).toBe(true);
    expect(commandCallCount("set_sync_auto_pull")).toBe(1);
  });

  it("changes automatic pull only after the local save succeeds", async () => {
    const saved = deferred<void>();
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_sync_auto_pull_status") {
        return Promise.resolve({ github: false, webdav: true });
      }
      if (command === "set_sync_auto_pull") return saved.promise;
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();
    await sync.loadAutoPullStatus();

    const saving = sync.saveAutoPull("github", true, "sync-password");
    expect(sync.autoPullEnabled("github")).toBe(false);

    saved.resolve(undefined);
    await saving;

    expect(sync.autoPullEnabled("github")).toBe(true);
    expect(sync.autoPullEnabled("webdav")).toBe(true);
    expect(invokeMock).toHaveBeenLastCalledWith("set_sync_auto_pull", {
      provider: "github",
      enabled: true,
      password: "sync-password",
    });
    expect(commandCallCount("sync_check_remotes")).toBe(0);
  });

  it("keeps automatic pull unchanged when the local save fails", async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === "get_sync_auto_pull_status") {
        return Promise.resolve({ github: false, webdav: false });
      }
      if (command === "set_sync_auto_pull") {
        return Promise.reject(new Error("local save failed"));
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();
    await sync.loadAutoPullStatus();

    await expect(
      sync.saveAutoPull("github", true, "sync-password"),
    ).rejects.toThrow("local save failed");

    expect(sync.autoPullEnabled("github")).toBe(false);
  });

  it("publishes local metadata before a remote check completes", async () => {
    const remote = deferred<CheckResult>();
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") return Promise.resolve(localV5);
      if (command === "sync_check_remotes") return remote.promise;
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();

    const checking = sync.runCheck();
    await vi.waitFor(() => {
      expect(sync.localMetadata()).toEqual(localV5);
    });

    expect(sync.providerStatus("github")).toBeNull();

    remote.resolve(result);
    await checking;
    expect(sync.providerStatus("github")).toEqual(result.github);
  });

  it("queues one fresh local snapshot when refresh is requested in flight", async () => {
    const first = deferred<Metadata>();
    const second = deferred<Metadata>();
    let calls = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") {
        calls += 1;
        return calls === 1 ? first.promise : second.promise;
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();

    const oldRefresh = sync.refreshLocalMetadata();
    const newRefresh = sync.refreshLocalMetadata();

    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);

    first.resolve(localV5);
    await vi.waitFor(() => {
      expect(commandCallCount("sync_refresh_local_metadata")).toBe(2);
    });
    const localV6 = { version: 6, config_digest: "local-6" };
    second.resolve(localV6);
    await Promise.all([oldRefresh, newRefresh]);

    expect(sync.localMetadata()).toEqual(localV6);
  });

  it("keeps the last remote version on provider error and clears it on a confirmed miss", async () => {
    const checks: CheckResult[] = [
      result,
      {
        ...result,
        github: { remote: null, error: "offline", pulled: false },
      },
      {
        ...result,
        github: { remote: null, error: null, pulled: false },
      },
    ];
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") return Promise.resolve(localV5);
      if (command === "sync_check_remotes") return Promise.resolve(checks.shift()!);
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();

    await sync.runCheck();
    await sync.runCheck();

    expect(sync.providerStatus("github")?.remote).toEqual(githubV6);
    expect(sync.providerStatus("github")?.error).toBe("offline");

    await sync.runCheck();
    expect(sync.providerStatus("github")?.remote).toBeNull();
    expect(sync.providerStatus("github")?.error).toBeNull();
  });

  it("uses the backend snapshot after an automatic pull", async () => {
    const localV6 = { version: 6, config_digest: "local-6" };
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") return Promise.resolve(localV5);
      if (command === "sync_check_remotes") {
        return Promise.resolve({
          ...result,
          local: localV6,
          github: { ...result.github, pulled: true },
        });
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();

    await sync.runCheck();

    expect(sync.localMetadata()).toEqual(localV6);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);
  });

  it("publishes the backend local snapshot after a failed automatic pull", async () => {
    const localV6 = { version: 6, config_digest: "local-6" };
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") return Promise.resolve(localV5);
      if (command === "sync_check_remotes") {
        return Promise.resolve({
          ...result,
          local: localV6,
          github: { ...result.github, error: "partial import", pulled: false },
        });
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();

    await sync.runCheck();

    expect(sync.localMetadata()).toEqual(localV6);
  });

  it("does not replace a newer local refresh with an older remote-check snapshot", async () => {
    const remote = deferred<CheckResult>();
    const localV6 = { version: 6, config_digest: "local-6" };
    let localCalls = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") {
        localCalls += 1;
        return Promise.resolve(localCalls === 1 ? localV5 : localV6);
      }
      if (command === "sync_check_remotes") return remote.promise;
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();

    const checking = sync.runCheck();
    await vi.waitFor(() => {
      expect(commandCallCount("sync_check_remotes")).toBe(1);
    });
    await sync.refreshLocalMetadata();
    remote.resolve(result);
    await checking;

    expect(sync.localMetadata()).toEqual(localV6);
  });

  it("publishes a pulled provider with its backend local snapshot", async () => {
    const oldRemote = { version: 4, config_digest: "github-4" };
    const localV6 = { version: 6, config_digest: "local-6" };
    const oldResult: CheckResult = {
      ...result,
      github: { remote: oldRemote, error: null, pulled: false },
    };
    const pulledResult: CheckResult = {
      ...result,
      local: localV6,
      github: { ...result.github, pulled: true },
    };
    let localCalls = 0;
    let remoteCalls = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") {
        localCalls += 1;
        return Promise.resolve(localCalls === 1 ? localV5 : localV6);
      }
      if (command === "sync_check_remotes") {
        remoteCalls += 1;
        return Promise.resolve(remoteCalls === 1 ? oldResult : pulledResult);
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();
    await sync.runCheck();

    await sync.runCheck({ silent: true });

    expect(sync.localMetadata()).toEqual(localV6);
    expect(sync.providerStatus("github")).toEqual(pulledResult.github);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(2);
  });

  it("runs one fresh pass after requests arrive during an active pass", async () => {
    const firstRemote = deferred<CheckResult>();
    const localV6 = { version: 6, config_digest: "local-6" };
    let localCalls = 0;
    let remoteCalls = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") {
        localCalls += 1;
        return Promise.resolve(localCalls === 1 ? localV5 : localV6);
      }
      if (command === "sync_check_remotes") {
        remoteCalls += 1;
        return remoteCalls === 1
          ? firstRemote.promise
          : Promise.resolve({ ...result, local: localV6 });
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();

    const first = sync.runCheck({ silent: true });
    await vi.waitFor(() => {
      expect(commandCallCount("sync_check_remotes")).toBe(1);
    });
    const second = sync.runCheck({ silent: true });
    const third = sync.runCheck({ silent: true });

    firstRemote.resolve(result);
    await Promise.all([first, second, third]);

    expect(commandCallCount("sync_refresh_local_metadata")).toBe(2);
    expect(commandCallCount("sync_check_remotes")).toBe(2);
    expect(sync.localMetadata()).toEqual(localV6);
  });

  it("drains a pending check after the active pass fails", async () => {
    const firstRemote = deferred<CheckResult>();
    vi.spyOn(console, "error").mockImplementation(() => {});
    let remoteCalls = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") {
        return Promise.resolve(localV5);
      }
      if (command === "sync_check_remotes") {
        remoteCalls += 1;
        return remoteCalls === 1
          ? firstRemote.promise
          : Promise.resolve(result);
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();

    const first = sync.runCheck();
    await vi.waitFor(() => {
      expect(commandCallCount("sync_check_remotes")).toBe(1);
    });
    const second = sync.runCheck({ silent: true });

    firstRemote.reject(new Error("offline"));
    await Promise.all([first, second]);

    expect(commandCallCount("sync_check_remotes")).toBe(2);
    expect(sync.providerStatus("github")).toEqual(result.github);
  });

  it("derives push, pull, and settings dots from version differences", async () => {
    const checked: CheckResult = {
      local: localV5,
      github: {
        remote: { version: 6, config_digest: "github-6" },
        error: null,
        pulled: false,
      },
      webdav: {
        remote: { version: 4, config_digest: "webdav-4" },
        error: null,
        pulled: false,
      },
    };
    mockSuccessfulChecks(localV5, checked);
    const sync = await loadSyncStore();

    await sync.runCheck();

    expect(sync.hasRemoteUpdate("github")).toBe(true);
    expect(sync.hasLocalUpdate("github")).toBe(false);
    expect(sync.hasRemoteUpdate("webdav")).toBe(false);
    expect(sync.hasLocalUpdate("webdav")).toBe(true);
    expect(sync.anyRemoteUpdate()).toBe(true);
    expect(sync.anyVersionDifference()).toBe(true);
  });

  it("preserves published data when a silent pass fails", async () => {
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    let fail = false;
    invokeMock.mockImplementation((command: string) => {
      if (command === "sync_refresh_local_metadata") return Promise.resolve(localV5);
      if (command === "sync_check_remotes") {
        return fail ? Promise.reject(new Error("offline")) : Promise.resolve(result);
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    const sync = await loadSyncStore();
    await sync.runCheck();

    fail = true;
    await sync.runCheck({ silent: true });

    expect(sync.localMetadata()).toEqual(localV5);
    expect(sync.providerStatus("github")).toEqual(result.github);
    expect(errorSpy).toHaveBeenCalled();
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

    await vi.advanceTimersByTimeAsync(60 * 60 * 1000);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(2);
    expect(commandCallCount("sync_check_remotes")).toBe(2);
  });

  it("starts the schedule once and can stop and restart it", async () => {
    vi.useFakeTimers();
    mockSuccessfulChecks();
    const sync = await loadSyncStore();

    sync.startBackgroundChecks();
    sync.startBackgroundChecks();
    sync.stopBackgroundChecks();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(invokeMock).not.toHaveBeenCalled();

    sync.startBackgroundChecks();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(commandCallCount("sync_refresh_local_metadata")).toBe(1);
    expect(commandCallCount("sync_check_remotes")).toBe(1);

    sync.stopBackgroundChecks();
    await vi.advanceTimersByTimeAsync(60 * 60 * 1000);
    expect(commandCallCount("sync_check_remotes")).toBe(1);
  });
});
