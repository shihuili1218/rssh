import { describe, expect, it } from "vitest";
import { setupXtermIme229Workaround } from "./xterm-ime-229-workaround.ts";

type Listener = (event: Event) => void;

class FakeHost {
  readonly listeners = new Map<string, Set<Listener>>();

  addEventListener(type: string, listener: EventListenerOrEventListenerObject): void {
    if (typeof listener !== "function") throw new Error("test host only supports function listeners");
    const listeners = this.listeners.get(type) ?? new Set<Listener>();
    listeners.add(listener as Listener);
    this.listeners.set(type, listeners);
  }

  removeEventListener(type: string, listener: EventListenerOrEventListenerObject): void {
    if (typeof listener !== "function") return;
    this.listeners.get(type)?.delete(listener as Listener);
  }

  fire(type: string, event: FakeEvent): void {
    for (const listener of this.listeners.get(type) ?? []) {
      listener(event as unknown as Event);
    }
  }
}

type FakeEvent = {
  target: unknown;
  keyCode?: number;
  data?: string | null;
  inputType?: string;
  isComposing?: boolean;
  cancelable?: boolean;
  defaultPrevented: boolean;
  propagationStopped: boolean;
  immediatePropagationStopped: boolean;
  preventDefault: () => void;
  stopPropagation: () => void;
  stopImmediatePropagation: () => void;
};

function fakeEvent(target: unknown, overrides: Partial<FakeEvent> = {}): FakeEvent {
  const event: FakeEvent = {
    target,
    cancelable: true,
    defaultPrevented: false,
    propagationStopped: false,
    immediatePropagationStopped: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    stopPropagation() {
      this.propagationStopped = true;
    },
    stopImmediatePropagation() {
      this.immediatePropagationStopped = true;
    },
    ...overrides,
  };
  return event;
}

function setup(enabled = true) {
  const host = new FakeHost();
  const calls: Array<[string, boolean | undefined]> = [];
  const textarea = { value: "stale" } as HTMLTextAreaElement;
  const terminal = {
    textarea,
    options: { screenReaderMode: false },
    input(data: string, wasUserInput?: boolean) {
      calls.push([data, wasUserInput]);
    },
  };
  const cleanup = setupXtermIme229Workaround({ terminal, host, enabled });
  return { host, calls, textarea, cleanup };
}

describe("setupXtermIme229Workaround", () => {
  it("replays insertText after a non-composing 229 keydown and blocks xterm's fallback input listener", () => {
    const { host, calls, textarea } = setup();

    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    const input = fakeEvent(textarea, { data: "o", inputType: "insertText", isComposing: false });
    host.fire("input", input);

    expect(calls).toEqual([["o", true]]);
    expect(textarea.value).toBe("");
    expect(input.propagationStopped).toBe(true);
    expect(input.immediatePropagationStopped).toBe(true);
    expect(input.defaultPrevented).toBe(true);
  });

  it("does not intercept normal input before a 229 keydown", () => {
    const { host, calls, textarea } = setup();
    const input = fakeEvent(textarea, { data: "m", inputType: "insertText", isComposing: false });

    host.fire("input", input);

    expect(calls).toEqual([]);
    expect(input.propagationStopped).toBe(false);
  });

  it("clears the pending 229 state on keyup", () => {
    const { host, calls, textarea } = setup();

    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    host.fire("keyup", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    host.fire("input", fakeEvent(textarea, { data: "x", inputType: "insertText", isComposing: false }));

    expect(calls).toEqual([]);
  });

  it("does not intercept real composition input", () => {
    const { host, calls, textarea } = setup();

    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    host.fire("compositionstart", fakeEvent(textarea));
    host.fire("input", fakeEvent(textarea, { data: "拼", inputType: "insertCompositionText", isComposing: true }));

    expect(calls).toEqual([]);
  });

  it("ignores events that do not come from xterm's helper textarea", () => {
    const { host, calls, textarea } = setup();
    const otherTarget = {};

    host.fire("keydown", fakeEvent(otherTarget, { keyCode: 229, isComposing: false }));
    host.fire("input", fakeEvent(textarea, { data: "o", inputType: "insertText", isComposing: false }));

    expect(calls).toEqual([]);
  });

  it("does nothing when disabled", () => {
    const { host, calls, textarea } = setup(false);

    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    host.fire("input", fakeEvent(textarea, { data: "o", inputType: "insertText", isComposing: false }));

    expect(calls).toEqual([]);
  });

  it("removes listeners on cleanup", () => {
    const { host, calls, textarea, cleanup } = setup();

    cleanup();
    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    host.fire("input", fakeEvent(textarea, { data: "o", inputType: "insertText", isComposing: false }));

    expect(calls).toEqual([]);
  });
});
