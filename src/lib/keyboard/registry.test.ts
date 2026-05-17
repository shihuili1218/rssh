import { beforeEach, describe, expect, it, vi } from "vitest";

const { settingsActive } = vi.hoisted(() => ({
  settingsActive: vi.fn(() => false),
}));

vi.mock("../stores/app.svelte.ts", () => ({
  settingsActive,
}));

import { attachKeyup, attachShortcuts, type Shortcut } from "./registry.ts";

type EventListener = (e: KeyboardEvent) => void;

let addEventListener: ReturnType<typeof vi.fn>;
let removeEventListener: ReturnType<typeof vi.fn>;
let listeners: Record<string, EventListener | undefined>;

function fakeEvent(): KeyboardEvent {
  return {
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  } as unknown as KeyboardEvent;
}

function keydownListener(): EventListener {
  const listener = listeners.keydown;
  if (!listener) throw new Error("keydown listener was not registered");
  return listener;
}

function keyupListener(): EventListener {
  const listener = listeners.keyup;
  if (!listener) throw new Error("keyup listener was not registered");
  return listener;
}

beforeEach(() => {
  settingsActive.mockReset();
  settingsActive.mockReturnValue(false);
  listeners = {};
  addEventListener = vi.fn((type: string, handler: EventListener) => {
    listeners[type] = handler;
  });
  removeEventListener = vi.fn((type: string, handler: EventListener) => {
    if (listeners[type] === handler) delete listeners[type];
  });
  vi.stubGlobal("window", {
    addEventListener,
    removeEventListener,
  });
});

describe("attachShortcuts", () => {
  it("registers a capture listener and detaches it", () => {
    const detach = attachShortcuts([]);

    expect(addEventListener).toHaveBeenCalledWith("keydown", expect.any(Function), { capture: true });
    expect(typeof listeners.keydown).toBe("function");

    detach();

    expect(removeEventListener).toHaveBeenCalledWith(
      "keydown",
      expect.any(Function),
      { capture: true },
    );
    expect(listeners.keydown).toBeUndefined();
  });

  it("runs only the first matching shortcut and prevents default by default", () => {
    const first = vi.fn();
    const second = vi.fn();
    const shortcuts: Shortcut[] = [
      { display: "first", match: () => true, handler: first },
      { display: "second", match: () => true, handler: second },
    ];

    attachShortcuts(shortcuts);
    const event = fakeEvent();
    keydownListener()(event);

    expect(first).toHaveBeenCalledWith(event);
    expect(second).not.toHaveBeenCalled();
    expect(event.preventDefault).toHaveBeenCalledTimes(1);
    expect(event.stopPropagation).toHaveBeenCalledTimes(1);
  });

  it("skips shortcuts marked skipInSettings while settings are active", () => {
    settingsActive.mockReturnValue(true);
    const skipped = vi.fn();
    const allowed = vi.fn();
    const shortcuts: Shortcut[] = [
      { display: "skip", skipInSettings: true, match: () => true, handler: skipped },
      { display: "allowed", match: () => true, handler: allowed },
    ];

    attachShortcuts(shortcuts);
    const event = fakeEvent();
    keydownListener()(event);

    expect(skipped).not.toHaveBeenCalled();
    expect(allowed).toHaveBeenCalledWith(event);
    expect(settingsActive).toHaveBeenCalledTimes(1);
  });

  it("leaves the event untouched when a handler explicitly returns false", () => {
    // Shortcut.handler 返回类型是 `void | false`，必须把 false 字面量
    // 标成 `false`（不是 `boolean`）才能匹配——否则 TS 推断 `() => boolean`
    // 不兼容 `() => false | void`（boolean 包含 true）。
    const handler = vi.fn((): false => false);

    attachShortcuts([{ display: "pass-through", match: () => true, handler }]);
    const event = fakeEvent();
    keydownListener()(event);

    expect(handler).toHaveBeenCalledWith(event);
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(event.stopPropagation).not.toHaveBeenCalled();
  });
});

describe("attachKeyup", () => {
  it("registers and detaches a capture keyup listener", () => {
    const handler = vi.fn();
    const detach = attachKeyup(handler);

    expect(addEventListener).toHaveBeenCalledWith("keyup", handler, { capture: true });
    keyupListener()(fakeEvent());
    expect(handler).toHaveBeenCalledTimes(1);

    detach();

    expect(removeEventListener).toHaveBeenCalledWith("keyup", handler, { capture: true });
    expect(listeners.keyup).toBeUndefined();
  });
});
