import { describe, expect, it } from "vitest";
import { setupXtermIme229Workaround } from "./xterm-ime-229-workaround.ts";

type Listener = (event: Event) => void;

class FakeHost {
  readonly listeners = new Map<string, Set<Listener>>();
  readonly targetListeners = new Map<string, Set<Listener>>();

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

  addTargetEventListener(type: string, listener: Listener): void {
    const listeners = this.targetListeners.get(type) ?? new Set<Listener>();
    listeners.add(listener);
    this.targetListeners.set(type, listeners);
  }

  fire(type: string, event: FakeEvent): void {
    for (const listener of this.listeners.get(type) ?? []) {
      listener(event as unknown as Event);
      if (event.immediatePropagationStopped) break;
    }
    if (event.propagationStopped) return;
    for (const listener of this.targetListeners.get(type) ?? []) {
      listener(event as unknown as Event);
      if (event.immediatePropagationStopped) break;
    }
  }
}

type FakeEvent = {
  target: unknown;
  keyCode?: number;
  data?: string | null;
  inputType?: string;
  isComposing?: boolean;
  composed?: boolean;
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

function textInputEvent(target: unknown, data: string, overrides: Partial<FakeEvent> = {}): FakeEvent {
  return fakeEvent(target, {
    data,
    inputType: "insertText",
    isComposing: false,
    composed: true,
    ...overrides,
  });
}

function setup(enabled = true, screenReaderMode = false) {
  const host = new FakeHost();
  const calls: Array<[string, boolean | undefined]> = [];
  const textarea = { value: "stale" } as HTMLTextAreaElement;
  const options = { screenReaderMode };
  const terminal = {
    textarea,
    options,
    input(data: string, wasUserInput?: boolean) {
      calls.push([data, wasUserInput]);
    },
  };
  const cleanup = setupXtermIme229Workaround({ terminal, host, enabled });
  return { host, calls, textarea, options, cleanup };
}

describe("setupXtermIme229Workaround", () => {
  it("replays the second insertText in the affected input-keydown-input sequence", () => {
    const { host, calls, textarea } = setup();
    let fallbackCalls = 0;
    host.addTargetEventListener("input", () => fallbackCalls++);

    const firstInput = textInputEvent(textarea, "m");
    host.fire("input", firstInput);
    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    const input = textInputEvent(textarea, "o");
    host.fire("input", input);

    expect(calls).toEqual([["o", true]]);
    expect(fallbackCalls).toBe(1);
    expect(firstInput.propagationStopped).toBe(false);
    expect(textarea.value).toBe("");
    expect(input.propagationStopped).toBe(true);
    expect(input.immediatePropagationStopped).toBe(true);
    expect(input.defaultPrevented).toBe(true);
  });

  it("does not intercept normal input before a 229 keydown", () => {
    const { host, calls, textarea } = setup();
    const input = textInputEvent(textarea, "m");

    host.fire("input", input);

    expect(calls).toEqual([]);
    expect(input.propagationStopped).toBe(false);
  });

  it("clears the pending 229 state on keyup", () => {
    const { host, calls, textarea } = setup();

    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    host.fire("keyup", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    host.fire("input", textInputEvent(textarea, "x"));

    expect(calls).toEqual([]);
  });

  it("does not intercept real composition input", () => {
    const { host, calls, textarea } = setup();

    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    host.fire("compositionstart", fakeEvent(textarea));
    host.fire("input", fakeEvent(textarea, {
      data: "拼",
      inputType: "insertCompositionText",
      isComposing: true,
      composed: true,
    }));

    expect(calls).toEqual([]);
  });

  it("ignores events that do not come from xterm's helper textarea", () => {
    const { host, calls, textarea } = setup();
    const otherTarget = {};

    host.fire("keydown", fakeEvent(otherTarget, { keyCode: 229, isComposing: false }));
    host.fire("input", textInputEvent(textarea, "o"));

    expect(calls).toEqual([]);
  });

  it("does nothing when disabled", () => {
    const { host, calls, textarea } = setup(false);

    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    host.fire("input", textInputEvent(textarea, "o"));

    expect(calls).toEqual([]);
  });

  it("consumes pending state when screen reader mode bypasses the workaround", () => {
    const { host, calls, textarea, options } = setup(true, true);

    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    const bypassedInput = textInputEvent(textarea, "o");
    host.fire("input", bypassedInput);

    options.screenReaderMode = false;
    const laterInput = textInputEvent(textarea, "x");
    host.fire("input", laterInput);

    expect(calls).toEqual([]);
    expect(bypassedInput.propagationStopped).toBe(false);
    expect(laterInput.propagationStopped).toBe(false);
  });

  it("leaves non-composed insertText to xterm and consumes pending state", () => {
    const { host, calls, textarea } = setup();

    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    const nonComposedInput = textInputEvent(textarea, "o", { composed: false });
    host.fire("input", nonComposedInput);
    const laterInput = textInputEvent(textarea, "x");
    host.fire("input", laterInput);

    expect(calls).toEqual([]);
    expect(nonComposedInput.propagationStopped).toBe(false);
    expect(laterInput.propagationStopped).toBe(false);
  });

  it("does nothing when xterm's helper textarea is unavailable", () => {
    const host = new FakeHost();
    const terminal = {
      textarea: undefined,
      options: { screenReaderMode: false },
      input() {
        throw new Error("input must not be called");
      },
    };

    const cleanup = setupXtermIme229Workaround({ terminal, host, enabled: true });

    expect(host.listeners.size).toBe(0);
    cleanup();
  });

  it("removes listeners on cleanup", () => {
    const { host, calls, textarea, cleanup } = setup();

    cleanup();
    host.fire("keydown", fakeEvent(textarea, { keyCode: 229, isComposing: false }));
    host.fire("input", textInputEvent(textarea, "o"));

    expect(calls).toEqual([]);
  });
});
