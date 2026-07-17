import type { ConnectionKind, DynamicPlatform, TabType } from "../stores/app.svelte.ts";

export const APP_ICON_NAMES = [
  "add",
  "ai",
  "check",
  "cloud-download",
  "cloud-upload",
  "docker",
  "edit",
  "file",
  "folder",
  "forward",
  "home",
  "kubernetes",
  "link",
  "pin",
  "save",
  "serial",
  "settings",
  "snippet",
  "ssh",
  "star",
  "telnet",
  "terminal",
  "transfer",
  "warning",
] as const;

export type AppIconName = (typeof APP_ICON_NAMES)[number];

export function connectionIconName(kind: ConnectionKind): AppIconName {
  switch (kind) {
    case "ssh": return "ssh";
    case "forward": return "forward";
    case "serial": return "serial";
    case "telnet": return "telnet";
  }
}

export function dynamicPlatformIconName(platform: DynamicPlatform): AppIconName {
  return platform === "docker" ? "docker" : "kubernetes";
}

export function tabIconName(type: TabType): AppIconName {
  switch (type) {
    case "home": return "home";
    case "ssh": return "ssh";
    case "local": return "terminal";
    case "serial": return "serial";
    case "telnet": return "telnet";
    case "docker_exec": return "docker";
    case "kubectl_exec": return "kubernetes";
    case "forward": return "forward";
    case "edit": return "edit";
  }
}
