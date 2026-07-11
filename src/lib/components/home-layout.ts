export type HomeGrouping = "group" | "type";
export type HomeSorting = "name" | "recent";
export type HomeItemKind =
  | "ssh"
  | "forward"
  | "serial"
  | "telnet"
  | "docker_exec"
  | "kubectl_exec";

export interface HomeLayoutItem {
  id: string;
  name: string;
  kind: HomeItemKind;
  groupId: string | null;
}

export interface HomeLayoutGroup {
  id: string;
  name: string;
  color: string | null;
  sortOrder: number;
}

export interface HomeSection<T extends HomeLayoutItem> {
  key: string;
  label: string;
  color: string | null;
  items: T[];
  offset: number;
}

export interface HomeViewPreferences {
  grouping: HomeGrouping;
  sorting: HomeSorting;
}

interface BuildHomeSectionsOptions<T extends HomeLayoutItem> {
  grouping: HomeGrouping;
  sorting: HomeSorting;
  items: readonly T[];
  groups: readonly HomeLayoutGroup[];
  recentItemIds: readonly string[];
  ungroupedLabel: string;
  kindLabel: (kind: HomeItemKind) => string;
  locale: string;
}

export const HOME_VIEW_STORAGE_KEY = "home.view.v1";

const HOME_KIND_ORDER: readonly HomeItemKind[] = [
  "ssh",
  "forward",
  "serial",
  "telnet",
  "docker_exec",
  "kubectl_exec",
];

function defaultHomeViewPreferences(): HomeViewPreferences {
  return { grouping: "group", sorting: "name" };
}

export function parseHomeViewPreferences(raw: string | null): HomeViewPreferences {
  if (raw === null) return defaultHomeViewPreferences();

  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return defaultHomeViewPreferences();
    }

    const value = parsed as Record<string, unknown>;
    const grouping: HomeGrouping = value.grouping === "type" ? "type" : "group";
    const sorting: HomeSorting = value.sorting === "recent" ? "recent" : "name";
    return { grouping, sorting };
  } catch {
    return defaultHomeViewPreferences();
  }
}

export function loadHomeViewPreferences(): HomeViewPreferences {
  if (typeof localStorage === "undefined") return defaultHomeViewPreferences();
  try {
    return parseHomeViewPreferences(localStorage.getItem(HOME_VIEW_STORAGE_KEY));
  } catch {
    return defaultHomeViewPreferences();
  }
}

export function saveHomeViewPreferences(preferences: HomeViewPreferences): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(HOME_VIEW_STORAGE_KEY, JSON.stringify(preferences));
  } catch {
    // View preferences are best-effort. The in-memory state remains usable.
  }
}

export function updateHomeViewPreferences(
  current: HomeViewPreferences,
  update: Partial<HomeViewPreferences>,
): HomeViewPreferences {
  let latest = current;
  if (typeof localStorage !== "undefined") {
    try {
      const raw = localStorage.getItem(HOME_VIEW_STORAGE_KEY);
      if (raw !== null) latest = parseHomeViewPreferences(raw);
    } catch {
      // Keep the in-memory value when storage is unavailable.
    }
  }

  const next: HomeViewPreferences = {
    grouping: update.grouping ?? latest.grouping,
    sorting: update.sorting ?? latest.sorting,
  };
  saveHomeViewPreferences(next);
  return next;
}

export function buildHomeSections<T extends HomeLayoutItem>(
  options: BuildHomeSectionsOptions<T>,
): HomeSection<T>[] {
  const compareItems = itemComparator(
    options.sorting,
    options.recentItemIds,
    options.locale,
  );
  const sections = options.grouping === "group"
    ? groupSections(options, compareItems)
    : typeSections(options, compareItems);

  let offset = 0;
  for (const section of sections) {
    section.offset = offset;
    offset += section.items.length;
  }
  return sections;
}

function groupSections<T extends HomeLayoutItem>(
  options: BuildHomeSectionsOptions<T>,
  compareItems: (left: T, right: T) => number,
): HomeSection<T>[] {
  const itemsByGroup = new Map<string, T[]>();
  const ungrouped: T[] = [];
  const knownGroupIds = new Set(options.groups.map((group) => group.id));

  for (const item of options.items) {
    if (!item.groupId || !knownGroupIds.has(item.groupId)) {
      ungrouped.push(item);
      continue;
    }
    const groupItems = itemsByGroup.get(item.groupId) ?? [];
    groupItems.push(item);
    itemsByGroup.set(item.groupId, groupItems);
  }

  const sections: HomeSection<T>[] = [];
  const orderedGroups = [...options.groups].sort((left, right) =>
    left.sortOrder - right.sortOrder || left.name.localeCompare(right.name)
  );
  for (const group of orderedGroups) {
    const items = itemsByGroup.get(group.id);
    if (!items?.length) continue;
    sections.push({
      key: `group:${group.id}`,
      label: group.name,
      color: group.color,
      items: [...items].sort(compareItems),
      offset: 0,
    });
  }

  if (ungrouped.length > 0) {
    sections.push({
      key: "ungrouped",
      label: options.ungroupedLabel,
      color: null,
      items: [...ungrouped].sort(compareItems),
      offset: 0,
    });
  }
  return sections;
}

function typeSections<T extends HomeLayoutItem>(
  options: BuildHomeSectionsOptions<T>,
  compareItems: (left: T, right: T) => number,
): HomeSection<T>[] {
  const itemsByKind = new Map<HomeItemKind, T[]>();
  for (const item of options.items) {
    const kindItems = itemsByKind.get(item.kind) ?? [];
    kindItems.push(item);
    itemsByKind.set(item.kind, kindItems);
  }

  const sections: HomeSection<T>[] = [];
  for (const kind of HOME_KIND_ORDER) {
    const items = itemsByKind.get(kind);
    if (!items?.length) continue;
    sections.push({
      key: `type:${kind}`,
      label: options.kindLabel(kind),
      color: null,
      items: [...items].sort(compareItems),
      offset: 0,
    });
  }
  return sections;
}

function itemComparator<T extends HomeLayoutItem>(
  sorting: HomeSorting,
  recentItemIds: readonly string[],
  locale: string,
): (left: T, right: T) => number {
  const collator = new Intl.Collator(locale, { sensitivity: "base", numeric: true });
  const recentRank = new Map<string, number>();
  for (const [index, id] of recentItemIds.entries()) {
    if (!recentRank.has(id)) recentRank.set(id, index);
  }

  const compareName = (left: T, right: T) =>
    collator.compare(left.name, right.name) || collator.compare(left.id, right.id);

  if (sorting === "name") return compareName;
  return (left, right) => {
    const leftRank = recentRank.get(left.id);
    const rightRank = recentRank.get(right.id);
    if (leftRank !== undefined || rightRank !== undefined) {
      if (leftRank === undefined) return 1;
      if (rightRank === undefined) return -1;
      if (leftRank !== rightRank) return leftRank - rightRank;
    }
    return compareName(left, right);
  };
}
