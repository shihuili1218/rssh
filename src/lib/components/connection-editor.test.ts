import { describe, expect, it } from "vitest";
import { availableConnectionKinds, connectionCopyName } from "./connection-editor";

describe("connection editor copy behavior", () => {
  it("gives every copied connection a new default name", () => {
    expect(connectionCopyName("production")).toBe("production_copy");
  });

  it("does not offer serial connections on mobile", () => {
    expect(availableConnectionKinds(true)).toEqual(["ssh", "forward", "telnet"]);
    expect(availableConnectionKinds(false)).toEqual(["ssh", "forward", "serial", "telnet"]);
  });
});
