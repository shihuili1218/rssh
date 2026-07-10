import { describe, expect, it } from "vitest";
import {
  buildConnectionItems,
  groupConnectionItems,
  type ConnectionListItem,
} from "./connection-list";

describe("connection list grouping", () => {
  it("keeps configured group order while mixing connection types", () => {
    const items: ConnectionListItem[] = [
      { kind: "ssh", id: "ssh-1", name: "API", detail: "api:22", groupId: "prod" },
      { kind: "forward", id: "fwd-1", name: "DB tunnel", detail: ":5432", groupId: "prod" },
      { kind: "serial", id: "serial-1", name: "Console", detail: "/dev/ttyUSB0", groupId: "lab" },
      { kind: "telnet", id: "telnet-1", name: "Switch", detail: "switch:23", groupId: "prod" },
    ];

    const sections = groupConnectionItems(
      [
        { id: "prod", name: "Production", color: "#f00", sort_order: 20 },
        { id: "lab", name: "Lab", color: "#0f0", sort_order: 10 },
      ],
      items,
      "Ungrouped",
    );

    expect(sections.map((section) => section.key)).toEqual(["lab", "prod"]);
    expect(sections[1].items.map((item) => item.kind)).toEqual(["ssh", "forward", "telnet"]);
  });

  it("normalizes all saved connection types in a stable order", () => {
    const items = buildConnectionItems({
      profiles: [{ id: "ssh-1", name: "API", host: "api", port: 22, group_id: "prod" }],
      forwards: [{
        id: "fwd-1", name: "DB tunnel", type: "local", local_port: 5432,
        remote_host: "db", remote_port: 5432, profile_id: "ssh-1", group_id: "prod",
      }],
      serialProfiles: [{
        id: "serial-1", name: "Console", port: "/dev/ttyUSB0", baud_rate: 115200,
        data_bits: 8, parity: "none", stop_bits: 1, group_id: "lab",
      }],
      telnetProfiles: [{ id: "telnet-1", name: "Switch", host: "switch", port: 23, group_id: null }],
    });

    expect(items.map((item) => item.kind)).toEqual(["ssh", "forward", "serial", "telnet"]);
    expect(items.map((item) => item.detail)).toEqual([
      "api:22",
      "-L 5432 → db:5432 · via API",
      "/dev/ttyUSB0 · 115200 8N1",
      "switch:23",
    ]);
  });

  it("formats local, remote, and dynamic forwards by their real direction", () => {
    const items = buildConnectionItems({
      profiles: [{ id: "ssh-1", name: "Gateway", host: "gateway", port: 22, group_id: null }],
      forwards: [
        { id: "local", name: "Local", type: "local", local_port: 9000, remote_host: "target", remote_port: 7000, profile_id: "ssh-1", group_id: null },
        { id: "remote", name: "Remote", type: "remote", local_port: 9000, remote_host: "target", remote_port: 7000, profile_id: "ssh-1", group_id: null },
        { id: "dynamic", name: "SOCKS", type: "dynamic", local_port: 1080, remote_host: "unused", remote_port: 0, profile_id: "ssh-1", group_id: null },
      ],
      serialProfiles: [],
      telnetProfiles: [],
    });

    expect(items.filter((item) => item.kind === "forward").map((item) => item.detail)).toEqual([
      "-L 9000 → target:7000 · via Gateway",
      "-R 7000 → target:9000 · via Gateway",
      "-D 1080 · via Gateway",
    ]);
  });

  it("hides empty groups and keeps missing groups in one trailing section", () => {
    const items: ConnectionListItem[] = [
      { kind: "ssh", id: "known", name: "Known", detail: "known:22", groupId: "used" },
      { kind: "serial", id: "missing", name: "Missing", detail: "/dev/tty0", groupId: "deleted" },
      { kind: "telnet", id: "none", name: "None", detail: "none:23", groupId: null },
    ];

    const sections = groupConnectionItems(
      [
        { id: "empty", name: "Empty", color: null, sort_order: 0 },
        { id: "used", name: "Used", color: null, sort_order: 1 },
      ],
      items,
      "Ungrouped",
    );

    expect(sections.map((section) => section.key)).toEqual(["used", "__ungrouped__"]);
    expect(sections[1].items.map((item) => item.id)).toEqual(["missing", "none"]);
  });
});
