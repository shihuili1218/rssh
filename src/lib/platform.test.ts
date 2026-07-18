import { describe, expect, it } from "vitest";

import { detectPlatform } from "./platform.ts";

describe("detectPlatform", () => {
  it.each([
    ["iPhone", "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)", 5],
    ["iPad", "Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X)", 5],
    ["iPod", "Mozilla/5.0 (iPod touch; CPU iPhone OS 15_7 like Mac OS X)", 5],
    [
      "iPadOS desktop UA",
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1",
      5,
    ],
  ])("recognizes %s as iOS mobile", (_name, userAgent, maxTouchPoints) => {
    expect(detectPlatform({ userAgent, maxTouchPoints })).toEqual({
      isIOS: true,
      isMobile: true,
    });
  });

  it("recognizes Android as mobile but not iOS", () => {
    expect(detectPlatform({ userAgent: "Mozilla/5.0 (Linux; Android 16)", maxTouchPoints: 5 }))
      .toEqual({ isIOS: false, isMobile: true });
  });

  it("does not misclassify a touch-capable Mac or ordinary desktop", () => {
    expect(detectPlatform({ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15)", maxTouchPoints: 1 }))
      .toEqual({ isIOS: false, isMobile: false });
    expect(detectPlatform({ userAgent: "Mozilla/5.0 (X11; Linux x86_64)", maxTouchPoints: 0 }))
      .toEqual({ isIOS: false, isMobile: false });
  });

  it("defaults safely outside a browser", () => {
    expect(detectPlatform(undefined)).toEqual({ isIOS: false, isMobile: false });
  });
});
