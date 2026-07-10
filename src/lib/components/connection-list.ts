import type { ConnectionKind } from "../stores/app.svelte.ts";

export type { ConnectionKind };

export interface ConnectionListItem {
  kind: ConnectionKind;
  id: string;
  name: string;
  detail: string;
  groupId: string | null;
}

export interface ConnectionGroup {
  id: string;
  name: string;
  color: string | null;
  sort_order: number;
}

export interface ConnectionSection {
  key: string;
  name: string;
  color: string | null;
  items: ConnectionListItem[];
}

interface GroupedSource {
  id: string;
  name: string;
  group_id: string | null;
}

interface ProfileSource extends GroupedSource {
  host: string;
  port: number;
}

interface ForwardSource extends GroupedSource {
  type: string;
  local_port: number;
  remote_host: string;
  remote_port: number;
  profile_id: string;
}

interface SerialSource extends GroupedSource {
  port: string;
  baud_rate: number;
  data_bits: number;
  parity: string;
  stop_bits: number;
}

interface TelnetSource extends GroupedSource {
  host: string;
  port: number;
}

export interface ConnectionSources {
  profiles: ProfileSource[];
  forwards: ForwardSource[];
  serialProfiles: SerialSource[];
  telnetProfiles: TelnetSource[];
}

export function buildConnectionItems(sources: ConnectionSources): ConnectionListItem[] {
  const profileNames = new Map(sources.profiles.map((profile) => [profile.id, profile.name]));
  return [
    ...sources.profiles.map((profile): ConnectionListItem => ({
      kind: "ssh",
      id: profile.id,
      name: profile.name,
      detail: `${profile.host}:${profile.port}`,
      groupId: profile.group_id,
    })),
    ...sources.forwards.map((forward): ConnectionListItem => ({
      kind: "forward",
      id: forward.id,
      name: forward.name,
      detail: `${forwardDetail(forward)} · via ${profileNames.get(forward.profile_id) ?? "?"}`,
      groupId: forward.group_id,
    })),
    ...sources.serialProfiles.map((serial): ConnectionListItem => ({
      kind: "serial",
      id: serial.id,
      name: serial.name,
      detail: `${serial.port} · ${serial.baud_rate} ${serial.data_bits}${(serial.parity[0] ?? "n").toUpperCase()}${serial.stop_bits}`,
      groupId: serial.group_id,
    })),
    ...sources.telnetProfiles.map((telnet): ConnectionListItem => ({
      kind: "telnet",
      id: telnet.id,
      name: telnet.name,
      detail: `${telnet.host}:${telnet.port}`,
      groupId: telnet.group_id,
    })),
  ];
}

function forwardDetail(forward: ForwardSource): string {
  if (forward.type === "dynamic") return `-D ${forward.local_port}`;
  if (forward.type === "remote") {
    return `-R ${forward.remote_port} → ${forward.remote_host}:${forward.local_port}`;
  }
  return `-L ${forward.local_port} → ${forward.remote_host}:${forward.remote_port}`;
}

export function groupConnectionItems(
  groups: ConnectionGroup[],
  items: ConnectionListItem[],
  ungroupedName: string,
): ConnectionSection[] {
  const knownGroups = new Set(groups.map((group) => group.id));
  const sections = [...groups]
    .sort((left, right) => left.sort_order - right.sort_order)
    .map((group) => ({
      key: group.id,
      name: group.name,
      color: group.color,
      items: items.filter((item) => item.groupId === group.id),
    }))
    .filter((section) => section.items.length > 0);

  const ungrouped = items.filter((item) => !item.groupId || !knownGroups.has(item.groupId));
  if (ungrouped.length > 0) {
    sections.push({ key: "__ungrouped__", name: ungroupedName, color: null, items: ungrouped });
  }

  return sections;
}
