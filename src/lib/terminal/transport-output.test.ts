import { describe, expect, it } from "vitest";

import { readTransportOutput } from "./transport-output.ts";

describe("readTransportOutput", () => {
  it("keeps the SSH flow-control sequence beside its bytes", () => {
    const output = readTransportOutput({ data: [3, 4, 5], sequence: 17 });

    expect([...output.bytes]).toEqual([3, 4, 5]);
    expect(output.sequence).toBe(17);
  });

  it("keeps legacy unsequenced transport payloads compatible", () => {
    const output = readTransportOutput([6, 7, 8]);

    expect([...output.bytes]).toEqual([6, 7, 8]);
    expect(output.sequence).toBeNull();
  });
});
