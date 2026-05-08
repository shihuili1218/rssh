import { describe, it, expect, vi, beforeEach } from "vitest";

// vi.mock 提升到 import 之前执行——测试文件不能让真实的 store 模块被 evaluate
// （它在顶层访问 navigator.userAgent，node 环境没有）。
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));
vi.mock("../stores/app.svelte.ts", () => ({
  addTab: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import * as app from "../stores/app.svelte.ts";
import { registerRsshOscHandlers } from "./handler.ts";

interface FakeParser {
  registerOscHandler: ReturnType<typeof vi.fn>;
}

/** 收集 dispatcher。register 之后用 `dispatch.fn(data)` 模拟 xterm 解到 OSC 7337。 */
function setup() {
  let captured: ((data: string) => boolean) | null = null;
  const parser: FakeParser = {
    registerOscHandler: vi.fn((id: number, fn: (data: string) => boolean) => {
      expect(id).toBe(7337);
      captured = fn;
    }),
  };
  const reporter = { error: vi.fn() };
  registerRsshOscHandlers(parser, reporter);
  if (!captured) throw new Error("OSC 7337 handler not registered");
  return { parser, reporter, dispatch: captured };
}

/** 让 fire-and-forget 的 async handler 跑完一轮 microtask。 */
async function flush() {
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("registerRsshOscHandlers — dispatch shape", () => {
  it("registers on OSC 7337", () => {
    const { parser } = setup();
    expect(parser.registerOscHandler).toHaveBeenCalledTimes(1);
    expect(parser.registerOscHandler.mock.calls[0][0]).toBe(7337);
  });

  it("returns false when payload has no colon", () => {
    const { dispatch } = setup();
    expect(dispatch("nokind")).toBe(false);
  });

  it("returns false for unknown kind", () => {
    const { dispatch } = setup();
    expect(dispatch("weird:something")).toBe(false);
  });

  it("returns true for known kind even before async work resolves", () => {
    const { dispatch } = setup();
    (invoke as any).mockResolvedValue([]);
    expect(dispatch("open:anything")).toBe(true);
  });
});

describe("open: handler", () => {
  it("opens a tab when profile exists (case-insensitive match)", async () => {
    const { dispatch } = setup();
    (invoke as any).mockImplementation(async (cmd: string) => {
      if (cmd === "list_profiles")
        return [
          {
            id: "p1",
            name: "MyHost",
            host: "1.2.3.4",
            port: 22,
            credential_id: "c1",
          },
        ];
      if (cmd === "get_credential")
        return { username: "alice", type: "key", secret: "PEM..." };
      throw new Error(`unexpected invoke ${cmd}`);
    });

    expect(dispatch("open:myhost")).toBe(true);
    await flush();

    expect(app.addTab).toHaveBeenCalledTimes(1);
    const arg = (app.addTab as any).mock.calls[0][0];
    expect(arg.type).toBe("ssh");
    expect(arg.label).toBe("MyHost");
    expect(arg.meta.profileId).toBe("p1");
    expect(arg.meta.host).toBe("1.2.3.4");
    expect(arg.meta.port).toBe("22");
    expect(arg.meta.username).toBe("alice");
    expect(arg.meta.authType).toBe("key");
  });

  it("reports error when profile not found", async () => {
    const { dispatch, reporter } = setup();
    (invoke as any).mockResolvedValue([]); // list_profiles → []

    expect(dispatch("open:nope")).toBe(true);
    await flush();

    expect(app.addTab).not.toHaveBeenCalled();
    expect(reporter.error).toHaveBeenCalledWith(
      "Profile 'nope' not found",
    );
  });

  it("opens tab even when get_credential throws (silently)", async () => {
    const { dispatch } = setup();
    (invoke as any).mockImplementation(async (cmd: string) => {
      if (cmd === "list_profiles")
        return [
          {
            id: "p1",
            name: "h",
            host: "h",
            port: 22,
            credential_id: "c1",
          },
        ];
      if (cmd === "get_credential") throw new Error("denied");
      throw new Error(`unexpected ${cmd}`);
    });

    expect(dispatch("open:h")).toBe(true);
    await flush();

    expect(app.addTab).toHaveBeenCalledTimes(1);
    const meta = (app.addTab as any).mock.calls[0][0].meta;
    // credential 拿不到 → username/secret 取默认空值，authType 用 password 默认
    expect(meta.username).toBe("");
    expect(meta.authType).toBe("password");
  });

  it("skips get_credential when profile has no credential_id", async () => {
    const { dispatch } = setup();
    const calls: string[] = [];
    (invoke as any).mockImplementation(async (cmd: string) => {
      calls.push(cmd);
      if (cmd === "list_profiles")
        return [{ id: "p1", name: "h", host: "h", port: 22, credential_id: null }];
      throw new Error(`unexpected ${cmd}`);
    });

    expect(dispatch("open:h")).toBe(true);
    await flush();
    expect(calls).toEqual(["list_profiles"]);
    expect(app.addTab).toHaveBeenCalledTimes(1);
  });
});

describe("fwd: handler", () => {
  it("opens a forward tab when forward exists", async () => {
    const { dispatch } = setup();
    (invoke as any).mockImplementation(async (cmd: string) => {
      if (cmd === "list_forwards")
        return [
          {
            id: "f1",
            name: "tunnel",
            type: "local",
            local_port: 8080,
            remote_host: "10.0.0.1",
            remote_port: 80,
            profile_id: "p1",
          },
        ];
      if (cmd === "get_profile") return { id: "p1", name: "prod" };
      throw new Error(`unexpected ${cmd}`);
    });

    expect(dispatch("fwd:tunnel")).toBe(true);
    await flush();

    expect(app.addTab).toHaveBeenCalledTimes(1);
    const arg = (app.addTab as any).mock.calls[0][0];
    expect(arg.type).toBe("forward");
    expect(arg.label).toBe("tunnel");
    expect(arg.meta.forwardId).toBe("f1");
    expect(arg.meta.forwardType).toBe("local");
    expect(arg.meta.localPort).toBe("8080");
    expect(arg.meta.remoteHost).toBe("10.0.0.1");
    expect(arg.meta.remotePort).toBe("80");
    expect(arg.meta.profileName).toBe("prod");
    // tab id 形态：fwd:<id>:<timestamp>
    expect(arg.id).toMatch(/^fwd:f1:\d+$/);
  });

  it("reports error when forward not found", async () => {
    const { dispatch, reporter } = setup();
    (invoke as any).mockResolvedValue([]);

    expect(dispatch("fwd:gone")).toBe(true);
    await flush();

    expect(app.addTab).not.toHaveBeenCalled();
    expect(reporter.error).toHaveBeenCalledWith("Forward 'gone' not found");
  });

  it("falls back to '?' profile name when get_profile fails", async () => {
    const { dispatch } = setup();
    (invoke as any).mockImplementation(async (cmd: string) => {
      if (cmd === "list_forwards")
        return [
          {
            id: "f1",
            name: "t",
            type: "local",
            local_port: 1,
            remote_host: "h",
            remote_port: 1,
            profile_id: "p-missing",
          },
        ];
      if (cmd === "get_profile") throw new Error("nope");
      throw new Error(`unexpected ${cmd}`);
    });

    expect(dispatch("fwd:t")).toBe(true);
    await flush();
    const meta = (app.addTab as any).mock.calls[0][0].meta;
    expect(meta.profileName).toBe("?");
  });
});
