import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import * as cli from "./cli.svelte.ts";

describe("CLI status", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    invokeMock.mockReset();
    cli.stopBackgroundChecks();
  });

  it("marks missing and outdated installations for attention", async () => {
    invokeMock.mockResolvedValueOnce({
      installed: false,
      path: "",
      bundled: true,
      installed_version: null,
      expected_version: "1.0.0",
      needs_update: true,
    });
    await cli.runCheck();
    expect(cli.needsAttention()).toBe(true);

    invokeMock.mockResolvedValueOnce({
      installed: true,
      path: "/usr/local/bin/rssh",
      bundled: true,
      installed_version: "0.9.0",
      expected_version: "1.0.0",
      needs_update: true,
    });
    await cli.runCheck();
    expect(cli.needsAttention()).toBe(true);
  });

  it("clears attention after the current CLI is installed", async () => {
    invokeMock.mockResolvedValue({
      installed: true,
      path: "/usr/local/bin/rssh",
      bundled: true,
      installed_version: "1.0.0",
      expected_version: "1.0.0",
      needs_update: false,
    });

    await cli.runCheck();

    expect(cli.needsAttention()).toBe(false);
    expect(cli.status()?.installed_version).toBe("1.0.0");
  });

  it("checks after the startup delay and does not schedule twice", async () => {
    invokeMock.mockResolvedValue({
      installed: false,
      path: "",
      bundled: true,
      installed_version: null,
      expected_version: "1.0.0",
      needs_update: true,
    });

    cli.startBackgroundChecks();
    cli.startBackgroundChecks();
    expect(invokeMock).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(10_000);
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });
});
