/**
 * OSC sequence handlers for the rssh CLI <-> app integration.
 *
 * Currently handles `OSC 7337 ; <kind>:<name> ST`, where kind is one of:
 *   - `open`  open an SSH profile by name
 *   - `fwd`   start a port forward by name
 */

import { invoke } from "@tauri-apps/api/core";
import * as app from "../stores/app.svelte.ts";

const OSC_RSSH_ID = 7337;

export interface OscReporter {
  /** Called when an OSC referenced item cannot be resolved. */
  error(message: string): void;
}

type OscHandler = (name: string, ctx: OscReporter) => Promise<void> | void;

const HANDLERS: Record<string, OscHandler> = {
  open: openProfile,
  fwd: openForward,
};

async function openProfile(name: string, ctx: OscReporter) {
  const profiles = await invoke<any[]>("list_profiles");
  const p = profiles.find(x => x.name.toLowerCase() === name.toLowerCase());
  if (!p) { ctx.error(`Profile '${name}' not found`); return; }
  // Profile.credential_id 是必填——拿不到 cred = DB 不一致或 keychain 锁定，
  // 都属于"无法连通"的硬错。早期 try/catch{} 静默吞错会让用户看到一个
  // 注定连不上的死 tab，浪费一次往返才报错。这里 fail-fast。
  let cred: any;
  try {
    cred = await invoke<any>("get_credential", { id: p.credential_id });
  } catch (e: any) {
    ctx.error(`Profile '${name}': cannot load credential (${e?.message ?? String(e)})`);
    return;
  }
  const tid = `ssh:${crypto.randomUUID()}`;
  app.addTab({
    id: tid, type: "ssh", label: p.name,
    meta: {
      profileId: p.id, host: p.host, port: String(p.port),
      username: cred.username,
      authType: cred.type ?? "password",
      secret: cred.secret ?? "",
    },
  });
}

async function openForward(name: string, ctx: OscReporter) {
  const forwards = await invoke<any[]>("list_forwards");
  const f = forwards.find(x => x.name.toLowerCase() === name.toLowerCase());
  if (!f) { ctx.error(`Forward '${name}' not found`); return; }
  let profileName = "?";
  try { const p = await invoke<any>("get_profile", { id: f.profile_id }); profileName = p.name; } catch {}
  const tid = `fwd:${f.id}:${Date.now()}`;
  app.addTab({
    id: tid, type: "forward", label: f.name,
    meta: {
      forwardId: f.id, name: f.name,
      forwardType: f.type, localPort: String(f.local_port),
      remoteHost: f.remote_host, remotePort: String(f.remote_port),
      profileName,
    },
  });
}

/** xterm.js Terminal.parser shape we depend on. */
export interface OscParser {
  registerOscHandler(id: number, handler: (data: string) => boolean): void;
}

/** Hook the rssh OSC sequences into a terminal's OSC parser. */
export function registerRsshOscHandlers(parser: OscParser, reporter: OscReporter): void {
  parser.registerOscHandler(OSC_RSSH_ID, (data: string) => {
    const sep = data.indexOf(":");
    if (sep < 0) return false;
    const kind = data.slice(0, sep);
    const name = data.slice(sep + 1);
    const handler = HANDLERS[kind];
    if (!handler) return false;
    void handler(name, reporter);
    return true;
  });
}
