import { describe, it, expect, vi, beforeEach } from "vitest";

import { registerClipboardOscHandler, type OscParser } from "./clipboard.ts";

function base64Utf8(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}

function setup() {
  let captured: ((data: string) => boolean) | null = null;
  const registerOscHandler = vi.fn<OscParser["registerOscHandler"]>((id, fn) => {
    expect(id).toBe(52);
    captured = fn;
  });
  const parser = { registerOscHandler };
  const clipboard = { writeText: vi.fn(async (_text: string) => {}) };

  registerClipboardOscHandler(parser, clipboard);

  if (!captured) throw new Error("OSC 52 handler not registered");
  const dispatch: (data: string) => boolean = captured;
  return { parser, clipboard, dispatch };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("registerClipboardOscHandler", () => {
  it("registers on OSC 52", () => {
    const { parser } = setup();
    expect(parser.registerOscHandler).toHaveBeenCalledTimes(1);
  });

  it("writes OSC 52 clipboard payloads to the system clipboard", () => {
    const { clipboard, dispatch } = setup();

    expect(dispatch(`c;${base64Utf8("hello from zellij")}`)).toBe(true);

    expect(clipboard.writeText).toHaveBeenCalledWith("hello from zellij");
  });

  it("decodes UTF-8 payloads", () => {
    const { clipboard, dispatch } = setup();

    expect(dispatch(`c;${base64Utf8("hello 中 👋")}`)).toBe(true);

    expect(clipboard.writeText).toHaveBeenCalledWith("hello 中 👋");
  });

  it("ignores clipboard read requests", () => {
    const { clipboard, dispatch } = setup();

    expect(dispatch("c;?")).toBe(true);

    expect(clipboard.writeText).not.toHaveBeenCalled();
  });

  it("does not handle non-clipboard selections", () => {
    const { clipboard, dispatch } = setup();

    expect(dispatch(`p;${base64Utf8("primary only")}`)).toBe(false);

    expect(clipboard.writeText).not.toHaveBeenCalled();
  });
});
