import { afterEach, describe, expect, it, vi } from "vitest";

import {
  buildHomeSections,
  HOME_VIEW_STORAGE_KEY,
  loadHomeViewPreferences,
  parseHomeViewPreferences,
  saveHomeViewPreferences,
  updateHomeViewPreferences,
  type HomeLayoutItem,
} from "./home-layout.ts";

afterEach(() => {
  vi.unstubAllGlobals();
});

const kindLabel = (kind: HomeLayoutItem["kind"]) => `label:${kind}`;

function item(
  id: string,
  name: string,
  kind: HomeLayoutItem["kind"],
  groupId: string | null,
): HomeLayoutItem {
  return { id, name, kind, groupId };
}

const groups = [
  { id: "empty", name: "Empty", color: null, sortOrder: 0 },
  { id: "prod", name: "Production", color: "#f00", sortOrder: 20 },
  { id: "lab", name: "Lab", color: "#0f0", sortOrder: 10 },
];

describe("buildHomeSections", () => {
  it("keeps configured group order and sorts only inside each group", () => {
    const sections = buildHomeSections({
      grouping: "group",
      sorting: "name",
      items: [
        item("ssh:z", "Zulu", "ssh", "prod"),
        item("serial:c", "Console", "serial", "lab"),
        item("forward:a", "alpha", "forward", "prod"),
        item("telnet:n", "Node 10", "telnet", null),
        item("ssh:m", "Missing", "ssh", "deleted"),
        item("telnet:n2", "Node 2", "telnet", null),
      ],
      groups,
      recentItemIds: [],
      ungroupedLabel: "Ungrouped",
      kindLabel,
      locale: "en",
    });

    expect(sections.map((section) => section.key)).toEqual([
      "group:lab",
      "group:prod",
      "ungrouped",
    ]);
    expect(sections[1].items.map((entry) => entry.name)).toEqual(["alpha", "Zulu"]);
    expect(sections[2].items.map((entry) => entry.name)).toEqual([
      "Missing",
      "Node 2",
      "Node 10",
    ]);
    expect(sections.map((section) => section.offset)).toEqual([0, 1, 3]);
  });

  it("uses a fixed type order and alphabetizes only inside each type", () => {
    const sections = buildHomeSections({
      grouping: "type",
      sorting: "name",
      items: [
        item("k8s:z", "Zulu pod", "kubectl_exec", null),
        item("forward:z", "Zulu tunnel", "forward", "prod"),
        item("ssh:z", "Zulu host", "ssh", "prod"),
        item("docker:a", "Alpha container", "docker_exec", null),
        item("ssh:a", "Alpha host", "ssh", "lab"),
        item("telnet:s", "Switch", "telnet", null),
        item("serial:c", "Console", "serial", null),
      ],
      groups,
      recentItemIds: [],
      ungroupedLabel: "Ungrouped",
      kindLabel,
      locale: "en",
    });

    expect(sections.map((section) => section.key)).toEqual([
      "type:ssh",
      "type:forward",
      "type:serial",
      "type:telnet",
      "type:docker_exec",
      "type:kubectl_exec",
    ]);
    expect(sections[0].items.map((entry) => entry.name)).toEqual(["Alpha host", "Zulu host"]);
    expect(sections.map((section) => section.label)).toEqual([
      "label:ssh",
      "label:forward",
      "label:serial",
      "label:telnet",
      "label:docker_exec",
      "label:kubectl_exec",
    ]);
  });

  it("applies recent order inside groups without reordering the groups", () => {
    const sections = buildHomeSections({
      grouping: "group",
      sorting: "recent",
      items: [
        item("lab:a", "Alpha lab", "ssh", "lab"),
        item("lab:z", "Zulu lab", "serial", "lab"),
        item("prod:a", "Alpha prod", "ssh", "prod"),
        item("prod:m", "Mike prod", "forward", "prod"),
        item("prod:z", "Zulu prod", "telnet", "prod"),
      ],
      groups,
      recentItemIds: ["prod:z", "lab:z", "prod:m"],
      ungroupedLabel: "Ungrouped",
      kindLabel,
      locale: "en",
    });

    expect(sections.map((section) => section.key)).toEqual(["group:lab", "group:prod"]);
    expect(sections[0].items.map((entry) => entry.id)).toEqual(["lab:z", "lab:a"]);
    expect(sections[1].items.map((entry) => entry.id)).toEqual([
      "prod:z",
      "prod:m",
      "prod:a",
    ]);
  });

  it("applies recent order inside types without reordering the types", () => {
    const sections = buildHomeSections({
      grouping: "type",
      sorting: "recent",
      items: [
        item("ssh:a", "Alpha host", "ssh", null),
        item("ssh:z", "Zulu host", "ssh", null),
        item("forward:a", "Alpha tunnel", "forward", null),
        item("forward:z", "Zulu tunnel", "forward", null),
      ],
      groups: [],
      recentItemIds: ["forward:z", "ssh:z"],
      ungroupedLabel: "Ungrouped",
      kindLabel,
      locale: "en",
    });

    expect(sections.map((section) => section.key)).toEqual(["type:ssh", "type:forward"]);
    expect(sections[0].items.map((entry) => entry.id)).toEqual(["ssh:z", "ssh:a"]);
    expect(sections[1].items.map((entry) => entry.id)).toEqual(["forward:z", "forward:a"]);
  });
});

describe("Home view preferences", () => {
  it("falls back safely when persisted JSON is absent or corrupt", () => {
    expect(parseHomeViewPreferences(null)).toEqual({
      grouping: "group",
      sorting: "name",
    });
    expect(parseHomeViewPreferences("not-json")).toEqual({
      grouping: "group",
      sorting: "name",
    });
  });

  it("validates persisted display and sorting modes", () => {
    expect(parseHomeViewPreferences(JSON.stringify({
      grouping: "type",
      sorting: "recent",
    }))).toEqual({
      grouping: "type",
      sorting: "recent",
    });

    expect(parseHomeViewPreferences(JSON.stringify({
      grouping: "invalid",
      sorting: "invalid",
    }))).toEqual({
      grouping: "group",
      sorting: "name",
    });
  });

  it("persists the per-device display preferences", () => {
    const store = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, value),
    });

    saveHomeViewPreferences({ grouping: "type", sorting: "recent" });

    expect(store.get(HOME_VIEW_STORAGE_KEY)).toBe(JSON.stringify({
      grouping: "type",
      sorting: "recent",
    }));
    expect(loadHomeViewPreferences()).toEqual({ grouping: "type", sorting: "recent" });
  });

  it("merges one changed field with the latest preferences from another window", () => {
    const store = new Map<string, string>([[
      HOME_VIEW_STORAGE_KEY,
      JSON.stringify({ grouping: "type", sorting: "name" }),
    ]]);
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, value),
    });

    const next = updateHomeViewPreferences(
      { grouping: "group", sorting: "name" },
      { sorting: "recent" },
    );

    expect(next).toEqual({ grouping: "type", sorting: "recent" });
    expect(loadHomeViewPreferences()).toEqual(next);
  });
});
